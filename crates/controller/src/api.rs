use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::Infallible;
use std::path::PathBuf;
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
use tokio::sync::{mpsc, watch, Mutex};
use tower_http::trace::TraceLayer;
use uuid::Uuid;
use webtop_contracts::{
    allocate_remote_port, ApiError, EnvironmentSpec, ErrorCode, FrpcServiceState,
    FrpcServiceStatus, FrpcTestResult, ImageCachePruneResult, ImagePullPhase, ImagePullProgress,
    OfficialImage, ServerSettings, API_VERSION,
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
    pub pull_cancellations: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
    pub operation_cancellations: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
    pub active_resources: Arc<Mutex<HashSet<String>>>,
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
            "/v1/settings/server/token/regenerate",
            post(regenerate_server_token),
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
    capabilities: [&'static str; 13],
    docker_version: Option<String>,
}

async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiFailure> {
    let version = state.docker.version().await.map_err(ApiFailure::docker)?;
    Ok(Json(HealthResponse {
        api_version: API_VERSION,
        controller_version: env!("CARGO_PKG_VERSION"),
        capabilities: [
            "frpc_lifecycle_v1",
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

#[derive(Deserialize)]
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
    {
        let mut resources = state.active_resources.lock().await;
        if resources.contains("docker-images") {
            return Err(ApiFailure::resource_busy());
        }
        resources.insert("docker-images".into());
    }
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut cancellations = state.pull_cancellations.lock().await;
        if cancellations.contains_key(&request.pull_id) {
            state.active_resources.lock().await.remove("docker-images");
            return Err(ApiFailure::invalid_request());
        }
        cancellations.insert(request.pull_id, cancel_tx);
    }
    let (event_tx, event_rx) = mpsc::channel::<ImagePullProgress>(32);
    let docker = state.docker.clone();
    let cancellations = state.pull_cancellations.clone();
    let active_resources = state.active_resources.clone();
    let reference = request.reference.clone();
    let pull_id = request.pull_id;
    tokio::spawn(async move {
        if let Err(error) =
            docker::pull_official_image(&docker, pull_id, &reference, cancel_rx, event_tx.clone())
                .await
        {
            tracing::warn!(%error, %pull_id, %reference, "Docker image pull failed");
            let _ = event_tx
                .send(ImagePullProgress {
                    pull_id,
                    reference,
                    phase: ImagePullPhase::Error,
                    layer_id: None,
                    status: "Docker image pull failed".into(),
                    current_bytes: None,
                    total_bytes: None,
                    aggregate_current_bytes: None,
                    aggregate_total_bytes: None,
                })
                .await;
        }
        cancellations.lock().await.remove(&pull_id);
        active_resources.lock().await.remove("docker-images");
    });

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

async fn cancel_official_image_pull(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if let Some(cancel) = state.pull_cancellations.lock().await.get(&id).cloned() {
        let _ = cancel.send(true);
    }
    StatusCode::NO_CONTENT
}

async fn get_server_settings(
    State(state): State<AppState>,
) -> Result<Json<ServerSettings>, ApiFailure> {
    ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    let mut settings = state
        .database
        .get_server_settings()
        .await
        .map_err(ApiFailure::internal)?;
    settings.token_configured = tokio::fs::try_exists(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(settings))
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
    let mut settings = request.settings;
    settings.token_configured = false;
    settings
        .validate()
        .map_err(|_| ApiFailure::invalid_request())?;

    ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    settings.token_configured = true;
    state
        .database
        .save_server_settings(&settings)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(settings))
}

async fn regenerate_server_token(
    State(state): State<AppState>,
) -> Result<Json<ServerSettings>, ApiFailure> {
    let token = generate_server_token();
    replace_secret(&state.server_token_path, token.as_bytes())
        .await
        .map_err(ApiFailure::internal)?;
    docker::remove_frpc(&state.docker)
        .await
        .map_err(ApiFailure::docker)?;
    state
        .database
        .set_frpc_desired_running(false)
        .await
        .map_err(ApiFailure::internal)?;
    let mut settings = state
        .database
        .get_server_settings()
        .await
        .map_err(ApiFailure::internal)?;
    settings.token_configured = true;
    Ok(Json(settings))
}

async fn get_frps_setup_guide(
    State(state): State<AppState>,
) -> Result<Json<FrpsSetupGuideResponse>, ApiFailure> {
    let settings = required_server_settings(&state).await?;
    let token = ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
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
    let status = docker::frpc_status(&state.docker)
        .await
        .map_err(ApiFailure::docker)?;
    Ok(Json(apply_frpc_desired_state(&state, status).await?))
}

async fn start_frpc(State(state): State<AppState>) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let settings = required_server_settings(&state).await?;
    let token = ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    let proxies = current_environment_proxies(&state)
        .await
        .map_err(ApiFailure::internal)?;
    let status = docker::start_frpc(&state.docker, &settings, &token, &proxies)
        .await
        .map_err(ApiFailure::docker)?;
    state
        .database
        .set_frpc_desired_running(true)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(status))
}

async fn stop_frpc(State(state): State<AppState>) -> Result<Json<FrpcServiceStatus>, ApiFailure> {
    let status = docker::stop_frpc(&state.docker)
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
    let settings = required_server_settings(&state).await?;
    let token = ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    let proxies = current_environment_proxies(&state)
        .await
        .map_err(ApiFailure::internal)?;
    let status = docker::restart_frpc(&state.docker, &settings, &token, &proxies)
        .await
        .map_err(ApiFailure::docker)?;
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
    let token = ensure_server_token(&state.server_token_path)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(
        docker::test_frpc_connectivity(&state.docker, &settings, &token)
            .await
            .map_err(ApiFailure::docker)?,
    ))
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
    reconcile_frpc_if_desired(&state).await;
    Ok(Json(record))
}

async fn unpublish_environment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnvironmentRecord>, ApiFailure> {
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
    reconcile_frpc_if_desired(&state).await;
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
    reconcile_frpc_if_desired(state).await;
}

async fn reconcile_frpc_if_desired(state: &AppState) {
    let result = async {
        if !state.database.frpc_desired_running().await? {
            return anyhow::Ok(());
        }
        let settings = state.database.get_server_settings().await?;
        settings.validate()?;
        let token = ensure_server_token(&state.server_token_path).await?;
        let proxies = current_environment_proxies(state).await?;
        docker::restart_frpc(&state.docker, &settings, &token, &proxies).await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, "failed to refresh frpc after environment change");
    }
}

async fn ensure_server_token(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    match tokio::fs::read(path).await {
        Ok(token) if !token.is_empty() => Ok(token),
        Ok(_) => {
            let token = generate_server_token().into_bytes();
            replace_secret(path, &token).await?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let token = generate_server_token().into_bytes();
            replace_secret(path, &token).await?;
            Ok(token)
        }
        Err(error) => Err(error),
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
