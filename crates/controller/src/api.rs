use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use bollard::Docker;
use bytes::Bytes;
use chrono::Utc;
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Mutex};
use tower_http::trace::TraceLayer;
use uuid::Uuid;
use webtop_contracts::{
    allocate_remote_port, ApiError, EnvironmentSpec, ErrorCode, FrpcServiceState,
    FrpcServiceStatus, FrpcTestResult, ImageCachePruneResult, ImagePullPhase, ImagePullProgress,
    OfficialImage, Operation, OperationKind, OperationPhase, ServerSettings, ServerTokenState,
    API_VERSION,
};

use crate::database::{Database, EnvironmentRecord};
use crate::docker;

#[derive(Clone)]
pub struct AppState {
    pub database: Database,
    pub docker: Docker,
    pub environment_root: PathBuf,
    pub host_environment_root: PathBuf,
    pub snapshot_root: PathBuf,
    pub staging_root: PathBuf,
    pub server_token_path: PathBuf,
    pub frpc_container_name: String,
    pub pull_cancellations: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
    pub operation_cancellations: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
    pub active_resources: Arc<Mutex<HashSet<String>>>,
    pub publication_lock: Arc<Mutex<()>>,
    pub token_lock: Arc<Mutex<()>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .merge(crate::templates::router())
        .route(
            "/v1/environments",
            get(list_environments).post(create_environment),
        )
        .route(
            "/v1/images/official",
            get(list_official_images).delete(delete_official_image),
        )
        .route("/v1/images/cache", delete(prune_image_cache))
        .route("/v1/images/pulls", post(pull_official_image))
        .route("/v1/images/pulls/{id}", delete(cancel_official_image_pull))
        .route(
            "/v1/settings/server",
            get(get_server_settings).put(update_server_settings),
        )
        .route(
            "/v1/settings/server/token/recover",
            post(recover_server_token),
        )
        .route("/v1/frps/setup", get(get_frps_setup_guide))
        .route("/v1/frpc", get(get_frpc_status))
        .route("/v1/frpc/start", post(start_frpc))
        .route("/v1/frpc/stop", post(stop_frpc))
        .route("/v1/frpc/restart", post(restart_frpc))
        .route("/v1/frpc/test", post(test_frpc_connectivity))
        .route("/v1/environments/{id}/start", post(start_environment))
        .route("/v1/environments/{id}/stop", post(stop_environment))
        .route("/v1/environments/{id}/restart", post(restart_environment))
        .route(
            "/v1/environments/{id}/credentials",
            get(get_environment_credentials),
        )
        .route("/v1/environments/{id}/publish", post(publish_environment))
        .route(
            "/v1/environments/{id}/unpublish",
            post(unpublish_environment),
        )
        .route("/v1/environments/{id}", delete(delete_environment))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    api_version: &'static str,
    controller_version: &'static str,
    capabilities: [&'static str; 16],
    docker_version: Option<String>,
}

async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiFailure> {
    let version = state.docker.version().await.map_err(ApiFailure::docker)?;
    Ok(Json(HealthResponse {
        api_version: API_VERSION,
        controller_version: env!("CARGO_PKG_VERSION"),
        capabilities: [
            "frpc_lifecycle_v1",
            "frp_token_recovery_v1",
            "frps_guide_v2",
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
        ],
        docker_version: version.version,
    }))
}

async fn list_environments(
    State(state): State<AppState>,
) -> Result<Json<Vec<EnvironmentRecord>>, ApiFailure> {
    let mut environments = state
        .database
        .list_environments()
        .await
        .map_err(ApiFailure::internal)?;
    match synchronize_environment_local_ports(&state, &mut environments).await {
        Ok(true) => reconcile_frpc_if_desired(&state).await,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(error = %error, "failed to synchronize environment ports while listing")
        }
    }
    Ok(Json(environments))
}

async fn list_official_images(
    State(state): State<AppState>,
) -> Result<Json<Vec<OfficialImage>>, ApiFailure> {
    Ok(Json(
        docker::list_official_images(&state.docker)
            .await
            .map_err(ApiFailure::docker)?,
    ))
}

async fn prune_image_cache(
    State(state): State<AppState>,
) -> Result<Json<ImageCachePruneResult>, ApiFailure> {
    {
        let mut resources = state.active_resources.lock().await;
        if resources.contains("docker-images") {
            return Err(ApiFailure::resource_busy());
        }
        resources.insert("docker-images".into());
    }
    let pull_cancellations = state.pull_cancellations.lock().await;
    if !pull_cancellations.is_empty() {
        state.active_resources.lock().await.remove("docker-images");
        return Err(ApiFailure::resource_busy());
    }
    let result = docker::prune_image_cache(&state.docker).await;
    drop(pull_cancellations);
    state.active_resources.lock().await.remove("docker-images");
    Ok(Json(result.map_err(ApiFailure::docker)?))
}

#[derive(Deserialize)]
struct DeleteOfficialImageRequest {
    reference: String,
}

async fn delete_official_image(
    State(state): State<AppState>,
    Json(request): Json<DeleteOfficialImageRequest>,
) -> Result<StatusCode, ApiFailure> {
    if !docker::is_official_image(&request.reference) {
        return Err(ApiFailure::invalid_request());
    }
    {
        let mut resources = state.active_resources.lock().await;
        if resources.contains("docker-images") {
            return Err(ApiFailure::resource_busy());
        }
        resources.insert("docker-images".into());
    }
    let pull_cancellations = state.pull_cancellations.lock().await;
    if !pull_cancellations.is_empty() {
        state.active_resources.lock().await.remove("docker-images");
        return Err(ApiFailure::resource_busy());
    }
    let in_use = docker::official_image_in_use(&state.docker, &request.reference).await;
    if in_use.as_ref().is_ok_and(|value| *value) {
        state.active_resources.lock().await.remove("docker-images");
        return Err(ApiFailure::resource_busy());
    }
    if let Err(error) = in_use {
        state.active_resources.lock().await.remove("docker-images");
        return Err(ApiFailure::docker(error));
    }
    let result = docker::remove_official_image(&state.docker, &request.reference).await;
    drop(pull_cancellations);
    state.active_resources.lock().await.remove("docker-images");
    result.map_err(ApiFailure::docker)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullImageRequest {
    pull_id: Uuid,
    reference: String,
}

async fn pull_official_image(
    State(state): State<AppState>,
    Json(request): Json<PullImageRequest>,
) -> Result<Response, ApiFailure> {
    if !docker::is_official_image(&request.reference) {
        return Err(ApiFailure::invalid_request());
    }
    if let Some(existing) = state
        .database
        .get_operation(request.pull_id)
        .await
        .map_err(ApiFailure::internal)?
    {
        let stored = state
            .database
            .get_operation_request::<PullImageRequest>(request.pull_id)
            .await
            .map_err(ApiFailure::internal)?;
        if existing.kind != OperationKind::PullImage
            || stored.as_ref().map(|value| value.reference.as_str())
                != Some(request.reference.as_str())
        {
            return Err(ApiFailure::invalid_request());
        }
        return Ok(reattach_image_pull_stream(
            state.database.clone(),
            request.pull_id,
            request.reference,
        ));
    }
    {
        let mut resources = state.active_resources.lock().await;
        if resources.contains("docker-images") {
            return Err(ApiFailure::resource_busy());
        }
        resources.insert("docker-images".into());
    }
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let operation_cancel_tx = cancel_tx.clone();
    {
        let mut cancellations = state.pull_cancellations.lock().await;
        if cancellations.contains_key(&request.pull_id) {
            state.active_resources.lock().await.remove("docker-images");
            return Err(ApiFailure::invalid_request());
        }
        cancellations.insert(request.pull_id, cancel_tx);
    }
    state
        .operation_cancellations
        .lock()
        .await
        .insert(request.pull_id, operation_cancel_tx);
    let operation = new_image_pull_operation(&request);
    if let Err(error) = state
        .database
        .insert_operation_with_request(&operation, &request)
        .await
    {
        state
            .pull_cancellations
            .lock()
            .await
            .remove(&request.pull_id);
        state
            .operation_cancellations
            .lock()
            .await
            .remove(&request.pull_id);
        state.active_resources.lock().await.remove("docker-images");
        return Err(ApiFailure::internal(error));
    }
    let (response_tx, event_rx) = mpsc::unbounded_channel::<ImagePullProgress>();
    spawn_image_pull_job(state, request, cancel_rx, Some(response_tx));

    let stream = futures_util::stream::unfold(event_rx, |mut receiver| async move {
        receiver.recv().await.map(|event| {
            let mut line = serde_json::to_vec(&event).expect("serialize image pull progress");
            line.push(b'\n');
            (Ok::<Bytes, Infallible>(Bytes::from(line)), receiver)
        })
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream(stream))
        .map_err(ApiFailure::internal)
}

fn new_image_pull_operation(request: &PullImageRequest) -> Operation {
    let now = Utc::now();
    Operation {
        id: request.pull_id,
        kind: OperationKind::PullImage,
        phase: OperationPhase::Queued,
        progress_percent: Some(0),
        cancellable: true,
        resource_id: None,
        error: None,
        result: None,
        log_lines: vec![
            "$ webtop-manager image pull".into(),
            "[controller] queued durable image pull".into(),
        ],
        created_at: now,
        updated_at: now,
    }
}

fn spawn_image_pull_job(
    state: AppState,
    request: PullImageRequest,
    cancellation: watch::Receiver<bool>,
    response: Option<mpsc::UnboundedSender<ImagePullProgress>>,
) {
    tokio::spawn(async move {
        let pull_id = request.pull_id;
        let reference = request.reference.clone();
        let _ = state
            .database
            .append_operation_log(pull_id, "[docker] pulling allowlisted image")
            .await;
        let _ = state
            .database
            .update_operation(pull_id, OperationPhase::Running, Some(5), None, None)
            .await;
        let (event_tx, mut event_rx) = mpsc::channel::<ImagePullProgress>(32);
        let progress_database = state.database.clone();
        let progress = tokio::spawn(async move {
            let mut last_percent = 5_u8;
            while let Some(event) = event_rx.recv().await {
                if let (Some(current), Some(total)) =
                    (event.aggregate_current_bytes, event.aggregate_total_bytes)
                {
                    if total > 0 {
                        let percent =
                            (5_i64 + current.saturating_mul(85) / total).clamp(5, 90) as u8;
                        if percent >= last_percent.saturating_add(2) {
                            last_percent = percent;
                            let _ = progress_database
                                .update_operation(
                                    pull_id,
                                    OperationPhase::Running,
                                    Some(percent),
                                    None,
                                    None,
                                )
                                .await;
                        }
                    }
                }
                if let Some(response) = &response {
                    let _ = response.send(event);
                }
            }
        });
        let result =
            docker::pull_official_image(&state.docker, pull_id, &reference, cancellation, event_tx)
                .await;
        let _ = progress.await;
        match result {
            Ok(docker::ImagePullOutcome::Complete) => {
                let result = serde_json::json!({"reference":reference});
                let _ = state
                    .database
                    .append_operation_log(pull_id, "[controller] image pull completed")
                    .await;
                let _ = state
                    .database
                    .update_operation(
                        pull_id,
                        OperationPhase::Succeeded,
                        Some(100),
                        None,
                        Some(&result),
                    )
                    .await;
            }
            Ok(docker::ImagePullOutcome::Cancelled) => {
                let _ = state
                    .database
                    .append_operation_log(pull_id, "[controller] image pull cancelled")
                    .await;
                let _ = state
                    .database
                    .update_operation(pull_id, OperationPhase::Cancelled, None, None, None)
                    .await;
            }
            Err(error) => {
                tracing::warn!(%error, %pull_id, %reference, "Docker image pull failed");
                let api_error = ApiError {
                    code: ErrorCode::DockerUnavailable,
                    params: BTreeMap::new(),
                };
                let _ = state
                    .database
                    .append_operation_log(pull_id, "[controller] image pull failed")
                    .await;
                let _ = state
                    .database
                    .update_operation(
                        pull_id,
                        OperationPhase::Failed,
                        None,
                        Some(&api_error),
                        None,
                    )
                    .await;
            }
        }
        state.pull_cancellations.lock().await.remove(&pull_id);
        state.operation_cancellations.lock().await.remove(&pull_id);
        state.active_resources.lock().await.remove("docker-images");
    });
}

fn reattach_image_pull_stream(database: Database, pull_id: Uuid, reference: String) -> Response {
    let stream = futures_util::stream::unfold(
        (database, pull_id, reference, false),
        |(database, pull_id, reference, finished)| async move {
            if finished {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let operation = database.get_operation(pull_id).await.ok().flatten();
            let (phase, status, finished) = match operation.as_ref().map(|value| &value.phase) {
                Some(OperationPhase::Succeeded) => {
                    (ImagePullPhase::Complete, "Image pull complete", true)
                }
                Some(OperationPhase::Cancelled) => {
                    (ImagePullPhase::Cancelled, "Image pull cancelled", true)
                }
                Some(OperationPhase::Failed | OperationPhase::Retryable) | None => {
                    (ImagePullPhase::Error, "Image pull failed", true)
                }
                _ => (
                    ImagePullPhase::Progress,
                    "Image pull running in controller",
                    false,
                ),
            };
            let event = ImagePullProgress {
                pull_id,
                reference: reference.clone(),
                phase,
                layer_id: None,
                status: status.into(),
                current_bytes: operation
                    .as_ref()
                    .and_then(|value| value.progress_percent)
                    .map(i64::from),
                total_bytes: Some(100),
                aggregate_current_bytes: operation
                    .as_ref()
                    .and_then(|value| value.progress_percent)
                    .map(i64::from),
                aggregate_total_bytes: Some(100),
            };
            let mut line = serde_json::to_vec(&event).expect("serialize image pull progress");
            line.push(b'\n');
            Some((
                Ok::<Bytes, Infallible>(Bytes::from(line)),
                (database, pull_id, reference, finished),
            ))
        },
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream(stream))
        .expect("valid image pull response")
}

pub async fn resume_interrupted_operations(state: &AppState, operations: Vec<Operation>) {
    for operation in operations {
        let request = state
            .database
            .get_operation_request::<PullImageRequest>(operation.id)
            .await;
        let Ok(Some(request)) = request else {
            let _ = state
                .database
                .update_operation(
                    operation.id,
                    OperationPhase::Retryable,
                    operation.progress_percent,
                    None,
                    None,
                )
                .await;
            continue;
        };
        let mut resources = state.active_resources.lock().await;
        if resources.contains("docker-images") {
            drop(resources);
            let _ = state
                .database
                .update_operation(
                    operation.id,
                    OperationPhase::Retryable,
                    operation.progress_percent,
                    None,
                    None,
                )
                .await;
            continue;
        }
        resources.insert("docker-images".into());
        drop(resources);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        state
            .pull_cancellations
            .lock()
            .await
            .insert(operation.id, cancel_tx.clone());
        state
            .operation_cancellations
            .lock()
            .await
            .insert(operation.id, cancel_tx);
        let _ = state
            .database
            .append_operation_log(
                operation.id,
                "[controller] reattached interrupted image pull",
            )
            .await;
        spawn_image_pull_job(state.clone(), request, cancel_rx, None);
    }
}

async fn cancel_official_image_pull(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiFailure> {
    let operation = state
        .database
        .get_operation(id)
        .await
        .map_err(ApiFailure::internal)?;
    if operation.as_ref().is_some_and(|operation| {
        matches!(
            operation.phase,
            OperationPhase::Succeeded
                | OperationPhase::Failed
                | OperationPhase::Cancelled
                | OperationPhase::Retryable
        )
    }) {
        return Ok(StatusCode::NO_CONTENT);
    }
    if let Some(cancel) = state.pull_cancellations.lock().await.get(&id).cloned() {
        let _ = cancel.send(true);
        if let Some(operation) = operation {
            state
                .database
                .append_operation_log(id, "[controller] image pull cancellation requested")
                .await
                .map_err(ApiFailure::internal)?;
            state
                .database
                .update_operation(
                    id,
                    OperationPhase::RollingBack,
                    operation.progress_percent,
                    None,
                    None,
                )
                .await
                .map_err(ApiFailure::internal)?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_server_settings(
    State(state): State<AppState>,
) -> Result<Json<ServerSettings>, ApiFailure> {
    let token = inspect_server_token(&state)
        .await
        .map_err(ApiFailure::internal)?;
    if token.state == ServerTokenState::Missing {
        suspend_frpc_for_missing_token(&state).await;
    }
    let settings = state
        .database
        .get_server_settings()
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(apply_server_token_state(settings, &token)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateServerSettingsRequest {
    settings: ServerSettings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrpsSetupGuideResponse {
    docker_setup_script: String,
    native_setup_script: String,
    public_address: String,
    bind_port: u16,
    remote_port_start: u16,
    remote_port_end: u16,
}

async fn update_server_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateServerSettingsRequest>,
) -> Result<Json<ServerSettings>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let mut settings = request.settings;
    settings.token_configured = false;
    settings
        .validate()
        .map_err(|_| ApiFailure::invalid_request())?;

    let token = inspect_server_token(&state)
        .await
        .map_err(ApiFailure::internal)?;
    settings = apply_server_token_state(settings, &token);
    state
        .database
        .save_server_settings(&settings)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(settings))
}

async fn recover_server_token(
    State(state): State<AppState>,
) -> Result<Json<ServerSettings>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let _token_guard = state.token_lock.lock().await;
    let current = inspect_or_initialize_server_token(&state.database, &state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    let recovered = match current.state.clone() {
        ServerTokenState::Ready => return Err(ApiFailure::invalid_request()),
        ServerTokenState::RecoveryPending => current,
        ServerTokenState::Missing => {
            docker::remove_frpc(&state.docker, &state.frpc_container_name)
                .await
                .map_err(ApiFailure::docker)?;
            state
                .database
                .set_frpc_desired_running(false)
                .await
                .map_err(ApiFailure::internal)?;
            let token = generate_server_token().into_bytes();
            replace_secret(&state.server_token_path, &token)
                .await
                .map_err(ApiFailure::internal)?;
            let fingerprint = token_fingerprint(&token);
            state
                .database
                .save_frp_token_metadata(&fingerprint, true)
                .await
                .map_err(ApiFailure::internal)?;
            ServerTokenMaterial {
                state: ServerTokenState::RecoveryPending,
                token: Some(token),
                fingerprint: Some(fingerprint),
            }
        }
    };
    let settings = state
        .database
        .get_server_settings()
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(apply_server_token_state(settings, &recovered)))
}

async fn get_frps_setup_guide(
    State(state): State<AppState>,
) -> Result<Json<FrpsSetupGuideResponse>, ApiFailure> {
    let settings = required_server_settings(&state).await?;
    let token = required_server_token(&state, true).await?;
    let token = std::str::from_utf8(&token).map_err(ApiFailure::internal)?;
    Ok(Json(FrpsSetupGuideResponse {
        docker_setup_script: crate::frp::render_frps_docker_setup_script(&settings, token),
        native_setup_script: crate::frp::render_frps_native_setup_script(&settings, token),
        public_address: settings.public_ip,
        bind_port: settings.frps_port,
        remote_port_start: settings.remote_port_start,
        remote_port_end: settings.remote_port_end,
    }))
}

async fn get_frpc_status(
    State(state): State<AppState>,
) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let status = docker::frpc_status(&state.docker, &state.frpc_container_name)
        .await
        .map_err(ApiFailure::docker)?;
    Ok(Json(apply_frpc_desired_state(&state, status).await?))
}

async fn start_frpc(State(state): State<AppState>) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let status = start_frpc_with_port_retry(&state).await?;
    state
        .database
        .set_frpc_desired_running(true)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(status))
}

async fn stop_frpc(State(state): State<AppState>) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let status = docker::stop_frpc(&state.docker, &state.frpc_container_name)
        .await
        .map_err(ApiFailure::docker)?;
    state
        .database
        .set_frpc_desired_running(false)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(mark_frpc_stopped(status)))
}

async fn restart_frpc(
    State(state): State<AppState>,
) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let status = start_frpc_with_port_retry(&state).await?;
    state
        .database
        .set_frpc_desired_running(true)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(status))
}

async fn test_frpc_connectivity(
    State(state): State<AppState>,
) -> Result<Json<FrpcTestResult>, ApiFailure> {
    let settings = required_server_settings(&state).await?;
    let material = inspect_server_token(&state)
        .await
        .map_err(ApiFailure::internal)?;
    let token = material
        .token
        .as_deref()
        .ok_or_else(ApiFailure::frp_token_recovery_required)?;
    let result = docker::test_frpc_connectivity(&state.docker, &settings, token)
        .await
        .map_err(ApiFailure::docker)?;
    if result.success && material.state == ServerTokenState::RecoveryPending {
        let fingerprint = material
            .fingerprint
            .as_deref()
            .ok_or_else(|| ApiFailure::internal("recovery token fingerprint missing"))?;
        state
            .database
            .complete_frp_token_recovery(fingerprint)
            .await
            .map_err(ApiFailure::internal)?;
    }
    Ok(Json(result))
}

async fn required_server_settings(state: &AppState) -> Result<ServerSettings, ApiFailure> {
    let settings = state
        .database
        .get_server_settings()
        .await
        .map_err(ApiFailure::internal)?;
    settings
        .validate()
        .map_err(|_| ApiFailure::invalid_request())?;
    Ok(settings)
}

async fn apply_frpc_desired_state(
    state: &AppState,
    status: FrpcServiceStatus,
) -> Result<FrpcServiceStatus, ApiFailure> {
    let desired_running = state
        .database
        .frpc_desired_running()
        .await
        .map_err(ApiFailure::internal)?;
    Ok(if desired_running {
        status
    } else {
        mark_frpc_stopped(status)
    })
}

fn mark_frpc_stopped(mut status: FrpcServiceStatus) -> FrpcServiceStatus {
    if !matches!(status.state, FrpcServiceState::NotCreated) {
        status.state = FrpcServiceState::Stopped;
        status.connected = false;
    }
    status
}

async fn create_environment(
    State(state): State<AppState>,
    Json(mut spec): Json<EnvironmentSpec>,
) -> Result<(StatusCode, Json<EnvironmentRecord>), ApiFailure> {
    spec.validate().map_err(|_| ApiFailure::invalid_request())?;
    if !docker::is_official_image(&spec.image) {
        return Err(ApiFailure::invalid_request());
    }
    if state
        .database
        .environment_name_exists(&spec.name)
        .await
        .map_err(ApiFailure::internal)?
    {
        return Err(ApiFailure::invalid_request());
    }

    if spec.publication.enabled {
        let settings = required_server_settings(&state).await?;
        let environments = state
            .database
            .list_environments()
            .await
            .map_err(ApiFailure::internal)?;
        let allocated = environments
            .iter()
            .filter_map(|record| record.spec.publication.remote_port)
            .collect::<BTreeSet<_>>();
        let requested = spec.publication.remote_port;
        let remote_port = if spec.publication.automatic_port || requested.is_none() {
            allocate_remote_port(
                settings.remote_port_start,
                settings.remote_port_end,
                &allocated,
            )
            .map_err(ApiFailure::port_conflict)?
        } else {
            let port = requested.expect("manual publication port was validated");
            if port < settings.remote_port_start
                || port > settings.remote_port_end
                || allocated.contains(&port)
            {
                return Err(ApiFailure::port_conflict(ApiError {
                    code: ErrorCode::PortConflict,
                    params: BTreeMap::new(),
                }));
            }
            port
        };
        spec.publication.remote_port = Some(remote_port);
    }

    let id = Uuid::new_v4();
    let resource_root = state.environment_root.join(id.to_string());
    let config_path = resource_root.join("config");
    let secret_path = resource_root.join("secrets/password");
    let host_resource_root = state.host_environment_root.join(id.to_string());
    let host_config_path = host_resource_root.join("config");
    let host_secret_path = host_resource_root.join("secrets/password");
    tokio::fs::create_dir_all(&config_path)
        .await
        .map_err(ApiFailure::internal)?;
    tokio::fs::create_dir_all(secret_path.parent().expect("password has a parent"))
        .await
        .map_err(ApiFailure::internal)?;
    let password = Alphanumeric.sample_string(&mut rand::rng(), 40);
    write_secret(&secret_path, password.as_bytes())
        .await
        .map_err(ApiFailure::internal)?;

    let created = match docker::create_environment_container(
        &state.docker,
        &id.to_string(),
        &spec,
        &host_config_path.display().to_string(),
        &host_secret_path.display().to_string(),
    )
    .await
    {
        Ok(created) => created,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&resource_root).await;
            return Err(ApiFailure::docker(error));
        }
    };
    let record = EnvironmentRecord {
        id,
        name: spec.name.clone(),
        container_id: created.id.clone(),
        config_path: config_path.display().to_string(),
        desired_running: true,
        local_port: created.local_port,
        template_id: None,
        spec,
        created_at: Utc::now(),
    };
    if let Err(error) = state.database.insert_environment(&record).await {
        let _ = docker::stop(&state.docker, &created.id).await;
        let _ = docker::remove(&state.docker, &created.id).await;
        let _ = tokio::fs::remove_dir_all(&resource_root).await;
        return Err(ApiFailure::internal(error));
    }
    if record.spec.publication.enabled {
        reconcile_frpc_if_desired(&state).await;
    }
    Ok((StatusCode::CREATED, Json(record)))
}

async fn start_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiFailure> {
    ensure_environment_not_busy(&state, id).await?;
    let mut record = required_environment(&state, id).await?;
    docker::start(&state.docker, &record.container_id)
        .await
        .map_err(ApiFailure::docker)?;
    state
        .database
        .set_desired_running(id, true)
        .await
        .map_err(ApiFailure::internal)?;
    record.desired_running = true;
    synchronize_environment_local_port(&state, &mut record)
        .await
        .map_err(ApiFailure::internal)?;
    if record.spec.publication.enabled {
        reconcile_frpc_if_desired(&state).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiFailure> {
    ensure_environment_not_busy(&state, id).await?;
    let record = required_environment(&state, id).await?;
    docker::stop(&state.docker, &record.container_id)
        .await
        .map_err(ApiFailure::docker)?;
    state
        .database
        .set_desired_running(id, false)
        .await
        .map_err(ApiFailure::internal)?;
    if record.spec.publication.enabled {
        reconcile_frpc_if_desired(&state).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn restart_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiFailure> {
    ensure_environment_not_busy(&state, id).await?;
    let mut record = required_environment(&state, id).await?;
    docker::restart(&state.docker, &record.container_id)
        .await
        .map_err(ApiFailure::docker)?;
    synchronize_environment_local_port(&state, &mut record)
        .await
        .map_err(ApiFailure::internal)?;
    if record.spec.publication.enabled {
        reconcile_frpc_if_desired(&state).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn publish_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnvironmentRecord>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let mut record = required_environment(&state, id).await?;
    if record.spec.publication.enabled && record.spec.publication.remote_port.is_some() {
        return Ok(Json(record));
    }
    let settings = required_server_settings(&state).await?;
    let allocated = state
        .database
        .list_environments()
        .await
        .map_err(ApiFailure::internal)?
        .into_iter()
        .filter(|environment| environment.id != id)
        .filter_map(|environment| environment.spec.publication.remote_port)
        .collect::<BTreeSet<_>>();
    let remote_port = allocate_remote_port(
        settings.remote_port_start,
        settings.remote_port_end,
        &allocated,
    )
    .map_err(ApiFailure::port_conflict)?;
    record.spec.publication.enabled = true;
    record.spec.publication.automatic_port = true;
    record.spec.publication.remote_port = Some(remote_port);
    state
        .database
        .update_environment_spec(id, &record.spec)
        .await
        .map_err(ApiFailure::internal)?;
    if state
        .database
        .frpc_desired_running()
        .await
        .map_err(ApiFailure::internal)?
    {
        start_frpc_with_port_retry(&state).await?;
        record = required_environment(&state, id).await?;
    }
    Ok(Json(record))
}

async fn unpublish_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnvironmentRecord>, ApiFailure> {
    let _guard = state.publication_lock.lock().await;
    let mut record = required_environment(&state, id).await?;
    if !record.spec.publication.enabled {
        return Ok(Json(record));
    }
    record.spec.publication.enabled = false;
    record.spec.publication.automatic_port = true;
    record.spec.publication.remote_port = None;
    state
        .database
        .update_environment_spec(id, &record.spec)
        .await
        .map_err(ApiFailure::internal)?;
    reconcile_frpc_if_desired_locked(&state).await;
    Ok(Json(record))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentCredentials {
    username: String,
    password: String,
}

async fn get_environment_credentials(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnvironmentCredentials>, ApiFailure> {
    required_environment(&state, id).await?;
    let secret_path = state
        .environment_root
        .join(id.to_string())
        .join("secrets/password");
    let password = tokio::fs::read_to_string(secret_path)
        .await
        .map_err(ApiFailure::internal)?;
    if password.is_empty() {
        return Err(ApiFailure::internal("environment password is empty"));
    }
    Ok(Json(EnvironmentCredentials {
        username: format!("webtop-{id}"),
        password,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteRequest {
    confirmation_name: String,
    delete_data: bool,
}

async fn delete_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<DeleteRequest>,
) -> Result<StatusCode, ApiFailure> {
    ensure_environment_not_busy(&state, id).await?;
    let record = required_environment(&state, id).await?;
    if request.confirmation_name != record.name {
        return Err(ApiFailure::invalid_request());
    }
    if record.desired_running {
        docker::stop(&state.docker, &record.container_id)
            .await
            .map_err(ApiFailure::docker)?;
    }
    docker::remove(&state.docker, &record.container_id)
        .await
        .map_err(ApiFailure::docker)?;

    if request.delete_data {
        let canonical_root = tokio::fs::canonicalize(&state.environment_root)
            .await
            .map_err(ApiFailure::internal)?;
        let resource_root = PathBuf::from(&record.config_path)
            .parent()
            .ok_or_else(ApiFailure::invalid_request)?
            .to_owned();
        let canonical_target = tokio::fs::canonicalize(&resource_root)
            .await
            .map_err(ApiFailure::internal)?;
        webtop_contracts::require_contained_path(&canonical_root, &canonical_target)
            .map_err(|error| ApiFailure(StatusCode::BAD_REQUEST, error))?;
        tokio::fs::remove_dir_all(canonical_target)
            .await
            .map_err(ApiFailure::internal)?;
    }
    state
        .database
        .delete_environment(id)
        .await
        .map_err(ApiFailure::internal)?;
    if record.spec.publication.enabled {
        reconcile_frpc_if_desired(&state).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn required_environment(state: &AppState, id: Uuid) -> Result<EnvironmentRecord, ApiFailure> {
    state
        .database
        .get_environment(id)
        .await
        .map_err(ApiFailure::internal)?
        .ok_or_else(ApiFailure::invalid_request)
}

async fn ensure_environment_not_busy(state: &AppState, id: Uuid) -> Result<(), ApiFailure> {
    if state
        .active_resources
        .lock()
        .await
        .contains(&format!("environment:{id}"))
    {
        Err(ApiFailure::resource_busy())
    } else {
        Ok(())
    }
}

fn generate_server_token() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 64)
}

#[derive(Debug)]
struct ServerTokenMaterial {
    state: ServerTokenState,
    token: Option<Vec<u8>>,
    fingerprint: Option<String>,
}

fn apply_server_token_state(
    mut settings: ServerSettings,
    material: &ServerTokenMaterial,
) -> ServerSettings {
    settings.token_configured = material.token.is_some();
    settings.token_state = material.state.clone();
    settings
}

fn environment_proxies(
    environments: &[EnvironmentRecord],
) -> anyhow::Result<Vec<crate::frp::Proxy>> {
    environments
        .iter()
        .filter(|record| record.spec.publication.enabled && record.desired_running)
        .map(|record| {
            Ok(crate::frp::Proxy {
                resource_id: record.id.to_string(),
                local_port: record
                    .local_port
                    .ok_or_else(|| anyhow::anyhow!("running environment has no local port"))?,
                remote_port: record
                    .spec
                    .publication
                    .remote_port
                    .ok_or_else(|| anyhow::anyhow!("published environment has no remote port"))?,
            })
        })
        .collect()
}

async fn current_environment_proxies(state: &AppState) -> anyhow::Result<Vec<crate::frp::Proxy>> {
    let mut environments = state.database.list_environments().await?;
    synchronize_environment_local_ports(state, &mut environments).await?;
    environment_proxies(&environments)
}

async fn start_frpc_with_port_retry(state: &AppState) -> Result<FrpcServiceStatus, ApiFailure> {
    let settings = required_server_settings(state).await?;
    let token = required_server_token(state, false).await?;
    let mut rejected_ports = BTreeSet::new();
    loop {
        let proxies = current_environment_proxies(state)
            .await
            .map_err(ApiFailure::internal)?;
        let status = docker::start_frpc(
            &state.docker,
            &state.frpc_container_name,
            &settings,
            &token,
            &proxies,
        )
        .await
        .map_err(ApiFailure::docker)?;
        let conflicts =
            docker::frpc_conflicting_proxy_ids(&state.docker, &state.frpc_container_name, &proxies)
                .await
                .map_err(ApiFailure::docker)?;
        if conflicts.is_empty() {
            return Ok(status);
        }

        let mut environments = state
            .database
            .list_environments()
            .await
            .map_err(ApiFailure::internal)?;
        let mut allocated = environments
            .iter()
            .filter_map(|environment| environment.spec.publication.remote_port)
            .collect::<BTreeSet<_>>();
        allocated.extend(rejected_ports.iter().copied());
        for resource_id in conflicts {
            let id = Uuid::parse_str(&resource_id).map_err(ApiFailure::internal)?;
            let environment = environments
                .iter_mut()
                .find(|environment| environment.id == id)
                .ok_or_else(ApiFailure::invalid_request)?;
            let current_port = environment
                .spec
                .publication
                .remote_port
                .ok_or_else(ApiFailure::invalid_request)?;
            if !environment.spec.publication.automatic_port {
                return Err(ApiFailure::port_conflict(ApiError {
                    code: ErrorCode::PortConflict,
                    params: BTreeMap::from([("remotePort".into(), current_port.to_string())]),
                }));
            }
            rejected_ports.insert(current_port);
            allocated.insert(current_port);
            let replacement = allocate_remote_port(
                settings.remote_port_start,
                settings.remote_port_end,
                &allocated,
            )
            .map_err(ApiFailure::port_conflict)?;
            tracing::warn!(
                environment_id = %id,
                rejected_port = current_port,
                replacement_port = replacement,
                "retrying FRP proxy after concurrent remote-port conflict"
            );
            environment.spec.publication.remote_port = Some(replacement);
            state
                .database
                .update_environment_spec(id, &environment.spec)
                .await
                .map_err(ApiFailure::internal)?;
            allocated.insert(replacement);
        }
    }
}

async fn synchronize_environment_local_ports(
    state: &AppState,
    environments: &mut [EnvironmentRecord],
) -> anyhow::Result<bool> {
    let mut changed = false;
    for environment in environments
        .iter_mut()
        .filter(|environment| environment.desired_running)
    {
        changed |= synchronize_environment_local_port(state, environment).await?;
    }
    Ok(changed)
}

async fn synchronize_environment_local_port(
    state: &AppState,
    environment: &mut EnvironmentRecord,
) -> anyhow::Result<bool> {
    let local_port =
        docker::environment_local_port(&state.docker, &environment.container_id, &environment.spec)
            .await?;
    if local_port == environment.local_port {
        return Ok(false);
    }
    tracing::info!(
        environment_id = %environment.id,
        previous_port = ?environment.local_port,
        current_port = ?local_port,
        "synchronized environment local port"
    );
    state
        .database
        .set_environment_local_port(environment.id, local_port)
        .await?;
    environment.local_port = local_port;
    Ok(true)
}

pub async fn reconcile_runtime_state(state: &AppState) {
    match inspect_server_token(state).await {
        Ok(material) if material.state == ServerTokenState::Missing => {
            suspend_frpc_for_missing_token(state).await;
            return;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %error, "failed to inspect FRP token state");
            return;
        }
    }
    reconcile_frpc_if_desired(state).await;
}

async fn reconcile_frpc_if_desired(state: &AppState) {
    let _guard = state.publication_lock.lock().await;
    reconcile_frpc_if_desired_locked(state).await;
}

async fn reconcile_frpc_if_desired_locked(state: &AppState) {
    let result = async {
        if !state.database.frpc_desired_running().await? {
            return anyhow::Ok(());
        }
        start_frpc_with_port_retry(state)
            .await
            .map_err(|error| anyhow::anyhow!("FRP reconciliation failed: {:?}", error.1.code))?;
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, "failed to refresh frpc after environment change");
    }
}

async fn inspect_or_initialize_server_token(
    database: &Database,
    path: &FsPath,
) -> anyhow::Result<ServerTokenMaterial> {
    let token = read_server_token(path).await?;
    let metadata = database.get_frp_token_metadata().await?;
    if let Some(metadata) = metadata {
        let Some(token) = token else {
            return Ok(ServerTokenMaterial {
                state: ServerTokenState::Missing,
                token: None,
                fingerprint: Some(metadata.fingerprint),
            });
        };
        let fingerprint = token_fingerprint(&token);
        if fingerprint != metadata.fingerprint {
            return Ok(ServerTokenMaterial {
                state: ServerTokenState::Missing,
                token: None,
                fingerprint: Some(metadata.fingerprint),
            });
        }
        return Ok(ServerTokenMaterial {
            state: if metadata.recovery_pending {
                ServerTokenState::RecoveryPending
            } else {
                ServerTokenState::Ready
            },
            token: Some(token),
            fingerprint: Some(fingerprint),
        });
    }

    if let Some(token) = token {
        let fingerprint = token_fingerprint(&token);
        database
            .save_frp_token_metadata(&fingerprint, false)
            .await?;
        return Ok(ServerTokenMaterial {
            state: ServerTokenState::Ready,
            token: Some(token),
            fingerprint: Some(fingerprint),
        });
    }

    if database.has_server_settings().await? {
        return Ok(ServerTokenMaterial {
            state: ServerTokenState::Missing,
            token: None,
            fingerprint: None,
        });
    }

    let token = generate_server_token().into_bytes();
    replace_secret(path, &token).await?;
    let fingerprint = token_fingerprint(&token);
    database
        .save_frp_token_metadata(&fingerprint, false)
        .await?;
    Ok(ServerTokenMaterial {
        state: ServerTokenState::Ready,
        token: Some(token),
        fingerprint: Some(fingerprint),
    })
}

async fn read_server_token(path: &FsPath) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(token) if !token.is_empty() => Ok(Some(token)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn token_fingerprint(token: &[u8]) -> String {
    hex::encode(Sha256::digest(token))
}

async fn required_server_token(
    state: &AppState,
    allow_recovery_pending: bool,
) -> Result<Vec<u8>, ApiFailure> {
    let material = inspect_server_token(state)
        .await
        .map_err(ApiFailure::internal)?;
    let usable = material.state == ServerTokenState::Ready
        || (allow_recovery_pending && material.state == ServerTokenState::RecoveryPending);
    if !usable {
        return Err(ApiFailure::frp_token_recovery_required());
    }
    material
        .token
        .ok_or_else(ApiFailure::frp_token_recovery_required)
}

async fn inspect_server_token(state: &AppState) -> anyhow::Result<ServerTokenMaterial> {
    let _guard = state.token_lock.lock().await;
    inspect_or_initialize_server_token(&state.database, &state.server_token_path).await
}

async fn suspend_frpc_for_missing_token(state: &AppState) {
    let desired_running = match state.database.frpc_desired_running().await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, "failed to read frpc desired state after token loss");
            return;
        }
    };
    if !desired_running {
        return;
    }
    let _guard = state.publication_lock.lock().await;
    if let Err(error) = docker::remove_frpc(&state.docker, &state.frpc_container_name).await {
        tracing::warn!(error = %error, "failed to remove frpc after token loss");
    }
    if let Err(error) = state.database.set_frpc_desired_running(false).await {
        tracing::warn!(error = %error, "failed to suspend frpc desired state after token loss");
    }
}

async fn write_secret(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(contents).await?;
    file.sync_all().await
}

async fn replace_secret(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    let mut file = options.open(&temporary).await?;
    if let Err(error) = async {
        file.write_all(contents).await?;
        file.sync_all().await?;
        tokio::fs::rename(&temporary, path).await
    }
    .await
    {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    Ok(())
}

pub(crate) struct ApiFailure(pub(crate) StatusCode, pub(crate) ApiError);

impl ApiFailure {
    pub(crate) fn invalid_request() -> Self {
        Self(
            StatusCode::BAD_REQUEST,
            ApiError {
                code: ErrorCode::InvalidRequest,
                params: BTreeMap::new(),
            },
        )
    }

    pub(crate) fn docker(error: impl std::fmt::Display) -> Self {
        tracing::warn!(error = %error, "Docker operation failed");
        Self(
            StatusCode::BAD_GATEWAY,
            ApiError {
                code: ErrorCode::DockerUnavailable,
                params: BTreeMap::new(),
            },
        )
    }

    pub(crate) fn resource_busy() -> Self {
        Self(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::ResourceBusy,
                params: BTreeMap::new(),
            },
        )
    }

    fn frp_token_recovery_required() -> Self {
        Self(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::FrpTokenRecoveryRequired,
                params: BTreeMap::new(),
            },
        )
    }

    fn port_conflict(error: ApiError) -> Self {
        Self(StatusCode::CONFLICT, error)
    }

    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "internal controller error");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiError {
                code: ErrorCode::Internal,
                params: BTreeMap::new(),
            },
        )
    }
}

impl axum::response::IntoResponse for ApiFailure {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(self.1)).into_response()
    }
}

#[cfg(test)]
mod token_tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn first_use_generates_once_and_registers_a_fingerprint() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let token_path = directory.path().join("frp-token");

        let first = inspect_or_initialize_server_token(&database, &token_path)
            .await
            .unwrap();
        let second = inspect_or_initialize_server_token(&database, &token_path)
            .await
            .unwrap();

        assert_eq!(first.state, ServerTokenState::Ready);
        assert_eq!(first.token, second.token);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            database
                .get_frp_token_metadata()
                .await
                .unwrap()
                .unwrap()
                .fingerprint,
            first.fingerprint.unwrap()
        );
    }

    #[tokio::test]
    async fn a_missing_paired_token_is_not_silently_replaced() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let token_path = directory.path().join("frp-token");
        let original = inspect_or_initialize_server_token(&database, &token_path)
            .await
            .unwrap();
        database
            .save_server_settings(&ServerSettings::default())
            .await
            .unwrap();
        tokio::fs::remove_file(&token_path).await.unwrap();

        let missing = inspect_or_initialize_server_token(&database, &token_path)
            .await
            .unwrap();

        assert_eq!(missing.state, ServerTokenState::Missing);
        assert!(missing.token.is_none());
        assert!(!tokio::fs::try_exists(&token_path).await.unwrap());
        assert_eq!(missing.fingerprint, original.fingerprint);
    }

    #[tokio::test]
    async fn an_existing_legacy_token_is_adopted_without_rotation() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let token_path = directory.path().join("frp-token");
        tokio::fs::write(&token_path, b"legacy-managed-token")
            .await
            .unwrap();

        let adopted = inspect_or_initialize_server_token(&database, &token_path)
            .await
            .unwrap();

        assert_eq!(adopted.state, ServerTokenState::Ready);
        assert_eq!(
            adopted.token.as_deref(),
            Some(b"legacy-managed-token".as_slice())
        );
        assert_eq!(
            database
                .get_frp_token_metadata()
                .await
                .unwrap()
                .unwrap()
                .fingerprint,
            token_fingerprint(b"legacy-managed-token")
        );
    }
}
