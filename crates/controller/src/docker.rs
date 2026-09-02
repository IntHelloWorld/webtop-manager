use std::collections::{BTreeSet, HashMap};
use std::io::Cursor;
use std::os::unix::fs::MetadataExt;

use anyhow::{Context, Result};
use bollard::models::{
    ContainerCreateBody, DeviceRequest, HostConfig, PortBinding, RestartPolicy,
    RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    ListImagesOptionsBuilder, LogsOptionsBuilder, PruneImagesOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveImageOptionsBuilder, StopContainerOptionsBuilder,
    UploadToContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::{StreamExt, TryStreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, Duration};
use uuid::Uuid;
use webtop_contracts::{
    EnvironmentSpec, FileTransferMode, FrpcServiceState, FrpcServiceStatus, FrpcTestCode,
    FrpcTestResult, GpuMode, ImageCachePruneResult, ImagePullPhase, ImagePullProgress,
    OfficialImage, SeccompMode, ServerSettings, OWNER_LABEL, RESOURCE_ID_LABEL,
    RESOURCE_KIND_LABEL, WEBTOP_HTTPS_PORT,
};

struct OfficialImageDefinition {
    tag: &'static str,
    distribution: &'static str,
    desktop: &'static str,
    wayland_support: bool,
    wayland_only: bool,
}

// Stable tags documented by LinuxServer.io. The explicitly unstable `dev` tag
// is intentionally excluded from the desktop creation flow.
const OFFICIAL_WEBTOP_IMAGES: &[OfficialImageDefinition] = &[
    OfficialImageDefinition {
        tag: "latest",
        distribution: "Alpine",
        desktop: "XFCE",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "alpine-i3",
        distribution: "Alpine",
        desktop: "i3",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "alpine-kde",
        distribution: "Alpine",
        desktop: "KDE",
        wayland_support: true,
        wayland_only: true,
    },
    OfficialImageDefinition {
        tag: "alpine-mate",
        distribution: "Alpine",
        desktop: "MATE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "arch-i3",
        distribution: "Arch",
        desktop: "i3",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "arch-kde",
        distribution: "Arch",
        desktop: "KDE",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "arch-mate",
        distribution: "Arch",
        desktop: "MATE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "arch-xfce",
        distribution: "Arch",
        desktop: "XFCE",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "debian-i3",
        distribution: "Debian",
        desktop: "i3",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "debian-kde",
        distribution: "Debian",
        desktop: "KDE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "debian-mate",
        distribution: "Debian",
        desktop: "MATE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "debian-xfce",
        distribution: "Debian",
        desktop: "XFCE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "fedora-i3",
        distribution: "Fedora",
        desktop: "i3",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "fedora-kde",
        distribution: "Fedora",
        desktop: "KDE",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "fedora-mate",
        distribution: "Fedora",
        desktop: "MATE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "fedora-xfce",
        distribution: "Fedora",
        desktop: "XFCE",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "ubuntu-i3",
        distribution: "Ubuntu",
        desktop: "i3",
        wayland_support: true,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "ubuntu-kde",
        distribution: "Ubuntu",
        desktop: "KDE",
        wayland_support: true,
        wayland_only: true,
    },
    OfficialImageDefinition {
        tag: "ubuntu-mate",
        distribution: "Ubuntu",
        desktop: "MATE",
        wayland_support: false,
        wayland_only: false,
    },
    OfficialImageDefinition {
        tag: "ubuntu-xfce",
        distribution: "Ubuntu",
        desktop: "XFCE",
        wayland_support: true,
        wayland_only: false,
    },
];

fn official_reference(tag: &str) -> String {
    format!("lscr.io/linuxserver/webtop:{tag}")
}

pub fn is_official_image(reference: &str) -> bool {
    OFFICIAL_WEBTOP_IMAGES
        .iter()
        .any(|image| official_reference(image.tag) == reference)
}

pub async fn list_official_images(docker: &Docker) -> Result<Vec<OfficialImage>> {
    let local_images = docker
        .list_images(Some(ListImagesOptionsBuilder::default().all(true).build()))
        .await
        .context("list local Docker images")?;

    Ok(OFFICIAL_WEBTOP_IMAGES
        .iter()
        .map(|definition| {
            let reference = official_reference(definition.tag);
            let local = local_images
                .iter()
                .find(|image| image.repo_tags.iter().any(|tag| tag == &reference));
            OfficialImage {
                reference,
                tag: definition.tag.into(),
                distribution: definition.distribution.into(),
                desktop: definition.desktop.into(),
                wayland_support: definition.wayland_support,
                wayland_only: definition.wayland_only,
                installed: local.is_some(),
                image_id: local.map(|image| image.id.clone()),
                size_bytes: local.map(|image| image.size),
            }
        })
        .collect())
}

pub enum ImagePullOutcome {
    Complete,
    Cancelled,
}

pub async fn pull_official_image(
    docker: &Docker,
    pull_id: Uuid,
    reference: &str,
    mut cancel: watch::Receiver<bool>,
    events: mpsc::Sender<ImagePullProgress>,
) -> Result<ImagePullOutcome> {
    anyhow::ensure!(
        is_official_image(reference),
        "unsupported official Webtop image"
    );
    if !send_pull_event(
        &events,
        ImagePullProgress {
            pull_id,
            reference: reference.into(),
            phase: ImagePullPhase::Starting,
            layer_id: None,
            status: "Preparing image pull".into(),
            current_bytes: None,
            total_bytes: None,
            aggregate_current_bytes: None,
            aggregate_total_bytes: None,
        },
    )
    .await
    {
        return Ok(ImagePullOutcome::Cancelled);
    }

    if docker.inspect_image(reference).await.is_ok() {
        let _ = send_pull_event(
            &events,
            ImagePullProgress {
                pull_id,
                reference: reference.into(),
                phase: ImagePullPhase::Complete,
                layer_id: None,
                status: "Image already available locally".into(),
                current_bytes: None,
                total_bytes: None,
                aggregate_current_bytes: None,
                aggregate_total_bytes: None,
            },
        )
        .await;
        return Ok(ImagePullOutcome::Complete);
    }

    let mut pull = docker.create_image(
        Some(
            CreateImageOptionsBuilder::default()
                .from_image(reference)
                .platform("linux/amd64")
                .build(),
        ),
        None,
        None,
    );
    let mut layer_progress: HashMap<String, (i64, i64)> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() {
                    let _ = send_pull_event(
                        &events,
                        ImagePullProgress {
                            pull_id,
                            reference: reference.into(),
                            phase: ImagePullPhase::Cancelled,
                            layer_id: None,
                            status: "Image pull cancelled".into(),
                            current_bytes: None,
                            total_bytes: None,
                            aggregate_current_bytes: None,
                            aggregate_total_bytes: None,
                        },
                    ).await;
                    return Ok(ImagePullOutcome::Cancelled);
                }
            }
            result = pull.next() => {
                let Some(result) = result else { break };
                let info = result.context("pull Webtop image")?;
                let current_bytes = info.progress_detail.as_ref().and_then(|progress| progress.current);
                let total_bytes = info.progress_detail.as_ref().and_then(|progress| progress.total);
                if let (Some(layer_id), Some(current), Some(total)) = (&info.id, current_bytes, total_bytes) {
                    layer_progress.insert(layer_id.clone(), (current.max(0), total.max(0)));
                }
                let (aggregate_current, aggregate_total) = layer_progress
                    .values()
                    .fold((0_i64, 0_i64), |(current_sum, total_sum), (current, total)| {
                        (current_sum.saturating_add(*current), total_sum.saturating_add(*total))
                    });
                let event = ImagePullProgress {
                    pull_id,
                    reference: reference.into(),
                    phase: ImagePullPhase::Progress,
                    layer_id: info.id,
                    status: info.status.unwrap_or_else(|| "Downloading".into()),
                    current_bytes,
                    total_bytes,
                    aggregate_current_bytes: (aggregate_total > 0).then_some(aggregate_current),
                    aggregate_total_bytes: (aggregate_total > 0).then_some(aggregate_total),
                };
                if !send_pull_event(&events, event).await {
                    return Ok(ImagePullOutcome::Cancelled);
                }
            }
        }
    }

    docker
        .inspect_image(reference)
        .await
        .context("verify pulled Webtop image")?;
    let _ = send_pull_event(
        &events,
        ImagePullProgress {
            pull_id,
            reference: reference.into(),
            phase: ImagePullPhase::Complete,
            layer_id: None,
            status: "Image pull complete".into(),
            current_bytes: None,
            total_bytes: None,
            aggregate_current_bytes: None,
            aggregate_total_bytes: None,
        },
    )
    .await;
    Ok(ImagePullOutcome::Complete)
}

pub async fn prune_image_cache(docker: &Docker) -> Result<ImageCachePruneResult> {
    let mut filters = HashMap::new();
    filters.insert("dangling", vec!["true"]);
    let response = docker
        .prune_images(Some(
            PruneImagesOptionsBuilder::default()
                .filters(&filters)
                .build(),
        ))
        .await
        .context("prune unused Docker image cache")?;

    Ok(ImageCachePruneResult {
        deleted_items: response.images_deleted.unwrap_or_default().len(),
        space_reclaimed_bytes: response.space_reclaimed.unwrap_or_default().max(0),
    })
}

pub async fn official_image_in_use(docker: &Docker, reference: &str) -> Result<bool> {
    anyhow::ensure!(
        is_official_image(reference),
        "unsupported official Webtop image"
    );
    let image_id = docker
        .inspect_image(reference)
        .await
        .context("inspect official Webtop image")?
        .id
        .context("official Webtop image has no ID")?;
    let containers = docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default().all(true).build(),
        ))
        .await
        .context("list containers before image deletion")?;
    Ok(containers
        .iter()
        .any(|container| container.image_id.as_deref() == Some(image_id.as_str())))
}

pub async fn remove_official_image(docker: &Docker, reference: &str) -> Result<()> {
    anyhow::ensure!(
        is_official_image(reference),
        "unsupported official Webtop image"
    );
    docker
        .remove_image(
            reference,
            Some(
                RemoveImageOptionsBuilder::default()
                    .force(false)
                    .noprune(false)
                    .build(),
            ),
            None,
        )
        .await
        .context("remove official Webtop image")?;
    Ok(())
}

async fn send_pull_event(
    events: &mpsc::Sender<ImagePullProgress>,
    event: ImagePullProgress,
) -> bool {
    // Progress subscribers are allowed to disconnect without cancelling the
    // controller-owned pull. Cancellation is explicit through the watch
    // channel so the job can survive a closed desktop or HTTP stream.
    let _ = events.send(event).await;
    true
}

pub struct CreatedContainer {
    pub id: String,
    pub local_port: Option<u16>,
}

pub async fn create_environment_container(
    docker: &Docker,
    resource_id: &str,
    spec: &EnvironmentSpec,
    config_path: &str,
    secret_path: &str,
) -> Result<CreatedContainer> {
    ensure_image(docker, &spec.image).await?;
    create_environment_container_inner(docker, resource_id, spec, config_path, secret_path).await
}

pub async fn create_environment_container_from_local_image(
    docker: &Docker,
    resource_id: &str,
    spec: &EnvironmentSpec,
    config_path: &str,
    secret_path: &str,
) -> Result<CreatedContainer> {
    docker
        .inspect_image(&spec.image)
        .await
        .context("template image is missing locally")?;
    create_environment_container_inner(docker, resource_id, spec, config_path, secret_path).await
}

async fn create_environment_container_inner(
    docker: &Docker,
    resource_id: &str,
    spec: &EnvironmentSpec,
    config_path: &str,
    secret_path: &str,
) -> Result<CreatedContainer> {
    let container_name = format!(
        "webtop-manager-{}-{resource_id}",
        normalized_name(&spec.name)
    );
    let file_transfers = match spec.display.file_transfer_mode {
        Some(FileTransferMode::UploadDownload) => "upload,download",
        Some(FileTransferMode::Upload) => "upload",
        Some(FileTransferMode::Download) => "download",
        Some(FileTransferMode::None) => "none",
        None if spec.display.file_transfer => "upload,download",
        None => "none",
    };
    let mut env = vec![
        format!("PUID={}", spec.identity.uid),
        format!("PGID={}", spec.identity.gid),
        format!("TZ={}", spec.identity.timezone),
        format!("CUSTOM_USER=webtop-{resource_id}"),
        "FILE__PASSWORD=/run/webtop-manager/password".to_owned(),
        format!("LC_ALL={}", spec.identity.locale),
        format!("SELKIES_AUDIO_ENABLED={}", spec.display.audio),
        format!("SELKIES_CLIPBOARD_ENABLED={}", spec.display.clipboard),
        format!("SELKIES_FILE_TRANSFERS={file_transfers}"),
    ];
    if let Some(wayland) = spec.display.wayland {
        env.push(format!("PIXELFLUX_WAYLAND={wayland}"));
    }
    if let (Some(width), Some(height)) = (spec.display.width, spec.display.height) {
        env.push(format!("SELKIES_MANUAL_WIDTH={width}"));
        env.push(format!("SELKIES_MANUAL_HEIGHT={height}"));
    }
    env.extend(
        spec.extra_environment
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );

    let mut binds = vec![
        format!("{config_path}:/config:rw"),
        format!("{secret_path}:/run/webtop-manager/password:ro"),
    ];
    binds.extend(spec.mounts.iter().map(|mount| {
        format!(
            "{}:{}:{}",
            mount.host_path.display(),
            mount.container_path.display(),
            if mount.read_only { "ro" } else { "rw" }
        )
    }));
    if spec.security.docker_socket {
        binds.push("/var/run/docker.sock:/var/run/docker.sock:rw".into());
    }

    let webtop_https_port = webtop_https_port(spec);
    let port_key = format!("{webtop_https_port}/tcp");
    let port_bindings = HashMap::from([(
        port_key.clone(),
        Some(vec![PortBinding {
            host_ip: Some("127.0.0.1".into()),
            host_port: Some(String::new()),
        }]),
    )]);
    let mut devices = spec
        .security
        .devices
        .iter()
        .map(|path| bollard::models::DeviceMapping {
            path_on_host: Some(path.display().to_string()),
            path_in_container: Some(path.display().to_string()),
            cgroup_permissions: Some("rwm".into()),
        })
        .collect::<Vec<_>>();
    if matches!(spec.display.gpu, GpuMode::Dri) && devices.is_empty() {
        devices.push(bollard::models::DeviceMapping {
            path_on_host: Some("/dev/dri".into()),
            path_in_container: Some("/dev/dri".into()),
            cgroup_permissions: Some("rwm".into()),
        });
    }
    let manual_gpu_nodes = spec.extra_environment.contains_key("DRI_NODE")
        || spec.extra_environment.contains_key("DRINODE");
    if matches!(spec.display.gpu, GpuMode::Dri) && !manual_gpu_nodes {
        env.push("AUTO_GPU=true".into());
    }
    let device_requests = matches!(spec.display.gpu, GpuMode::Nvidia).then(|| {
        if !manual_gpu_nodes {
            env.push("AUTO_GPU=true".into());
        }
        vec![DeviceRequest {
            driver: Some("nvidia".into()),
            count: Some(-1),
            capabilities: Some(vec![vec!["gpu".into()]]),
            ..Default::default()
        }]
    });
    let docker_socket_gid = if spec.security.docker_socket {
        Some(
            std::fs::metadata("/var/run/docker.sock")
                .context("inspect Docker socket group")?
                .gid(),
        )
    } else {
        None
    };

    let security_opt = match spec.security.seccomp {
        SeccompMode::Default => Some(vec!["no-new-privileges=true".into()]),
        SeccompMode::Unconfined => Some(vec![
            "no-new-privileges=true".into(),
            "seccomp=unconfined".into(),
        ]),
    };
    let nano_cpus = spec
        .resources
        .cpu_limit
        .map(|value| (value * 1_000_000_000.0) as i64);
    let host_config = HostConfig {
        binds: Some(binds),
        port_bindings: Some(port_bindings),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            maximum_retry_count: None,
        }),
        shm_size: Some(spec.resources.shm_bytes),
        memory: spec.resources.memory_bytes,
        nano_cpus,
        privileged: Some(false),
        security_opt,
        devices: Some(devices),
        device_requests,
        group_add: docker_socket_gid.map(|gid| vec![gid.to_string()]),
        ..Default::default()
    };

    let labels = HashMap::from([
        (OWNER_LABEL.into(), "managed".into()),
        (RESOURCE_ID_LABEL.into(), resource_id.into()),
        (RESOURCE_KIND_LABEL.into(), "environment".into()),
    ]);
    let config = ContainerCreateBody {
        image: Some(spec.image.clone()),
        env: Some(env),
        labels: Some(labels),
        exposed_ports: Some(vec![port_key.clone()]),
        host_config: Some(host_config),
        ..Default::default()
    };

    let response = docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(&container_name)
                    .build(),
            ),
            config,
        )
        .await
        .context("create Webtop container")?;
    if let Err(error) = docker.start_container(&response.id, None).await {
        let _ = docker
            .remove_container(
                &response.id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        return Err(error).context("start Webtop container");
    }
    let local_port = environment_local_port(docker, &response.id, spec).await?;
    Ok(CreatedContainer {
        id: response.id,
        local_port,
    })
}

pub async fn environment_local_port(
    docker: &Docker,
    container_id: &str,
    spec: &EnvironmentSpec,
) -> Result<Option<u16>> {
    let port_key = format!("{}/tcp", webtop_https_port(spec));
    let inspect = docker.inspect_container(container_id, None).await?;
    Ok(inspect
        .network_settings
        .and_then(|settings| settings.ports)
        .and_then(|ports| host_port_from_bindings(&ports, &port_key)))
}

fn webtop_https_port(spec: &EnvironmentSpec) -> u16 {
    spec.extra_environment
        .get("CUSTOM_HTTPS_PORT")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(WEBTOP_HTTPS_PORT)
}

fn host_port_from_bindings(
    ports: &HashMap<String, Option<Vec<PortBinding>>>,
    port_key: &str,
) -> Option<u16> {
    ports
        .get(port_key)
        .and_then(Option::as_ref)
        .and_then(|bindings| bindings.first())
        .and_then(|binding| binding.host_port.as_deref())
        .and_then(|port| port.parse().ok())
}

async fn ensure_image(docker: &Docker, image: &str) -> Result<()> {
    if docker.inspect_image(image).await.is_ok() {
        return Ok(());
    }
    let mut pull = docker.create_image(
        Some(
            CreateImageOptionsBuilder::default()
                .from_image(image)
                .platform("linux/amd64")
                .build(),
        ),
        None,
        None,
    );
    while let Some(result) = pull.next().await {
        result.context("pull Webtop image")?;
    }
    docker
        .inspect_image(image)
        .await
        .context("verify pulled Webtop image")?;
    Ok(())
}

pub async fn start_frpc(
    docker: &Docker,
    container_name: &str,
    settings: &ServerSettings,
    token: &[u8],
    proxies: &[crate::frp::Proxy],
) -> Result<FrpcServiceStatus> {
    ensure_image(docker, &settings.frpc_image).await?;
    remove_frpc(docker, container_name).await?;
    create_frpc_container(
        docker,
        container_name,
        ("frpc", "shared"),
        settings,
        token,
        proxies,
        true,
    )
    .await?;
    docker
        .start_container(container_name, None)
        .await
        .context("start shared frpc container")?;
    sleep(Duration::from_millis(350)).await;
    frpc_status(docker, container_name).await
}

pub async fn stop_frpc(docker: &Docker, container_name: &str) -> Result<FrpcServiceStatus> {
    let Some(inspect) = inspect_managed_frpc(docker, container_name).await? else {
        return Ok(not_created_frpc_status());
    };
    if inspect.state.as_ref().and_then(|state| state.running) == Some(true) {
        docker
            .stop_container(
                container_name,
                Some(StopContainerOptionsBuilder::default().t(10).build()),
            )
            .await
            .context("stop shared frpc container")?;
    }
    frpc_status(docker, container_name).await
}

pub async fn remove_frpc(docker: &Docker, container_name: &str) -> Result<()> {
    if inspect_managed_frpc(docker, container_name)
        .await?
        .is_some()
    {
        docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .context("remove shared frpc container")?;
    }
    Ok(())
}

pub async fn frpc_status(docker: &Docker, container_name: &str) -> Result<FrpcServiceStatus> {
    let Some(inspect) = inspect_managed_frpc(docker, container_name).await? else {
        return Ok(not_created_frpc_status());
    };
    let state = inspect.state.unwrap_or_default();
    let running = state.running == Some(true);
    let logs = recent_logs(docker, container_name)
        .await
        .unwrap_or_default();
    let connected = running && logs.contains("login to server success");
    let service_state = if running {
        FrpcServiceState::Running
    } else if state.exit_code.unwrap_or_default() == 0 {
        FrpcServiceState::Stopped
    } else {
        FrpcServiceState::Error
    };
    Ok(FrpcServiceStatus {
        state: service_state,
        connected,
        image: inspect.config.and_then(|config| config.image),
        started_at: state.started_at,
        exit_code: state.exit_code,
    })
}

pub async fn frpc_conflicting_proxy_ids(
    docker: &Docker,
    container_name: &str,
    proxies: &[crate::frp::Proxy],
) -> Result<BTreeSet<String>> {
    if proxies.is_empty() {
        return Ok(BTreeSet::new());
    }
    for _ in 0..20 {
        let logs = recent_logs(docker, container_name).await?;
        let (conflicts, settled) = classify_proxy_registrations(&logs, proxies);
        if !conflicts.is_empty() || settled == proxies.len() {
            return Ok(conflicts);
        }
        sleep(Duration::from_millis(250)).await;
    }
    Ok(BTreeSet::new())
}

fn classify_proxy_registrations(
    logs: &str,
    proxies: &[crate::frp::Proxy],
) -> (BTreeSet<String>, usize) {
    let mut conflicts = BTreeSet::new();
    let mut settled = BTreeSet::new();
    for line in logs.lines() {
        let lower = line.to_ascii_lowercase();
        let is_conflict = lower.contains("port already used")
            || lower.contains("port is already used")
            || lower.contains("remote port is used")
            || lower.contains("proxy name") && lower.contains("already in use");
        let is_success = lower.contains("start proxy success")
            || lower.contains("proxy added")
            || lower.contains("start proxy") && lower.contains("success");
        if !is_conflict && !is_success {
            continue;
        }
        for proxy in proxies {
            let proxy_name = format!("webtop-{}", proxy.resource_id);
            if line.contains(&proxy_name) {
                settled.insert(proxy.resource_id.clone());
                if is_conflict {
                    conflicts.insert(proxy.resource_id.clone());
                }
            }
        }
    }
    (conflicts, settled.len())
}

pub async fn test_frpc_connectivity(
    docker: &Docker,
    settings: &ServerSettings,
    token: &[u8],
) -> Result<FrpcTestResult> {
    ensure_image(docker, &settings.frpc_image).await?;
    let test_id = Uuid::new_v4();
    let container_name = format!("webtop-manager-frpc-test-{test_id}");
    let test_resource_id = test_id.to_string();
    create_frpc_container(
        docker,
        &container_name,
        ("frpc-test", &test_resource_id),
        settings,
        token,
        &[],
        false,
    )
    .await?;
    if let Err(error) = docker.start_container(&container_name, None).await {
        let _ = remove_container_force(docker, &container_name).await;
        return Err(error).context("start frpc connectivity test");
    }

    let mut result = FrpcTestResult {
        success: false,
        code: FrpcTestCode::Unknown,
    };
    for _ in 0..40 {
        sleep(Duration::from_millis(250)).await;
        let logs = recent_logs(docker, &container_name)
            .await
            .unwrap_or_default();
        if logs.contains("login to server success") {
            result = FrpcTestResult {
                success: true,
                code: FrpcTestCode::Connected,
            };
            break;
        }
        if let Some(code) = classify_frpc_failure(&logs) {
            result.code = code;
            break;
        }
        let inspect = docker
            .inspect_container(&container_name, None)
            .await
            .context("inspect frpc connectivity test")?;
        if inspect.state.and_then(|state| state.running) == Some(false) {
            result.code = FrpcTestCode::ClientExited;
            break;
        }
    }
    remove_container_force(docker, &container_name).await?;
    Ok(result)
}

async fn create_frpc_container(
    docker: &Docker,
    container_name: &str,
    ownership: (&str, &str),
    settings: &ServerSettings,
    token: &[u8],
    proxies: &[crate::frp::Proxy],
    persistent: bool,
) -> Result<()> {
    let (resource_kind, resource_id) = ownership;
    let labels = HashMap::from([
        (OWNER_LABEL.into(), "managed".into()),
        (RESOURCE_ID_LABEL.into(), resource_id.into()),
        (RESOURCE_KIND_LABEL.into(), resource_kind.into()),
    ]);
    let host_config = HostConfig {
        network_mode: Some("host".into()),
        restart_policy: persistent.then_some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            maximum_retry_count: None,
        }),
        security_opt: Some(vec!["no-new-privileges=true".into()]),
        cap_drop: Some(vec!["ALL".into()]),
        memory: Some(128 * 1024 * 1024),
        pids_limit: Some(128),
        ..Default::default()
    };
    let config = ContainerCreateBody {
        image: Some(settings.frpc_image.clone()),
        cmd: Some(vec!["-c".into(), "/etc/frp/frpc.toml".into()]),
        labels: Some(labels),
        host_config: Some(host_config),
        ..Default::default()
    };
    let response = docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(container_name)
                    .build(),
            ),
            config,
        )
        .await
        .context("create frpc container")?;
    let rendered_config = if persistent {
        crate::frp::render(settings, proxies)
    } else {
        crate::frp::render_connectivity_test(settings)
    };
    let archive = frpc_archive(&rendered_config, token)?;
    if let Err(error) = docker
        .upload_to_container(
            &response.id,
            Some(UploadToContainerOptionsBuilder::default().path("/").build()),
            bollard::body_full(archive.into()),
        )
        .await
    {
        let _ = remove_container_force(docker, &response.id).await;
        return Err(error).context("install frpc configuration");
    }
    Ok(())
}

fn frpc_archive(config: &str, token: &[u8]) -> Result<Vec<u8>> {
    let mut archive = tar::Builder::new(Vec::new());
    append_archive_dir(&mut archive, "etc/frp")?;
    append_archive_dir(&mut archive, "run/webtop-manager")?;
    append_archive_file(&mut archive, "etc/frp/frpc.toml", config.as_bytes(), 0o644)?;
    append_archive_file(&mut archive, "run/webtop-manager/frp-token", token, 0o600)?;
    archive.finish()?;
    archive
        .into_inner()
        .context("finish frpc configuration archive")
}

fn append_archive_dir(archive: &mut tar::Builder<Vec<u8>>, path: &str) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_size(0);
    header.set_cksum();
    archive.append_data(&mut header, path, Cursor::new([]))?;
    Ok(())
}

fn append_archive_file(
    archive: &mut tar::Builder<Vec<u8>>,
    path: &str,
    contents: &[u8],
    mode: u32,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_mode(mode);
    header.set_size(contents.len() as u64);
    header.set_cksum();
    archive.append_data(&mut header, path, Cursor::new(contents))?;
    Ok(())
}

async fn inspect_managed_frpc(
    docker: &Docker,
    container_name: &str,
) -> Result<Option<bollard::models::ContainerInspectResponse>> {
    match docker.inspect_container(container_name, None).await {
        Ok(inspect) => {
            let labels = inspect
                .config
                .as_ref()
                .and_then(|config| config.labels.as_ref());
            anyhow::ensure!(
                labels
                    .and_then(|labels| labels.get(OWNER_LABEL))
                    .map(String::as_str)
                    == Some("managed")
                    && labels
                        .and_then(|labels| labels.get(RESOURCE_KIND_LABEL))
                        .map(String::as_str)
                        == Some("frpc"),
                "frpc container name is occupied by an unmanaged container"
            );
            Ok(Some(inspect))
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => Ok(None),
        Err(error) => Err(error).context("inspect shared frpc container"),
    }
}

async fn recent_logs(docker: &Docker, container_name: &str) -> Result<String> {
    let chunks = docker
        .logs(
            container_name,
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .tail("100")
                    .build(),
            ),
        )
        .try_collect::<Vec<_>>()
        .await?;
    Ok(chunks.into_iter().map(|chunk| chunk.to_string()).collect())
}

fn classify_frpc_failure(logs: &str) -> Option<FrpcTestCode> {
    let logs = logs.to_ascii_lowercase();
    if logs.contains("token") && (logs.contains("match") || logs.contains("auth")) {
        Some(FrpcTestCode::AuthenticationFailed)
    } else if logs.contains("no such host") || logs.contains("server misbehaving") {
        Some(FrpcTestCode::DnsFailed)
    } else if logs.contains("connection refused") {
        Some(FrpcTestCode::ConnectionRefused)
    } else if logs.contains("timed out") || logs.contains("i/o timeout") {
        Some(FrpcTestCode::TimedOut)
    } else {
        None
    }
}

async fn remove_container_force(docker: &Docker, container_name: &str) -> Result<()> {
    docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await?;
    Ok(())
}

fn not_created_frpc_status() -> FrpcServiceStatus {
    FrpcServiceStatus {
        state: FrpcServiceState::NotCreated,
        connected: false,
        image: None,
        started_at: None,
        exit_code: None,
    }
}

pub async fn start(docker: &Docker, container_id: &str) -> Result<()> {
    docker.start_container(container_id, None).await?;
    Ok(())
}

pub async fn stop(docker: &Docker, container_id: &str) -> Result<()> {
    docker
        .stop_container(
            container_id,
            Some(StopContainerOptionsBuilder::default().t(15).build()),
        )
        .await?;
    Ok(())
}

pub async fn restart(docker: &Docker, container_id: &str) -> Result<()> {
    docker.restart_container(container_id, None).await?;
    Ok(())
}

pub async fn remove(docker: &Docker, container_id: &str) -> Result<()> {
    docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptionsBuilder::default().build()),
        )
        .await?;
    Ok(())
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_names_are_safe() {
        assert_eq!(normalized_name("我的 Worktop"), "---worktop");
    }

    #[test]
    fn pull_allowlist_accepts_only_documented_official_tags() {
        assert!(is_official_image("lscr.io/linuxserver/webtop:ubuntu-mate"));
        assert!(is_official_image("lscr.io/linuxserver/webtop:latest"));
        assert!(!is_official_image("lscr.io/linuxserver/webtop:dev"));
        assert!(!is_official_image("example.invalid/webtop:latest"));
    }

    #[test]
    fn connectivity_failures_are_classified_without_returning_logs() {
        assert_eq!(
            classify_frpc_failure("dial tcp: connection refused"),
            Some(FrpcTestCode::ConnectionRefused)
        );
        assert_eq!(
            classify_frpc_failure("token in login doesn't match token from configuration"),
            Some(FrpcTestCode::AuthenticationFailed)
        );
    }

    #[test]
    fn reads_the_current_dynamic_host_port_from_docker_bindings() {
        let ports = HashMap::from([(
            "3001/tcp".into(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".into()),
                host_port: Some("32774".into()),
            }]),
        )]);

        assert_eq!(host_port_from_bindings(&ports, "3001/tcp"), Some(32774));
        assert_eq!(host_port_from_bindings(&ports, "3000/tcp"), None);
    }

    #[test]
    fn identifies_the_proxy_that_lost_a_remote_port_race() {
        let proxies = vec![
            crate::frp::Proxy {
                resource_id: "environment-a".into(),
                local_port: 32770,
                remote_port: 43000,
            },
            crate::frp::Proxy {
                resource_id: "environment-b".into(),
                local_port: 32771,
                remote_port: 43001,
            },
        ];
        let logs = r#"
[I] [webtop-environment-a] start proxy success
[W] [webtop-environment-b] start error: port already used
"#;

        let (conflicts, settled) = classify_proxy_registrations(logs, &proxies);

        assert_eq!(conflicts, BTreeSet::from(["environment-b".into()]));
        assert_eq!(settled, 2);
    }
}
