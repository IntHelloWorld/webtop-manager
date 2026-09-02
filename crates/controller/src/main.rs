mod api;
mod database;
mod docker;
mod frp;
mod templates;

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bollard::Docker;
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::{reconcile_runtime_state, router, AppState};
use crate::database::Database;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "webtop_controller=info".into()),
        )
        .with_target(false)
        .compact()
        .init();

    let state_dir = required_absolute_path("WEBTOP_MANAGER_STATE_DIR", "/state")?;
    let environment_root =
        required_absolute_path("WEBTOP_MANAGER_ENVIRONMENT_ROOT", "/data/environments")?;
    let host_environment_root =
        required_absolute_path("WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT", "/data/environments")?;
    let snapshot_root = required_absolute_path("WEBTOP_MANAGER_SNAPSHOT_ROOT", "/data/snapshots")?;
    let staging_root = required_absolute_path("WEBTOP_MANAGER_STAGING_ROOT", "/data/staging")?;
    let socket_path = required_absolute_path(
        "WEBTOP_MANAGER_SOCKET",
        "/run/webtop-manager/controller.sock",
    )?;

    tokio::fs::create_dir_all(&state_dir).await?;
    let secret_dir = state_dir.join("secrets");
    tokio::fs::create_dir_all(&secret_dir).await?;
    tokio::fs::set_permissions(&secret_dir, std::fs::Permissions::from_mode(0o700)).await?;
    tokio::fs::create_dir_all(&environment_root).await?;
    tokio::fs::create_dir_all(&snapshot_root).await?;
    tokio::fs::create_dir_all(&staging_root).await?;
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::try_exists(&socket_path).await? {
        tokio::fs::remove_file(&socket_path).await?;
    }

    let database = Database::open(&state_dir.join("controller.sqlite3")).await?;
    let recovered = database.recover_unfinished_operations().await?;
    if recovered > 0 {
        tracing::warn!(recovered, "marked interrupted operations retryable");
    }
    cleanup_partial_files(&snapshot_root).await?;
    cleanup_partial_files(&staging_root).await?;
    let docker = Docker::connect_with_socket_defaults().context("connect to Docker socket")?;
    let listener = UnixListener::bind(&socket_path).context("bind controller socket")?;
    tokio::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).await?;

    let state = AppState {
        database,
        docker,
        environment_root,
        host_environment_root,
        snapshot_root,
        staging_root,
        server_token_path: secret_dir.join("frp-token"),
        pull_cancellations: Arc::new(Mutex::new(HashMap::new())),
        operation_cancellations: Arc::new(Mutex::new(HashMap::new())),
        active_resources: Arc::new(Mutex::new(HashSet::new())),
    };
    reconcile_runtime_state(&state).await;
    info!(socket = %socket_path.display(), "controller ready");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn cleanup_partial_files(root: &std::path::Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".partial") || name.contains(".partial-") {
            if entry.file_type().await?.is_dir() {
                tokio::fs::remove_dir_all(path).await?;
            } else {
                tokio::fs::remove_file(path).await?;
            }
        }
    }
    Ok(())
}

fn required_absolute_path(variable: &str, fallback: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env::var_os(variable).unwrap_or_else(|| fallback.into()));
    anyhow::ensure!(path.is_absolute(), "{variable} must be absolute");
    Ok(path)
}
