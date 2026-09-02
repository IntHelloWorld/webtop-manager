use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use bollard::models::{ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ImportImageOptionsBuilder, ListContainersOptionsBuilder,
    ListImagesOptionsBuilder, RemoveContainerOptionsBuilder, RenameContainerOptionsBuilder,
    StopContainerOptionsBuilder,
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
const CONTROLLER_BACKUP_NAME: &str = "controller-backup";
const REQUIRED_CONTROLLER_CAPABILITIES: &[&str] = &[
    "frpc_lifecycle_v1",
    "frp_token_recovery_v1",
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
    "durable_image_pull_v1",
    "controller_schema_v1",
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
    pub state_backup_dir: PathBuf,
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
        let state_backup_dir = data.join(CONTROLLER_BACKUP_NAME);
        let environment_root = data.join("environments");
        let snapshot_root = data.join("snapshots");
        let staging_root = data.join("staging");
        let runtime_dir = persistent_runtime_dir(&data);
        let controller_socket = runtime_dir.join("controller.sock");
        let host_uid = nix::unistd::getuid().as_raw();
        let host_gid = nix::unistd::getgid().as_raw();
        Ok(Self {
            state_dir,
            state_backup_dir,
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

fn persistent_runtime_dir(data_dir: &Path) -> PathBuf {
    // The controller has an `unless-stopped` restart policy, so its bind-mount
    // source must survive logout and reboot. If this lived in XDG_RUNTIME_DIR,
    // Docker could restart first and recreate the missing directory as root,
    // preventing the unprivileged controller from binding its Unix socket.
    data_dir.join("runtime").join("webtop-manager")
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
    let controller_is_compatible = has_compatible_controller(controller_health.as_ref());
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
    let controller = containers.into_iter().find(|container| {
        container.names.as_ref().is_some_and(|names| {
            names
                .iter()
                .any(|name| name == &format!("/{CONTROLLER_NAME}"))
        })
    });
    if let Some(container) = controller {
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

        // Import before stopping the existing controller. An invalid bundle
        // therefore cannot disturb the last-known-good process.
        ensure_controller_image(app, &docker, true).await?;
        stop_controller_if_running(&docker, id).await?;
        if let Err(error) = backup_controller_state(&paths).await {
            let _ = docker.start_container(id, None).await;
            return Err(error);
        }

        let candidate_name = format!("{CONTROLLER_NAME}-candidate-{}", uuid::Uuid::new_v4());
        let candidate_id = match create_controller(&docker, &paths, &candidate_name).await {
            Ok(candidate_id) => candidate_id,
            Err(error) => {
                restore_state_and_restart(&docker, &paths, id).await?;
                return Err(error);
            }
        };
        if wait_for_current_controller(app).await.is_none() {
            rollback_controller_upgrade(&docker, &paths, id, &candidate_id, false).await?;
            return Err(controller_unavailable());
        }

        let rollback_name = format!("{CONTROLLER_NAME}-rollback-{}", uuid::Uuid::new_v4());
        if docker
            .rename_container(
                id,
                RenameContainerOptionsBuilder::default()
                    .name(&rollback_name)
                    .build(),
            )
            .await
            .is_err()
        {
            rollback_controller_upgrade(&docker, &paths, id, &candidate_id, false).await?;
            return Err(docker_unavailable());
        }
        if docker
            .rename_container(
                &candidate_id,
                RenameContainerOptionsBuilder::default()
                    .name(CONTROLLER_NAME)
                    .build(),
            )
            .await
            .is_err()
        {
            rollback_controller_upgrade(&docker, &paths, id, &candidate_id, true).await?;
            return Err(docker_unavailable());
        }
        // The healthy candidate has already taken the stable name. A stale
        // stopped rollback container is harmless and can be cleaned up on the
        // next bootstrap, so do not turn a successful upgrade into a failure.
        let _ = docker
            .remove_container(
                id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
    } else {
        ensure_controller_image(app, &docker, false).await?;
        create_controller(&docker, &paths, CONTROLLER_NAME).await?;
    }

    if let Some(status) = wait_for_current_controller(app).await {
        return Ok(status);
    }
    Err(controller_unavailable())
}

async fn wait_for_current_controller(app: &AppHandle) -> Option<BootStatus> {
    for _ in 0..40 {
        sleep(Duration::from_millis(250)).await;
        if let Ok(status) = docker_diagnostics_impl(app).await {
            if matches!(status.state, BootState::Ready) {
                return Some(status);
            }
        }
    }
    None
}

async fn stop_controller_if_running(docker: &Docker, id: &str) -> Result<(), ApiError> {
    let inspect = docker
        .inspect_container(id, None)
        .await
        .map_err(|_| docker_unavailable())?;
    if inspect.state.and_then(|state| state.running) == Some(true) {
        docker
            .stop_container(
                id,
                Some(StopContainerOptionsBuilder::default().t(20).build()),
            )
            .await
            .map_err(|_| docker_unavailable())?;
    }
    Ok(())
}

async fn rollback_controller_upgrade(
    docker: &Docker,
    paths: &AppPaths,
    previous_id: &str,
    candidate_id: &str,
    restore_previous_name: bool,
) -> Result<(), ApiError> {
    let _ = docker
        .remove_container(
            candidate_id,
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await;
    let rename_failed = if restore_previous_name {
        docker
            .rename_container(
                previous_id,
                RenameContainerOptionsBuilder::default()
                    .name(CONTROLLER_NAME)
                    .build(),
            )
            .await
            .is_err()
    } else {
        false
    };
    let restore_result = rollback_controller_state(paths).await;
    let restart_result = docker
        .start_container(previous_id, None)
        .await
        .map_err(|_| docker_unavailable());
    if rename_failed {
        return Err(docker_unavailable());
    }
    restore_result?;
    restart_result
}

async fn restore_state_and_restart(
    docker: &Docker,
    paths: &AppPaths,
    previous_id: &str,
) -> Result<(), ApiError> {
    let restore_result = rollback_controller_state(paths).await;
    let restart_result = docker
        .start_container(previous_id, None)
        .await
        .map_err(|_| docker_unavailable());
    restore_result?;
    restart_result
}

async fn backup_controller_state(paths: &AppPaths) -> Result<(), ApiError> {
    let source = paths.state_dir.clone();
    let backup = paths.state_backup_dir.clone();
    tokio::task::spawn_blocking(move || replace_backup(&source, &backup))
        .await
        .map_err(|_| internal())?
        .map_err(|_| internal())
}

async fn rollback_controller_state(paths: &AppPaths) -> Result<(), ApiError> {
    let backup = paths.state_backup_dir.clone();
    let destination = paths.state_dir.clone();
    tokio::task::spawn_blocking(move || restore_backup(&backup, &destination))
        .await
        .map_err(|_| internal())?
        .map_err(|_| internal())
}

fn replace_backup(source: &Path, backup: &Path) -> std::io::Result<()> {
    let parent = backup.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "backup has no parent")
    })?;
    let partial = parent.join(format!(
        ".{CONTROLLER_BACKUP_NAME}.partial-{}",
        uuid::Uuid::new_v4()
    ));
    let previous = parent.join(format!(
        ".{CONTROLLER_BACKUP_NAME}.previous-{}",
        uuid::Uuid::new_v4()
    ));
    if let Err(error) = copy_tree(source, &partial) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(error);
    }
    let had_previous = backup.exists();
    if had_previous {
        std::fs::rename(backup, &previous)?;
    }
    if let Err(error) = std::fs::rename(&partial, backup) {
        if had_previous {
            let _ = std::fs::rename(&previous, backup);
        }
        let _ = std::fs::remove_dir_all(&partial);
        return Err(error);
    }
    if had_previous {
        std::fs::remove_dir_all(previous)?;
    }
    std::fs::File::open(parent)?.sync_all()
}

fn restore_backup(backup: &Path, destination: &Path) -> std::io::Result<()> {
    if !backup.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "controller backup is missing",
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "state has no parent")
    })?;
    let restored = parent.join(format!(".controller-restored-{}", uuid::Uuid::new_v4()));
    let failed = parent.join(format!(".controller-failed-{}", uuid::Uuid::new_v4()));
    if let Err(error) = copy_tree(backup, &restored) {
        let _ = std::fs::remove_dir_all(&restored);
        return Err(error);
    }
    std::fs::rename(destination, &failed)?;
    if let Err(error) = std::fs::rename(&restored, destination) {
        let _ = std::fs::rename(&failed, destination);
        let _ = std::fs::remove_dir_all(&restored);
        return Err(error);
    }
    std::fs::remove_dir_all(failed)?;
    std::fs::File::open(parent)?.sync_all()
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "controller state root must be a real directory",
        ));
    }
    std::fs::create_dir(destination)?;
    std::fs::set_permissions(destination, metadata.permissions())?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "symlinks are forbidden in controller state",
            ));
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            std::fs::copy(&source_path, &destination_path)?;
            std::fs::set_permissions(&destination_path, metadata.permissions())?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "special files are forbidden in controller state",
            ));
        }
    }
    std::fs::File::open(destination)?.sync_all()
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

async fn create_controller(
    docker: &Docker,
    paths: &AppPaths,
    container_name: &str,
) -> Result<String, ApiError> {
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
                    .name(container_name)
                    .build(),
            ),
            config,
        )
        .await
        .map_err(|_| docker_unavailable())?;
    if docker.start_container(&response.id, None).await.is_err() {
        let _ = docker
            .remove_container(
                &response.id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        return Err(docker_unavailable());
    }
    Ok(response.id)
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

fn controller_unavailable() -> ApiError {
    ApiError {
        code: ErrorCode::ControllerUnavailable,
        params: BTreeMap::new(),
    }
}

fn internal() -> ApiError {
    ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    }
}

fn has_compatible_controller(health: Option<&serde_json::Value>) -> bool {
    if health
        .and_then(|value| value.get("controllerVersion"))
        .and_then(serde_json::Value::as_str)
        != Some(env!("CARGO_PKG_VERSION"))
    {
        return false;
    }
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
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        copy_tree, has_compatible_controller, persistent_runtime_dir, replace_backup,
        restore_backup,
    };

    #[test]
    fn controller_runtime_directory_is_persistent_application_data() {
        let data_dir = Path::new("/home/example/.local/share/com.cue.webtop-manager");

        assert_eq!(
            persistent_runtime_dir(data_dir),
            data_dir.join("runtime/webtop-manager")
        );
    }

    #[test]
    fn rejects_controller_health_missing_required_capabilities() {
        let legacy = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": env!("CARGO_PKG_VERSION")
        });
        let current = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["frpc_lifecycle_v1", "frp_token_recovery_v1", "frps_guide_v2", "frps_guide_v3", "host_environment_paths_v1", "environment_publication_v1", "environment_credentials_v1", "official_image_delete_v1", "templates_v1", "template_transfer_v1", "operations_v1", "operation_output_v1", "operation_cancel_v1", "template_publication_reconcile_v1", "durable_image_pull_v1", "controller_schema_v1"]
        });
        let previous = serde_json::json!({
            "apiVersion": "v1",
            "controllerVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["frpc_lifecycle_v1", "frps_guide_v2", "frps_guide_v3", "host_environment_paths_v1", "environment_publication_v1", "environment_credentials_v1", "official_image_delete_v1", "templates_v1", "template_transfer_v1", "operations_v1", "operation_output_v1", "operation_cancel_v1"]
        });

        let old_version = serde_json::json!({
            "controllerVersion": "0.0.0",
            "capabilities": current["capabilities"].clone()
        });

        assert!(!has_compatible_controller(Some(&legacy)));
        assert!(!has_compatible_controller(Some(&previous)));
        assert!(!has_compatible_controller(Some(&old_version)));
        assert!(has_compatible_controller(Some(&current)));
    }

    #[test]
    fn controller_backup_preserves_state_and_permissions() {
        let directory = tempdir().unwrap();
        let state = directory.path().join("controller");
        let backup = directory.path().join("controller-backup");
        std::fs::create_dir(&state).unwrap();
        std::fs::create_dir(state.join("secrets")).unwrap();
        std::fs::write(state.join("controller.sqlite3"), b"schema-v1").unwrap();
        std::fs::write(state.join("secrets/frp-token"), b"secret-token").unwrap();
        std::fs::set_permissions(
            state.join("secrets/frp-token"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        replace_backup(&state, &backup).unwrap();
        std::fs::write(state.join("controller.sqlite3"), b"broken-migration").unwrap();
        restore_backup(&backup, &state).unwrap();

        assert_eq!(
            std::fs::read(state.join("controller.sqlite3")).unwrap(),
            b"schema-v1"
        );
        assert_eq!(
            std::fs::read(state.join("secrets/frp-token")).unwrap(),
            b"secret-token"
        );
        assert_eq!(
            std::fs::metadata(state.join("secrets/frp-token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn controller_backup_rejects_symlinks() {
        let directory = tempdir().unwrap();
        let state = directory.path().join("controller");
        let destination = directory.path().join("copy");
        std::fs::create_dir(&state).unwrap();
        std::fs::write(directory.path().join("outside"), b"secret").unwrap();
        symlink(
            directory.path().join("outside"),
            state.join("unexpected-link"),
        )
        .unwrap();

        let error = copy_tree(&state, &destination).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
