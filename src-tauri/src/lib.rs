mod bootstrap;
mod controller_client;

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bollard::Docker;
use bootstrap::{bootstrap_controller_impl, docker_diagnostics_impl, AppPaths, BootStatus};
use controller_client::ControllerClient;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use webtop_contracts::{
    ApiError, EnvironmentSpec, ErrorCode, ImagePullPhase, ImagePullProgress, ServerSettings,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TemplateTransferProgress {
    phase: String,
    message: String,
    current_bytes: u64,
    total_bytes: u64,
}

#[derive(Default)]
struct TemplateExportDestinations(Mutex<HashMap<uuid::Uuid, PathBuf>>);

#[derive(Default)]
struct TemplateImportSources(Mutex<HashMap<uuid::Uuid, PathBuf>>);

#[derive(Default)]
struct TemplateTransferCancellations(Mutex<HashMap<uuid::Uuid, Arc<AtomicBool>>>);

static LAST_LOCAL_DESKTOP_OPEN: OnceLock<Mutex<Option<(u16, Instant)>>> = OnceLock::new();

fn claim_local_desktop_open(local_port: u16) -> bool {
    let now = Instant::now();
    let mut last_open = LAST_LOCAL_DESKTOP_OPEN
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if last_open.as_ref().is_some_and(|(port, opened_at)| {
        *port == local_port && now.duration_since(*opened_at) < Duration::from_secs(2)
    }) {
        return false;
    }
    *last_open = Some((local_port, now));
    true
}

fn send_template_transfer(
    channel: &Channel<TemplateTransferProgress>,
    phase: &str,
    message: &str,
    current_bytes: u64,
    total_bytes: u64,
) {
    let _ = channel.send(TemplateTransferProgress {
        phase: phase.into(),
        message: message.into(),
        current_bytes,
        total_bytes,
    });
}

#[tauri::command]
async fn docker_diagnostics(app: AppHandle) -> Result<BootStatus, ApiError> {
    docker_diagnostics_impl(&app).await
}

#[tauri::command]
async fn bootstrap_controller(app: AppHandle) -> Result<BootStatus, ApiError> {
    bootstrap_controller_impl(&app).await
}

#[tauri::command]
async fn list_environments(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/environments", Option::<Value>::None).await
}

#[tauri::command]
async fn list_templates(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/templates", Option::<Value>::None).await
}

#[tauri::command]
async fn template_preflight(app: AppHandle, id: String) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "POST",
        &format!("/v1/environments/{id}/template-preflight"),
        Option::<Value>::None,
    )
    .await
}

#[tauri::command]
async fn create_template(app: AppHandle, request_body: Value) -> Result<Value, ApiError> {
    request(&app, "POST", "/v1/templates", Some(request_body)).await
}

#[tauri::command]
async fn create_environment_from_template(
    app: AppHandle,
    id: String,
    spec: EnvironmentSpec,
) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request(
        &app,
        "POST",
        &format!("/v1/templates/{id}/environments"),
        Some(serde_json::json!({ "spec": spec })),
    )
    .await
}

#[tauri::command]
async fn check_template_sources(
    app: AppHandle,
    template_ids: Vec<String>,
) -> Result<Value, ApiError> {
    let template_ids = template_ids
        .into_iter()
        .map(|id| uuid::Uuid::parse_str(&id).map_err(|_| invalid_request()))
        .collect::<Result<Vec<_>, _>>()?;
    request(
        &app,
        "POST",
        "/v1/templates/source-checks",
        Some(serde_json::json!({ "templateIds": template_ids })),
    )
    .await
}

#[tauri::command]
async fn export_template(app: AppHandle, id: String) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "POST",
        &format!("/v1/templates/{id}/exports"),
        Option::<Value>::None,
    )
    .await
}

#[tauri::command]
async fn get_operation(app: AppHandle, id: String) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "GET",
        &format!("/v1/operations/{id}"),
        Option::<Value>::None,
    )
    .await
}

#[tauri::command]
async fn cancel_operation(app: AppHandle, id: String) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "DELETE",
        &format!("/v1/operations/{id}"),
        Option::<Value>::None,
    )
    .await
}

#[tauri::command]
async fn import_template_preflight(
    app: AppHandle,
    staging_file_id: String,
) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&staging_file_id).map_err(|_| invalid_request())?;
    request(
        &app,
        "POST",
        "/v1/template-imports/preflight",
        Some(serde_json::json!({ "stagingFileId": id })),
    )
    .await
}

#[tauri::command]
async fn import_template(app: AppHandle, request_body: Value) -> Result<Value, ApiError> {
    request(&app, "POST", "/v1/template-imports", Some(request_body)).await
}

#[tauri::command]
async fn delete_template(
    app: AppHandle,
    id: String,
    confirmation_name: String,
) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request(
        &app,
        "DELETE",
        &format!("/v1/templates/{id}"),
        Some(serde_json::json!({ "confirmationName": confirmation_name })),
    )
    .await
}

#[tauri::command]
async fn select_template_import(
    app: AppHandle,
    sources: State<'_, TemplateImportSources>,
) -> Result<Option<String>, ApiError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Webtop template", &["wtmpl"])
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    let Some(file) = receiver.await.map_err(|_| internal_error())? else {
        return Ok(None);
    };
    let source = file.into_path().map_err(|_| invalid_request())?;
    if source.extension().and_then(|value| value.to_str()) != Some("wtmpl") {
        return Err(invalid_request());
    }
    let id = uuid::Uuid::new_v4();
    sources
        .0
        .lock()
        .map_err(|_| internal_error())?
        .insert(id, source);
    Ok(Some(id.to_string()))
}

#[tauri::command]
async fn stage_template_import(
    app: AppHandle,
    sources: State<'_, TemplateImportSources>,
    cancellations: State<'_, TemplateTransferCancellations>,
    source_id: String,
    transfer_id: String,
    on_progress: Channel<TemplateTransferProgress>,
) -> Result<Option<String>, ApiError> {
    let source_id = uuid::Uuid::parse_str(&source_id).map_err(|_| invalid_request())?;
    let transfer_id = uuid::Uuid::parse_str(&transfer_id).map_err(|_| invalid_request())?;
    let source = sources
        .0
        .lock()
        .map_err(|_| internal_error())?
        .get(&source_id)
        .cloned()
        .ok_or_else(invalid_request)?;
    let paths = AppPaths::resolve(&app)?;
    let id = uuid::Uuid::new_v4();
    let destination = paths.staging_root.join(format!("{id}.wtmpl"));
    let total_bytes = tokio::fs::metadata(&source)
        .await
        .map_err(|_| internal_error())?
        .len();
    send_template_transfer(
        &on_progress,
        "copying",
        "[desktop] copying package into protected staging",
        0,
        total_bytes,
    );
    let cancelled = cancellations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .entry(transfer_id)
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    let result = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&paths.staging_root).map_err(|_| internal_error())?;
        let copied = copy_atomic_0600(
            &source,
            &destination,
            |current, total| {
                send_template_transfer(
                    &on_progress,
                    "copying",
                    "[desktop] staging import package",
                    current,
                    total,
                );
            },
            || cancelled.load(Ordering::Relaxed),
        )?;
        if !copied {
            send_template_transfer(
                &on_progress,
                "cancelled",
                "[desktop] import stopped; partial staging copy removed",
                0,
                total_bytes,
            );
            return Ok(false);
        }
        send_template_transfer(
            &on_progress,
            "complete",
            "[desktop] import package staged atomically",
            total_bytes,
            total_bytes,
        );
        Ok(true)
    })
    .await
    .map_err(|_| internal_error())?;
    cancellations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .remove(&transfer_id);
    sources
        .0
        .lock()
        .map_err(|_| internal_error())?
        .remove(&source_id);
    Ok(if result? { Some(id.to_string()) } else { None })
}

#[tauri::command]
async fn select_template_export(
    app: AppHandle,
    destinations: State<'_, TemplateExportDestinations>,
    suggested_name: String,
) -> Result<Option<String>, ApiError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Webtop template", &["wtmpl"])
        .set_file_name(suggested_name)
        .save_file(move |file| {
            let _ = sender.send(file);
        });
    let Some(file) = receiver.await.map_err(|_| internal_error())? else {
        return Ok(None);
    };
    let mut destination = file.into_path().map_err(|_| invalid_request())?;
    if destination.extension().is_none() {
        destination.set_extension("wtmpl");
    }
    let id = uuid::Uuid::new_v4();
    destinations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .insert(id, destination);
    Ok(Some(id.to_string()))
}

#[tauri::command]
async fn save_template_export(
    app: AppHandle,
    destinations: State<'_, TemplateExportDestinations>,
    cancellations: State<'_, TemplateTransferCancellations>,
    staging_file_id: String,
    destination_id: String,
    transfer_id: String,
    on_progress: Channel<TemplateTransferProgress>,
) -> Result<bool, ApiError> {
    let id = uuid::Uuid::parse_str(&staging_file_id).map_err(|_| invalid_request())?;
    let destination_id = uuid::Uuid::parse_str(&destination_id).map_err(|_| invalid_request())?;
    let transfer_id = uuid::Uuid::parse_str(&transfer_id).map_err(|_| invalid_request())?;
    let paths = AppPaths::resolve(&app)?;
    let source = paths.staging_root.join(format!("{id}.wtmpl"));
    if !source.is_file() {
        return Err(invalid_request());
    }
    let destination = destinations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .get(&destination_id)
        .cloned()
        .ok_or_else(invalid_request)?;
    let total_bytes = tokio::fs::metadata(&source)
        .await
        .map_err(|_| internal_error())?
        .len();
    send_template_transfer(
        &on_progress,
        "copying",
        "[desktop] copying staged export to destination",
        0,
        total_bytes,
    );
    let cancelled = cancellations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .entry(transfer_id)
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    let source_for_cleanup = source.clone();
    let result = tokio::task::spawn_blocking(move || {
        let copied = copy_atomic_0600(
            &source,
            &destination,
            |current, total| {
                send_template_transfer(
                    &on_progress,
                    "copying",
                    "[desktop] writing export package",
                    current,
                    total,
                );
            },
            || cancelled.load(Ordering::Relaxed),
        )?;
        if !copied {
            let _ = std::fs::remove_file(source);
            send_template_transfer(
                &on_progress,
                "cancelled",
                "[desktop] export stopped; staged package and partial destination removed",
                0,
                total_bytes,
            );
            return Ok(false);
        }
        std::fs::remove_file(source).map_err(|_| internal_error())?;
        send_template_transfer(
            &on_progress,
            "complete",
            "[desktop] export saved atomically; staged copy removed",
            total_bytes,
            total_bytes,
        );
        Ok(true)
    })
    .await
    .map_err(|_| internal_error())?;
    cancellations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .remove(&transfer_id);
    let copied = result?;
    if !copied {
        let _ = std::fs::remove_file(source_for_cleanup);
    }
    destinations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .remove(&destination_id);
    Ok(copied)
}

#[tauri::command]
fn cancel_template_transfer(
    cancellations: State<'_, TemplateTransferCancellations>,
    transfer_id: String,
) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&transfer_id).map_err(|_| invalid_request())?;
    let cancellation = cancellations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .entry(id)
        .or_insert_with(|| Arc::new(AtomicBool::new(true)))
        .clone();
    cancellation.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn discard_template_export_destination(
    destinations: State<'_, TemplateExportDestinations>,
    destination_id: String,
) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&destination_id).map_err(|_| invalid_request())?;
    destinations
        .0
        .lock()
        .map_err(|_| internal_error())?
        .remove(&id);
    Ok(())
}

#[tauri::command]
fn discard_template_staging(app: AppHandle, staging_file_id: String) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&staging_file_id).map_err(|_| invalid_request())?;
    let path = AppPaths::resolve(&app)?
        .staging_root
        .join(format!("{id}.wtmpl"));
    if path.exists() {
        std::fs::remove_file(path).map_err(|_| internal_error())?;
    }
    Ok(())
}

fn copy_atomic_0600(
    source: &std::path::Path,
    destination: &std::path::Path,
    mut on_progress: impl FnMut(u64, u64),
    mut should_cancel: impl FnMut() -> bool,
) -> Result<bool, ApiError> {
    let parent = destination.parent().ok_or_else(invalid_request)?;
    let temporary = parent.join(format!(
        ".{}.partial-{}",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(invalid_request)?,
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> std::io::Result<bool> {
        let mut input = File::open(source)?;
        let total = input.metadata()?.len();
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        let mut buffer = vec![0_u8; 1024 * 1024];
        let mut copied = 0_u64;
        let mut last_reported = 0_u64;
        loop {
            if should_cancel() {
                return Ok(false);
            }
            let read = std::io::Read::read(&mut input, &mut buffer)?;
            if read == 0 {
                break;
            }
            std::io::Write::write_all(&mut output, &buffer[..read])?;
            copied = copied.saturating_add(read as u64);
            if copied.saturating_sub(last_reported) >= 16 * 1024 * 1024 || copied == total {
                last_reported = copied;
                on_progress(copied, total);
            }
        }
        output.sync_all()?;
        if should_cancel() {
            return Ok(false);
        }
        std::fs::rename(&temporary, destination)?;
        File::open(parent)?.sync_all()?;
        Ok(true)
    })();
    if !matches!(result, Ok(true)) {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| internal_error())
}

fn internal_error() -> ApiError {
    ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    }
}

#[tauri::command]
async fn list_official_images(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/images/official", Option::<Value>::None).await
}

#[tauri::command]
async fn delete_official_image(app: AppHandle, reference: String) -> Result<(), ApiError> {
    request::<Value>(
        &app,
        "DELETE",
        "/v1/images/official",
        Some(serde_json::json!({ "reference": reference })),
    )
    .await?;
    let _ = app.emit("resource-changed", serde_json::json!({ "kind": "image" }));
    Ok(())
}

#[tauri::command]
async fn pull_official_image(
    app: AppHandle,
    pull_id: String,
    reference: String,
    on_progress: Channel<ImagePullProgress>,
) -> Result<PullImageResult, ApiError> {
    let pull_id = uuid::Uuid::parse_str(&pull_id).map_err(|_| invalid_request())?;
    let paths = AppPaths::resolve(&app)?;
    let client = ControllerClient::new(paths.controller_socket);
    let mut terminal_phase = None;
    client
        .request_ndjson(
            "POST",
            "/v1/images/pulls",
            Some(serde_json::json!({ "pullId": pull_id, "reference": reference })),
            |event: ImagePullProgress| {
                if matches!(
                    event.phase,
                    ImagePullPhase::Complete | ImagePullPhase::Cancelled | ImagePullPhase::Error
                ) {
                    terminal_phase = Some(event.phase.clone());
                }
                on_progress
                    .send(event)
                    .map_err(|_| controller_unavailable())
            },
        )
        .await?;

    match terminal_phase {
        Some(ImagePullPhase::Complete) => {
            let _ = app.emit("resource-changed", serde_json::json!({ "kind": "image" }));
            Ok(PullImageResult { cancelled: false })
        }
        Some(ImagePullPhase::Cancelled) => Ok(PullImageResult { cancelled: true }),
        Some(ImagePullPhase::Error) => Err(ApiError {
            code: ErrorCode::DockerUnavailable,
            params: BTreeMap::new(),
        }),
        _ => Err(controller_unavailable()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PullImageResult {
    cancelled: bool,
}

#[tauri::command]
async fn cancel_official_image_pull(app: AppHandle, pull_id: String) -> Result<(), ApiError> {
    let pull_id = uuid::Uuid::parse_str(&pull_id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "DELETE",
        &format!("/v1/images/pulls/{pull_id}"),
        Option::<Value>::None,
    )
    .await?;
    Ok(())
}

#[tauri::command]
async fn prune_image_cache(app: AppHandle) -> Result<Value, ApiError> {
    let response =
        request::<Value>(&app, "DELETE", "/v1/images/cache", Option::<Value>::None).await?;
    let _ = app.emit("resource-changed", serde_json::json!({ "kind": "image" }));
    Ok(response)
}

#[tauri::command]
async fn get_server_settings(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/settings/server", Option::<Value>::None).await
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateServerSettingsInput {
    settings: ServerSettings,
}

#[tauri::command]
async fn save_server_settings(
    app: AppHandle,
    request_body: UpdateServerSettingsInput,
) -> Result<Value, ApiError> {
    let response = request(&app, "PUT", "/v1/settings/server", Some(request_body)).await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "server-settings" }),
    );
    Ok(response)
}

#[tauri::command]
async fn regenerate_server_token(app: AppHandle) -> Result<Value, ApiError> {
    let response = request::<Value>(
        &app,
        "POST",
        "/v1/settings/server/token/regenerate",
        Option::<Value>::None,
    )
    .await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "server-settings" }),
    );
    Ok(response)
}

#[tauri::command]
async fn get_frps_setup_guide(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/frps/setup", Option::<Value>::None).await
}

#[tauri::command]
async fn get_frpc_status(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "GET", "/v1/frpc", Option::<Value>::None).await
}

#[tauri::command]
async fn frpc_action(app: AppHandle, action: String) -> Result<Value, ApiError> {
    if !matches!(action.as_str(), "start" | "stop" | "restart") {
        return Err(invalid_request());
    }
    let response = request::<Value>(
        &app,
        "POST",
        &format!("/v1/frpc/{action}"),
        Option::<Value>::None,
    )
    .await?;
    let _ = app.emit("resource-changed", serde_json::json!({ "kind": "frpc" }));
    Ok(response)
}

#[tauri::command]
async fn test_frpc_connectivity(app: AppHandle) -> Result<Value, ApiError> {
    request::<Value>(&app, "POST", "/v1/frpc/test", Option::<Value>::None).await
}

#[tauri::command]
async fn create_environment(app: AppHandle, spec: EnvironmentSpec) -> Result<Value, ApiError> {
    let response = request(&app, "POST", "/v1/environments", Some(spec)).await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "environment" }),
    );
    Ok(response)
}

#[tauri::command]
async fn environment_action(app: AppHandle, id: String, action: String) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    if !matches!(action.as_str(), "start" | "stop" | "restart") {
        return Err(invalid_request());
    }
    let path = format!("/v1/environments/{id}/{action}");
    request::<Value>(&app, "POST", &path, Option::<Value>::None).await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "environment", "id": id }),
    );
    Ok(())
}

#[tauri::command]
async fn environment_publication_action(
    app: AppHandle,
    id: String,
    action: String,
) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    if !matches!(action.as_str(), "publish" | "unpublish") {
        return Err(invalid_request());
    }
    let response = request::<Value>(
        &app,
        "POST",
        &format!("/v1/environments/{id}/{action}"),
        Option::<Value>::None,
    )
    .await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "environment", "id": id }),
    );
    Ok(response)
}

#[tauri::command]
async fn get_environment_credentials(app: AppHandle, id: String) -> Result<Value, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    request::<Value>(
        &app,
        "GET",
        &format!("/v1/environments/{id}/credentials"),
        Option::<Value>::None,
    )
    .await
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteEnvironmentInput {
    confirmation_name: String,
    delete_data: bool,
}

#[tauri::command]
async fn delete_environment(
    app: AppHandle,
    id: String,
    request_body: DeleteEnvironmentInput,
) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    let paths = AppPaths::resolve(&app)?;
    let client = ControllerClient::new(paths.controller_socket);
    client
        .request::<_, Value>(
            "DELETE",
            &format!("/v1/environments/{id}"),
            Some(request_body),
        )
        .await?;
    let _ = app.emit(
        "resource-changed",
        serde_json::json!({ "kind": "environment", "id": id }),
    );
    Ok(())
}

#[tauri::command]
fn open_local_environment(local_port: u16) -> Result<(), ApiError> {
    if local_port == 0 {
        return Err(invalid_request());
    }
    if !claim_local_desktop_open(local_port) {
        return Ok(());
    }
    let url = format!("https://127.0.0.1:{local_port}/");
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ApiError {
            code: ErrorCode::Internal,
            params: BTreeMap::new(),
        })?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentDirectoryRecord {
    id: uuid::Uuid,
    container_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicEnvironmentRecord {
    id: uuid::Uuid,
    desired_running: bool,
    spec: PublicEnvironmentSpec,
}

#[derive(Deserialize)]
struct PublicEnvironmentSpec {
    publication: PublicEnvironmentPublication,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicEnvironmentPublication {
    enabled: bool,
    remote_port: Option<u16>,
}

#[tauri::command]
async fn open_environment_data_directory(app: AppHandle, id: String) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    let records = request::<Vec<EnvironmentDirectoryRecord>>(
        &app,
        "GET",
        "/v1/environments",
        Option::<Value>::None,
    )
    .await?;
    let record = records
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(invalid_request)?;
    let paths = AppPaths::resolve(&app)?;
    let expected = paths.environment_root.join(id.to_string()).join("config");
    let legacy = PathBuf::from("/data/environments")
        .join(id.to_string())
        .join("config");

    let docker = Docker::connect_with_socket_defaults().map_err(|_| controller_unavailable())?;
    let inspected = docker
        .inspect_container(&record.container_id, None)
        .await
        .map_err(|_| controller_unavailable())?;
    let mounted_source = inspected
        .mounts
        .unwrap_or_default()
        .into_iter()
        .find(|mount| mount.destination.as_deref() == Some("/config"))
        .and_then(|mount| mount.source)
        .map(PathBuf::from)
        .ok_or_else(invalid_request)?;
    if mounted_source != expected && mounted_source != legacy {
        return Err(invalid_request());
    }
    let allowed_root = if mounted_source == expected {
        paths.environment_root
    } else {
        PathBuf::from("/data/environments")
    };
    let canonical_root = std::fs::canonicalize(allowed_root).map_err(|_| ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    })?;
    let directory = std::fs::canonicalize(mounted_source).map_err(|_| ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    })?;
    webtop_contracts::require_contained_path(&canonical_root, &directory)?;
    std::process::Command::new("xdg-open")
        .arg(directory)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ApiError {
            code: ErrorCode::Internal,
            params: BTreeMap::new(),
        })?;
    Ok(())
}

#[tauri::command]
async fn open_public_environment(app: AppHandle, id: String) -> Result<(), ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    let records = request::<Vec<PublicEnvironmentRecord>>(
        &app,
        "GET",
        "/v1/environments",
        Option::<Value>::None,
    )
    .await?;
    let record = records
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(invalid_request)?;
    if !record.desired_running || !record.spec.publication.enabled {
        return Err(invalid_request());
    }
    let remote_port = record
        .spec
        .publication
        .remote_port
        .ok_or_else(invalid_request)?;
    let settings =
        request::<ServerSettings>(&app, "GET", "/v1/settings/server", Option::<Value>::None)
            .await?;
    settings.validate().map_err(|_| invalid_request())?;
    let public_address = settings.public_ip.trim().trim_matches(['[', ']']);
    let url_host = if public_address.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{public_address}]")
    } else {
        public_address.to_owned()
    };
    let mut url =
        url::Url::parse(&format!("https://{url_host}/")).map_err(|_| invalid_request())?;
    url.set_port(Some(remote_port))
        .map_err(|_| invalid_request())?;
    std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ApiError {
            code: ErrorCode::Internal,
            params: BTreeMap::new(),
        })?;
    Ok(())
}

async fn request<R: DeserializeOwned>(
    app: &AppHandle,
    method: &str,
    path: &str,
    body: Option<impl serde::Serialize>,
) -> Result<R, ApiError> {
    let paths = AppPaths::resolve(app)?;
    ControllerClient::new(paths.controller_socket)
        .request(method, path, body)
        .await
}

fn invalid_request() -> ApiError {
    ApiError {
        code: ErrorCode::InvalidRequest,
        params: BTreeMap::new(),
    }
}

fn controller_unavailable() -> ApiError {
    ApiError {
        code: ErrorCode::ControllerUnavailable,
        params: BTreeMap::new(),
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            docker_diagnostics,
            bootstrap_controller,
            list_environments,
            list_templates,
            template_preflight,
            create_template,
            create_environment_from_template,
            check_template_sources,
            export_template,
            get_operation,
            cancel_operation,
            import_template_preflight,
            import_template,
            delete_template,
            select_template_import,
            stage_template_import,
            select_template_export,
            save_template_export,
            cancel_template_transfer,
            discard_template_export_destination,
            discard_template_staging,
            list_official_images,
            delete_official_image,
            pull_official_image,
            cancel_official_image_pull,
            prune_image_cache,
            get_server_settings,
            save_server_settings,
            regenerate_server_token,
            get_frps_setup_guide,
            get_frpc_status,
            frpc_action,
            test_frpc_connectivity,
            create_environment,
            environment_action,
            environment_publication_action,
            get_environment_credentials,
            delete_environment,
            open_local_environment,
            open_environment_data_directory,
            open_public_environment,
        ])
        .manage(TemplateExportDestinations::default())
        .manage(TemplateImportSources::default())
        .manage(TemplateTransferCancellations::default())
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window exists");
            window.set_title("Webtop Manager")?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Webtop Manager");
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn duplicate_local_desktop_open_is_suppressed() {
        assert!(claim_local_desktop_open(65_431));
        assert!(!claim_local_desktop_open(65_431));
        assert!(claim_local_desktop_open(65_432));
    }

    #[test]
    fn atomic_template_copy_reports_progress_and_writes_mode_0600() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.wtmpl");
        let destination = directory.path().join("destination.wtmpl");
        let contents = vec![0x5a; 2 * 1024 * 1024];
        std::fs::write(&source, &contents).unwrap();
        let mut progress = Vec::new();

        copy_atomic_0600(
            &source,
            &destination,
            |current, total| {
                progress.push((current, total));
            },
            || false,
        )
        .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), contents);
        assert_eq!(progress.last(), Some(&(2 * 1024 * 1024, 2 * 1024 * 1024)));
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn cancelled_template_copy_removes_partial_destination() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.wtmpl");
        let destination = directory.path().join("destination.wtmpl");
        std::fs::write(&source, vec![0x5a; 2 * 1024 * 1024]).unwrap();

        let copied =
            copy_atomic_0600(&source, &destination, |_current, _total| {}, || true).unwrap();

        assert!(!copied);
        assert!(!destination.exists());
        assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("partial")
        }));
    }
}
