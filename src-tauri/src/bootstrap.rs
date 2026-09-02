use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use bollard::models::{ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ImportImageOptionsBuilder, ListContainersOptionsBuilder,
    ListImagesOptionsBuilder, RemoveContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Duration};
use webtop_contracts::{ApiError, ErrorCode, OWNER_LABEL, RESOURCE_KIND_LABEL};

use crate::controller_client::ControllerClient;

const DOCKER_SOCKET: &str = "/var/run/docker.sock";
const CONTROLLER_IMAGE: &str = concat!(
    "com.cue.webtop-manager/controller:",
    env!("CARGO_PKG_VERSION")
);
const CONTROLLER_NAME: &str = "webtop-manager-controller";
const REQUIRED_CONTROLLER_CAPABILITIES: &[&str] = &[
    "frpc_lifecycle_v1",
    "frps_guide_v3",
    "host_environment_paths_v1",
    "environment_publication_v1",
    "environment_credentials_v1",
    "official_image_delete_v1",
    "templates_v1",
    "template_transfer_v1",
    "operations_v1",
    "operation_output_v1",
    "operation_cancel_v1",
    "template_publication_reconcile_v1",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootState {
    DockerMissing,
    PermissionDenied,
    DockerUnavailable,
    ControllerUnavailable,
    Ready,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStatus {
    pub state: BootState,
    pub docker_version: Option<String>,
    pub docker_api_version: Option<String>,
    pub socket_mode: Option<u32>,
    pub socket_world_writable: bool,
    pub controller_version: Option<String>,
    pub host_uid: u32,
    pub host_gid: u32,
}

pub struct AppPaths {
    pub state_dir: PathBuf,
    pub environment_root: PathBuf,
    pub snapshot_root: PathBuf,
    pub staging_root: PathBuf,
    pub runtime_dir: PathBuf,
    pub controller_socket: PathBuf,
    pub host_uid: u32,
    pub host_gid: u32,
}

impl AppPaths {
    pub fn resolve(app: &AppHandle) -> Result<Self, ApiError> {
        let data = app.path().app_data_dir().map_err(|_| internal())?;
        let state_dir = data.join("controller");
        let environment_root = data.join("environments");
        let snapshot_root = data.join("snapshots");
        let staging_root = data.join("staging");
        let runtime_base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data.join("runtime"));
        let runtime_dir = runtime_base.join("webtop-manager");
        let controller_socket = runtime_dir.join("controller.sock");
        let host_uid = nix::unistd::getuid().as_raw();
        let host_gid = nix::unistd::getgid().as_raw();
        Ok(Self {
            state_dir,
            environment_root,
            snapshot_root,
            staging_root,
            runtime_dir,
            controller_socket,
            host_uid,
            host_gid,
        })
    }

    pub async fn create(&self) -> Result<(), ApiError> {
        for path in [
            &self.state_dir,
            &self.environment_root,
            &self.snapshot_root,
            &self.staging_root,
            &self.runtime_dir,
        ] {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|_| internal())?;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|_| internal())?;
        }
        Ok(())
    }
}

pub async fn docker_diagnostics_impl(app: &AppHandle) -> Result<BootStatus, ApiError> {
    let paths = AppPaths::resolve(app)?;
    let metadata = match tokio::fs::metadata(DOCKER_SOCKET).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(status(BootState::DockerMissing, None, None, None, None));
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Ok(status(BootState::PermissionDenied, None, None, None, None));
        }
        Err(_) => return Ok(status(BootState::DockerUnavailable, None, None, None, None)),
    };
    let mode = metadata.mode() & 0o777;
    let docker = match Docker::connect_with_socket_defaults() {
        Ok(docker) => docker,
        Err(_) => {
            return Ok(status(
                BootState::DockerUnavailable,
                None,
                None,
                Some(mode),
                None,
            ))
        }
    };
    let version = match docker.version().await {
        Ok(version) => version,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 403, ..
        }) => {
            return Ok(status(
                BootState::PermissionDenied,
                None,
                None,
                Some(mode),
                None,
            ));
        }
        Err(_) => {
            return Ok(status(
                BootState::DockerUnavailable,
                None,
                None,
                Some(mode),
                None,
            ))
        }
    };
    let controller_health = ControllerClient::new(paths.controller_socket)
        .request::<serde_json::Value, serde_json::Value>(
            "GET",
            "/v1/health",
            Option::<serde_json::Value>::None,
        )
        .await
        .ok();
    let controller_is_compatible = has_required_controller_capability(controller_health.as_ref());
    let state = if controller_is_compatible {
        BootState::Ready
    } else {
        BootState::ControllerUnavailable
    };
    let controller_version = controller_health
        .as_ref()
        .and_then(|value| value.get("controllerVersion")?.as_str().map(str::to_owned));
    Ok(status(
        state,
        version.version,
        version.api_version,
        Some(mode),
        controller_version,
    ))
}

pub async fn bootstrap_controller_impl(app: &AppHandle) -> Result<BootStatus, ApiError> {
    let diagnostics = docker_diagnostics_impl(app).await?;
    if !matches!(
        diagnostics.state,
        BootState::ControllerUnavailable | BootState::Ready
    ) {
        return Ok(diagnostics);
    }
    if matches!(diagnostics.state, BootState::Ready) {
        return Ok(diagnostics);
    }
    let paths = AppPaths::resolve(app)?;
    paths.create().await?;
    let docker = Docker::connect_with_socket_defaults().map_err(|_| docker_unavailable())?;

    let containers = docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&HashMap::from([(
                    "name".to_string(),
                    vec![CONTROLLER_NAME.to_string()],
                )]))
                .build(),
        ))
        .await
        .map_err(|_| docker_unavailable())?;
    if let Some(container) = containers.first() {
        let id = container.id.as_deref().ok_or_else(internal)?;
        let managed = container.labels.as_ref().is_some_and(|labels| {
            labels.get(OWNER_LABEL).map(String::as_str) == Some("managed")
                && labels.get(RESOURCE_KIND_LABEL).map(String::as_str) == Some("controller")
        });
        if !managed {
            return Err(ApiError {
                code: ErrorCode::ControllerUnavailable,
                params: BTreeMap::new(),
            });
        }

        // A controller can be healthy according to an older API while lacking
        // routes required by the current desktop app. Import the bundled image
        // before replacing the app-owned container so an import failure leaves
        // the existing process untouched. Persistent state lives in bind mounts.
        ensure_controller_image(app, &docker, true).await?;
        docker
            .remove_container(
                id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .map_err(|_| docker_unavailable())?;
        create_controller(&docker, &paths).await?;
    } else {
        ensure_controller_image(app, &docker, false).await?;
        create_controller(&docker, &paths).await?;
    }

    for _ in 0..40 {
        sleep(Duration::from_millis(250)).await;
        let status = docker_diagnostics_impl(app).await?;
        if matches!(status.state, BootState::Ready) {
            return Ok(status);
        }
    }
    Err(ApiError {
        code: ErrorCode::ControllerUnavailable,
        params: BTreeMap::new(),
    })
}

async fn ensure_controller_image(
    app: &AppHandle,
    docker: &Docker,
    force_import: bool,
) -> Result<(), ApiError> {
    if !force_import {
        let images = docker
            .list_images(Some(
                ListImagesOptionsBuilder::default()
                    .filters(&HashMap::from([(
                        "reference".to_string(),
                        vec![CONTROLLER_IMAGE.to_string()],
                    )]))
                    .build(),
            ))
            .await
            .map_err(|_| docker_unavailable())?;
        if !images.is_empty() {
            return Ok(());
        }
    }
    let asset = app
        .path()
        .resolve(
            "assets/controller-image.tar.zst",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|_| controller_image_missing())?;
    if !asset.is_file() {
        return Err(controller_image_missing());
    }
    // Image loading is deliberately delegated to a fixed Docker API operation;
    // no image name, command or path is accepted from the WebView. The release
    // pipeline replaces this development placeholder with the signed OCI asset.
    let compressed = tokio::fs::read(asset)
        .await
        .map_err(|_| controller_image_missing())?;
    let decoded = tokio::task::spawn_blocking(move || zstd::decode_all(&compressed[..]))
        .await
        .map_err(|_| internal())?
        .map_err(|_| controller_image_missing())?;
    let body = bollard::body_full(decoded.into());
    let mut stream = docker.import_image(
        ImportImageOptionsBuilder::default().quiet(true).build(),
        body,
        None,
    );
    while let Some(result) = stream.next().await {
        result.map_err(|_| docker_unavailable())?;
    }
    let images = docker
        .list_images(Some(
            ListImagesOptionsBuilder::default()
                .filters(&HashMap::from([(
                    "reference".to_string(),
                    vec![CONTROLLER_IMAGE.to_string()],
                )]))
                .build(),
        ))
        .await
        .map_err(|_| docker_unavailable())?;
    if images.is_empty() {
        return Err(controller_image_missing());
    }
    Ok(())
}

async fn create_controller(docker: &Docker, paths: &AppPaths) -> Result<(), ApiError> {
    let docker_socket_gid = tokio::fs::metadata(DOCKER_SOCKET)
        .await
        .map_err(|_| docker_unavailable())?
        .gid();
    let binds = vec![
        format!("{DOCKER_SOCKET}:{DOCKER_SOCKET}:rw"),
        format!("{}:/state:rw", paths.state_dir.display()),
        format!("{}:/data/environments:rw", paths.environment_root.display()),
        format!("{}:/data/snapshots:rw", paths.snapshot_root.display()),
        format!("{}:/data/staging:rw", paths.staging_root.display()),
        format!("{}:/run/webtop-manager:rw", paths.runtime_dir.display()),
    ];
    let labels = HashMap::from([
        (OWNER_LABEL.to_owned(), "managed".to_owned()),
        (RESOURCE_KIND_LABEL.to_owned(), "controller".to_owned()),
    ]);
    let config = ContainerCreateBody {
        image: Some(CONTROLLER_IMAGE.to_owned()),
        user: Some(format!("{}:{}", paths.host_uid, paths.host_gid)),
        env: Some(vec![format!(
            "WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT={}",
            paths.environment_root.display()
        )]),
        labels: Some(labels),
        host_config: Some(HostConfig {
            binds: Some(binds),
            readonly_rootfs: Some(true),
            network_mode: Some("none".into()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            security_opt: Some(vec!["no-new-privileges=true".into()]),
            tmpfs: Some(HashMap::from([(
                "/tmp".to_owned(),
                "rw,noexec,nosuid,size=64m".to_owned(),
            )])),
            cap_drop: Some(vec!["ALL".into()]),
            group_add: Some(vec![docker_socket_gid.to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let response = docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(CONTROLLER_NAME)
                    .build(),
            ),
            config,
        )
        .await
        .map_err(|_| docker_unavailable())?;
    docker
        .start_container(&response.id, None)
        .await
        .map_err(|_| docker_unavailable())?;
    Ok(())
}

fn status(
    state: BootState,
    docker_version: Option<String>,
    docker_api_version: Option<String>,
    socket_mode: Option<u32>,
    controller_version: Option<String>,
) -> BootStatus {
    BootStatus {
        state,
        docker_version,
        docker_api_version,
        socket_mode,
        socket_world_writable: socket_mode.is_some_and(|mode| mode & 0o002 != 0),
        controller_version,
        host_uid: nix::unistd::getuid().as_raw(),
        host_gid: nix::unistd::getgid().as_raw(),
    }
}

fn docker_unavailable() -> ApiError {
    ApiError {
        code: ErrorCode::DockerUnavailable,
        params: BTreeMap::new(),
    }
}

fn controller_image_missing() -> ApiError {
    ApiError {
        code: ErrorCode::ControllerImageMissing,
        params: BTreeMap::new(),
    }
}

fn internal() -> ApiError {
    ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    }
}

fn has_required_controller_capability(health: Option<&serde_json::Value>) -> bool {
    let Some(capabilities) = health
        .and_then(|value| value.get("capabilities"))
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    REQUIRED_CONTROLLER_CAPABILITIES.iter().all(|required| {
        capabilities
            .iter()
            .any(|capability| capability.as_str() == Some(required))
    })
}

#[cfg(test)]
mod tests {
    use super::has_required_controller_capability;

    #[test]
    fn rejects_controller_health_missing_required_capabilities() {
        let legacy = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": "0.1.0"
        });
        let current = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": "0.1.0",
            "capabilities": ["frpc_lifecycle_v1", "frps_guide_v2", "frps_guide_v3", "host_environment_paths_v1", "environment_publication_v1", "environment_credentials_v1", "official_image_delete_v1", "templates_v1", "template_transfer_v1", "operations_v1", "operation_output_v1", "operation_cancel_v1", "template_publication_reconcile_v1"]
        });
        let previous = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": "0.1.0",
            "capabilities": ["frpc_lifecycle_v1", "frps_guide_v2", "frps_guide_v3", "host_environment_paths_v1", "environment_publication_v1", "environment_credentials_v1", "official_image_delete_v1", "templates_v1", "template_transfer_v1", "operations_v1", "operation_output_v1", "operation_cancel_v1"]
        });

        assert!(!has_required_controller_capability(Some(&legacy)));
        assert!(!has_required_controller_capability(Some(&previous)));
        assert!(has_required_controller_capability(Some(&current)));
    }
}
