use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path as FsPath, PathBuf};

use anyhow::{bail, Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use bollard::models::{ContainerConfig, ImageConfig};
use bollard::query_parameters::{
    CommitContainerOptionsBuilder, DownloadFromContainerOptionsBuilder, ImportImageOptionsBuilder,
    InspectContainerOptionsBuilder, RemoveImageOptionsBuilder, TagImageOptionsBuilder,
};
use chrono::Utc;
use futures_util::StreamExt;
use nix::sys::statvfs::statvfs;
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use uuid::Uuid;
use webtop_contracts::{
    ApiError, EnvironmentSpec, ErrorCode, OfficialTemplateSource, Operation, OperationKind,
    OperationPhase, Template, TemplateImportPreflight, TemplateIntegrity, TemplateManifest,
    TemplatePayload, TemplatePreflight, TemplateSourceCheck, TemplateSourceStatus, TemplateTrust,
    OWNER_LABEL, RESERVED_ENVIRONMENT_KEYS, RESOURCE_ID_LABEL, RESOURCE_KIND_LABEL,
};

use crate::api::{ApiFailure, AppState};
use crate::database::EnvironmentRecord;
use crate::docker;

const TEMPLATE_REPOSITORY: &str = "com.cue.webtop-manager/template";
const STAGING_REPOSITORY: &str = "com.cue.webtop-manager/import-staging";
const PACKAGE_SCHEMA_VERSION: u32 = 1;
const WORKER_DEFAULT: &str = "/usr/local/bin/webtop-worker";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/templates", get(list_templates).post(create_template))
        .route(
            "/v1/environments/{id}/template-preflight",
            post(template_preflight),
        )
        .route(
            "/v1/templates/{id}/environments",
            post(create_environment_from_template),
        )
        .route("/v1/templates/source-checks", post(check_template_sources))
        .route("/v1/templates/{id}/exports", post(export_template))
        .route("/v1/template-imports/preflight", post(import_preflight))
        .route("/v1/template-imports", post(import_template))
        .route("/v1/templates/{id}", delete(delete_template))
        .route(
            "/v1/operations/{id}",
            get(get_operation).delete(cancel_operation),
        )
}

enum CancellableJobOutcome {
    Completed(serde_json::Value),
    Cancelled,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTemplateRequest {
    environment_id: Uuid,
    name: String,
    confirmed_sensitive_data: bool,
    confirmed_space_warning: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateFromTemplateRequest {
    spec: EnvironmentSpec,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceChecksRequest {
    #[serde(default)]
    template_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingRequest {
    staging_file_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportTemplateRequest {
    staging_file_id: Uuid,
    name: String,
    confirmed_sensitive_data: bool,
    confirmed_untrusted_image: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteTemplateRequest {
    confirmation_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerReport {
    files: u64,
    directories: u64,
    symlinks: u64,
    skipped_special: u64,
    original_bytes: u64,
    archive_bytes: Option<u64>,
    sha256: Option<String>,
    sensitive_paths: Vec<String>,
}

async fn list_templates(State(state): State<AppState>) -> Result<Json<Vec<Template>>, ApiFailure> {
    let mut templates = state
        .database
        .list_templates()
        .await
        .map_err(ApiFailure::internal)?;
    for template in &mut templates {
        let next = if !state.snapshot_root.join(&template.snapshot_path).is_file() {
            TemplateIntegrity::MissingSnapshot
        } else if state
            .docker
            .inspect_image(&template.image_reference)
            .await
            .is_err()
        {
            TemplateIntegrity::MissingImage
        } else {
            TemplateIntegrity::Complete
        };
        if next != template.integrity {
            template.integrity = next;
            state
                .database
                .update_template(template)
                .await
                .map_err(ApiFailure::internal)?;
        }
    }
    Ok(Json(templates))
}

async fn template_preflight(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TemplatePreflight>, ApiFailure> {
    let record = required_environment(&state, id).await?;
    ensure_stopped_and_owned(&state, &record).await?;
    let report = run_worker(
        &state,
        serde_json::json!({"kind":"preflight","source":record.config_path}),
    )
    .await
    .map_err(ApiFailure::internal)?;
    let inspect = state
        .docker
        .inspect_container(
            &record.container_id,
            Some(InspectContainerOptionsBuilder::default().size(true).build()),
        )
        .await
        .map_err(ApiFailure::docker)?;
    let system_change_bytes = inspect.size_rw.unwrap_or_default().max(0) as u64;
    let image_upper_bound_bytes = system_change_bytes;
    let snapshot_upper_bound_bytes = report.original_bytes;
    let conservative_total_bytes = image_upper_bound_bytes
        .saturating_add(snapshot_upper_bound_bytes)
        .saturating_add(64 * 1024 * 1024);
    let available_bytes = statvfs(&state.snapshot_root).ok().map(|stats| {
        stats
            .blocks_available()
            .saturating_mul(stats.fragment_size())
    });
    Ok(Json(TemplatePreflight {
        environment_id: id,
        system_change_bytes,
        config_original_bytes: report.original_bytes,
        file_count: report.files,
        directory_count: report.directories,
        symlink_count: report.symlinks,
        skipped_special_files: report.skipped_special,
        sensitive_paths: report.sensitive_paths,
        image_upper_bound_bytes,
        snapshot_upper_bound_bytes,
        conservative_total_bytes,
        available_bytes,
        insufficient_space_warning: available_bytes
            .is_some_and(|available| available < conservative_total_bytes),
    }))
}

async fn create_template(
    State(state): State<AppState>,
    Json(request): Json<CreateTemplateRequest>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    validate_name(&request.name)?;
    if !request.confirmed_sensitive_data {
        return Err(ApiFailure::invalid_request());
    }
    let preflight = template_preflight(State(state.clone()), Path(request.environment_id))
        .await?
        .0;
    if preflight.insufficient_space_warning && !request.confirmed_space_warning {
        return Err(ApiFailure(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::InsufficientDiskSpace,
                params: BTreeMap::new(),
            },
        ));
    }
    if state
        .database
        .template_name_exists(request.name.trim())
        .await
        .map_err(ApiFailure::internal)?
    {
        return Err(template_name_conflict());
    }
    let template_id = Uuid::new_v4();
    let operation = new_operation(OperationKind::CreateTemplate, Some(template_id));
    reserve_resources(
        &state,
        &[
            "docker-images".into(),
            format!("environment:{}", request.environment_id),
            format!("template:{template_id}"),
        ],
    )
    .await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let operation_for_response = operation.clone();
    tokio::spawn(async move {
        let resources = vec![
            "docker-images".into(),
            format!("environment:{}", request.environment_id),
            format!("template:{template_id}"),
        ];
        let result = create_template_job(&state, operation.id, template_id, request).await;
        finish_job(&state, operation.id, result).await;
        release_resources(&state, &resources).await;
    });
    Ok((StatusCode::ACCEPTED, Json(operation_for_response)))
}

async fn create_template_job(
    state: &AppState,
    operation_id: Uuid,
    template_id: Uuid,
    request: CreateTemplateRequest,
) -> Result<serde_json::Value, ApiError> {
    operation_log(
        state,
        operation_id,
        "[controller] validating stopped managed environment",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(10)).await?;
    let record = required_environment_value(state, request.environment_id).await?;
    ensure_stopped_and_owned_value(state, &record).await?;
    let container = state
        .docker
        .inspect_container(
            &record.container_id,
            Some(InspectContainerOptionsBuilder::default().size(true).build()),
        )
        .await
        .map_err(docker_error)?;
    let source_image_id = container.image.ok_or_else(invalid_error)?;
    let source_image = state
        .docker
        .inspect_image(&source_image_id)
        .await
        .map_err(docker_error)?;
    verify_container_path(state, &record.container_id, "/init").await?;
    let image_reference = format!("{TEMPLATE_REPOSITORY}:{template_id}");
    let snapshot_relative = PathBuf::from(template_id.to_string()).join("config.tar.zst");
    let snapshot = state.snapshot_root.join(&snapshot_relative);
    let snapshot_parent = snapshot.parent().expect("snapshot has a parent");
    tokio::fs::create_dir_all(snapshot_parent)
        .await
        .map_err(internal_error)?;

    operation_log(
        state,
        operation_id,
        "[worker] archiving complete /config as tar.zst",
    )
    .await;
    let worker = run_worker(
        state,
        serde_json::json!({
            "kind":"snapshot",
            "source":record.config_path,
            "destination":snapshot,
        }),
    )
    .await
    .map_err(internal_error)?;
    operation_log(
        state,
        operation_id,
        &format!(
            "[worker] /config snapshot complete: {} original -> {} compressed",
            human_bytes(worker.original_bytes),
            human_bytes(worker.archive_bytes.unwrap_or_default())
        ),
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(50)).await?;

    operation_log(
        state,
        operation_id,
        "[docker] committing stopped container to app-owned template tag",
    )
    .await;
    let image_config = sanitized_commit_config(
        source_image.config.clone().unwrap_or_default(),
        template_id,
        request.environment_id,
    );
    let commit = state
        .docker
        .commit_container(
            CommitContainerOptionsBuilder::default()
                .container(&record.container_id)
                .repo(TEMPLATE_REPOSITORY)
                .tag(&template_id.to_string())
                .pause(true)
                .comment("Webtop Manager portable template")
                .build(),
            image_config,
        )
        .await;
    let commit = match commit {
        Ok(commit) => commit,
        Err(error) => {
            let _ = tokio::fs::remove_file(&snapshot).await;
            let _ = tokio::fs::remove_dir(snapshot_parent).await;
            return Err(docker_error(error));
        }
    };
    operation_log(
        state,
        operation_id,
        "[docker] commit complete; verifying platform and boot metadata",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Verifying, Some(80)).await?;
    let template_image = match verify_template_image(state, &image_reference).await {
        Ok(image) => image,
        Err(error) => {
            let _ = remove_owned_image(state, &image_reference).await;
            let _ = tokio::fs::remove_file(&snapshot).await;
            let _ = tokio::fs::remove_dir(snapshot_parent).await;
            return Err(error);
        }
    };
    let official_source = official_source(&record.spec.image, &source_image_id, &source_image);
    let parent_template_id = record.template_id;
    let mut external_lineage = Vec::new();
    if let Some(parent) = parent_template_id {
        if let Some(parent_record) = state
            .database
            .get_template(parent)
            .await
            .map_err(internal_error)?
        {
            external_lineage.extend(parent_record.external_lineage);
        }
    }
    let template = Template {
        id: template_id,
        name: request.name.trim().into(),
        image_reference: image_reference.clone(),
        image_id: template_image.id.clone().unwrap_or(commit.id),
        platform: image_platform(&template_image)?,
        system_size_bytes: template_image.size.unwrap_or_default().max(0) as u64,
        system_delta_bytes: container.size_rw.unwrap_or_default().max(0) as u64,
        snapshot_path: snapshot_relative,
        snapshot_sha256: worker.sha256.ok_or_else(invalid_error)?,
        snapshot_size_bytes: worker.archive_bytes.ok_or_else(invalid_error)?,
        snapshot_original_bytes: worker.original_bytes,
        source_environment_id: Some(request.environment_id),
        parent_template_id,
        external_lineage,
        source_spec: record.spec,
        official_source,
        source_check: TemplateSourceCheck::default(),
        integrity: TemplateIntegrity::Complete,
        trust: TemplateTrust::Local,
        created_at: Utc::now(),
    };
    if let Err(error) = state.database.insert_template(&template).await {
        let _ = remove_owned_image(state, &image_reference).await;
        let _ = tokio::fs::remove_file(&snapshot).await;
        let _ = tokio::fs::remove_dir(snapshot_parent).await;
        return Err(internal_error(error));
    }
    operation_log(
        state,
        operation_id,
        "[controller] image and snapshot published atomically",
    )
    .await;
    Ok(serde_json::json!({"templateId":template_id}))
}

async fn create_environment_from_template(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
    Json(request): Json<CreateFromTemplateRequest>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    let template = required_template(&state, template_id).await?;
    if template.integrity != TemplateIntegrity::Complete
        || state
            .docker
            .inspect_image(&template.image_reference)
            .await
            .is_err()
    {
        return Err(ApiFailure(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::TemplateImageMissing,
                params: BTreeMap::new(),
            },
        ));
    }
    let mut spec = request.spec;
    spec.image = template.image_reference.clone();
    spec.validate().map_err(|_| ApiFailure::invalid_request())?;
    validate_external_resources(&spec)?;
    if state
        .database
        .environment_name_exists(&spec.name)
        .await
        .map_err(ApiFailure::internal)?
    {
        return Err(ApiFailure::invalid_request());
    }
    let environment_id = Uuid::new_v4();
    let operation = new_operation(OperationKind::RestoreTemplate, Some(environment_id));
    let resources = vec![
        format!("template:{template_id}"),
        format!("environment:{environment_id}"),
    ];
    reserve_resources(&state, &resources).await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let response = operation.clone();
    tokio::spawn(async move {
        let result =
            create_environment_job(&state, operation.id, environment_id, template, spec).await;
        finish_job(&state, operation.id, result).await;
        release_resources(&state, &resources).await;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn create_environment_job(
    state: &AppState,
    operation_id: Uuid,
    environment_id: Uuid,
    template: Template,
    mut spec: EnvironmentSpec,
) -> Result<serde_json::Value, ApiError> {
    operation_log(
        state,
        operation_id,
        "[controller] validating template image and snapshot hash",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(10)).await?;
    let snapshot = state.snapshot_root.join(&template.snapshot_path);
    if hash_file_async(snapshot.clone())
        .await
        .map_err(internal_error)?
        .0
        != template.snapshot_sha256
    {
        return Err(ApiError {
            code: ErrorCode::SnapshotCorrupt,
            params: BTreeMap::new(),
        });
    }
    allocate_publication_port(state, &mut spec).await?;
    let resource_root = state.environment_root.join(environment_id.to_string());
    let config_path = resource_root.join("config");
    let secret_path = resource_root.join("secrets/password");
    tokio::fs::create_dir_all(&config_path)
        .await
        .map_err(internal_error)?;
    tokio::fs::create_dir_all(secret_path.parent().expect("secret parent"))
        .await
        .map_err(internal_error)?;
    let password = Alphanumeric.sample_string(&mut rand::rng(), 40);
    write_secret(&secret_path, password.as_bytes())
        .await
        .map_err(internal_error)?;
    operation_log(state, operation_id, "[worker] restoring /config snapshot").await;
    if let Err(error) = run_worker(
        state,
        serde_json::json!({"kind":"restore","archive":snapshot,"destination":config_path}),
    )
    .await
    {
        let _ = tokio::fs::remove_dir_all(&resource_root).await;
        return Err(internal_error(error));
    }
    update_operation(state, operation_id, OperationPhase::Running, Some(55)).await?;
    operation_log(
        state,
        operation_id,
        "[docker] creating environment from local template image",
    )
    .await;
    let host_root = state.host_environment_root.join(environment_id.to_string());
    let created = match docker::create_environment_container_from_local_image(
        &state.docker,
        &environment_id.to_string(),
        &spec,
        &host_root.join("config").display().to_string(),
        &host_root.join("secrets/password").display().to_string(),
    )
    .await
    {
        Ok(created) => created,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&resource_root).await;
            return Err(docker_error(error));
        }
    };
    let record = EnvironmentRecord {
        id: environment_id,
        name: spec.name.clone(),
        container_id: created.id.clone(),
        config_path: config_path.display().to_string(),
        desired_running: true,
        local_port: created.local_port,
        template_id: Some(template.id),
        spec,
        created_at: Utc::now(),
    };
    if let Err(error) = state.database.insert_environment(&record).await {
        let _ = docker::stop(&state.docker, &created.id).await;
        let _ = docker::remove(&state.docker, &created.id).await;
        let _ = tokio::fs::remove_dir_all(&resource_root).await;
        return Err(internal_error(error));
    }
    if record.spec.publication.enabled {
        crate::api::reconcile_runtime_state(state).await;
    }
    operation_log(
        state,
        operation_id,
        "[controller] environment created with new credentials and port allocation",
    )
    .await;
    Ok(serde_json::json!({"environmentId":environment_id}))
}

async fn check_template_sources(
    State(state): State<AppState>,
    Json(request): Json<SourceChecksRequest>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    let operation = new_operation(OperationKind::CheckTemplateSource, None);
    reserve_resources(&state, &["template-source-checks".into()]).await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let response = operation.clone();
    tokio::spawn(async move {
        let result = check_sources_job(&state, operation.id, request.template_ids).await;
        finish_job(&state, operation.id, result).await;
        release_resources(&state, &["template-source-checks".into()]).await;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn check_sources_job(
    state: &AppState,
    operation_id: Uuid,
    ids: Vec<Uuid>,
) -> Result<serde_json::Value, ApiError> {
    let mut templates = state
        .database
        .list_templates()
        .await
        .map_err(internal_error)?;
    if !ids.is_empty() {
        let ids: HashSet<_> = ids.into_iter().collect();
        templates.retain(|template| ids.contains(&template.id));
    }
    let total = templates.len().max(1);
    for (index, template) in templates.iter_mut().enumerate() {
        let Some(source) = &template.official_source else {
            continue;
        };
        let checked_at = Utc::now();
        template.source_check = match state
            .docker
            .inspect_registry_image(&source.reference, None)
            .await
        {
            Ok(inspect) => {
                let digest = inspect.descriptor.digest;
                let status = if digest.is_some() && digest == source.digest {
                    TemplateSourceStatus::Current
                } else {
                    TemplateSourceStatus::Updated
                };
                TemplateSourceCheck {
                    status,
                    checked_at: Some(checked_at),
                    current_digest: digest,
                }
            }
            Err(_) => TemplateSourceCheck {
                status: TemplateSourceStatus::Unavailable,
                checked_at: Some(checked_at),
                current_digest: None,
            },
        };
        state
            .database
            .update_template(template)
            .await
            .map_err(internal_error)?;
        update_operation(
            state,
            operation_id,
            OperationPhase::Running,
            Some((((index + 1) * 100) / total) as u8),
        )
        .await?;
    }
    Ok(serde_json::json!({"checked":templates.len()}))
}

async fn delete_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<DeleteTemplateRequest>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    let template = required_template(&state, id).await?;
    if request.confirmation_name != template.name {
        return Err(ApiFailure::invalid_request());
    }
    let (environments, children) = state
        .database
        .template_dependency_counts(id)
        .await
        .map_err(ApiFailure::internal)?;
    if environments > 0 || children > 0 {
        return Err(ApiFailure(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::TemplateDependency,
                params: BTreeMap::from([
                    ("environments".into(), environments.to_string()),
                    ("children".into(), children.to_string()),
                ]),
            },
        ));
    }
    let operation = new_operation(OperationKind::DeleteTemplate, Some(id));
    let resources = vec!["docker-images".into(), format!("template:{id}")];
    reserve_resources(&state, &resources).await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let response = operation.clone();
    tokio::spawn(async move {
        let result = async {
            update_operation(&state, operation.id, OperationPhase::Running, Some(20)).await?;
            remove_owned_image(&state, &template.image_reference).await?;
            let snapshot = state.snapshot_root.join(&template.snapshot_path);
            if tokio::fs::try_exists(&snapshot)
                .await
                .map_err(internal_error)?
            {
                tokio::fs::remove_file(&snapshot)
                    .await
                    .map_err(internal_error)?;
            }
            if let Some(parent) = snapshot.parent() {
                let _ = tokio::fs::remove_dir(parent).await;
            }
            state
                .database
                .delete_template(id)
                .await
                .map_err(internal_error)?;
            Ok(serde_json::json!({"templateId":id}))
        }
        .await;
        finish_job(&state, operation.id, result).await;
        release_resources(&state, &resources).await;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn get_operation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Operation>, ApiFailure> {
    state
        .database
        .get_operation(id)
        .await
        .map_err(ApiFailure::internal)?
        .map(Json)
        .ok_or_else(ApiFailure::invalid_request)
}

async fn cancel_operation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    let operation = state
        .database
        .get_operation(id)
        .await
        .map_err(ApiFailure::internal)?
        .ok_or_else(ApiFailure::invalid_request)?;
    if matches!(
        operation.phase,
        OperationPhase::Succeeded
            | OperationPhase::Failed
            | OperationPhase::Cancelled
            | OperationPhase::Retryable
    ) {
        return Ok((StatusCode::OK, Json(operation)));
    }
    if !operation.cancellable {
        return Err(ApiFailure(
            StatusCode::CONFLICT,
            ApiError {
                code: ErrorCode::OperationNotCancellable,
                params: BTreeMap::new(),
            },
        ));
    }
    let cancellation = state
        .operation_cancellations
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiFailure::invalid_request)?;
    cancellation
        .send(true)
        .map_err(|_| ApiFailure::invalid_request())?;
    operation_log(
        &state,
        id,
        "[controller] stop requested; cleaning intermediate artifacts",
    )
    .await;
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
    let updated = state
        .database
        .get_operation(id)
        .await
        .map_err(ApiFailure::internal)?
        .ok_or_else(ApiFailure::invalid_request)?;
    Ok((StatusCode::ACCEPTED, Json(updated)))
}

// Transfer endpoints are kept in this module so every path reaching Docker is
// constrained to a UUID under the controller-owned staging directory.
async fn export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    let template = required_template(&state, id).await?;
    if template.integrity != TemplateIntegrity::Complete {
        return Err(ApiFailure::invalid_request());
    }
    let staging_id = Uuid::new_v4();
    let operation = new_operation(OperationKind::ExportTemplate, Some(id));
    let resources = vec![
        "docker-images".into(),
        format!("template:{id}"),
        format!("staging:{staging_id}"),
    ];
    reserve_resources(&state, &resources).await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let (cancellation_sender, cancellation) = watch::channel(false);
    state
        .operation_cancellations
        .lock()
        .await
        .insert(operation.id, cancellation_sender);
    let response = operation.clone();
    tokio::spawn(async move {
        let result = export_job(&state, operation.id, &template, staging_id, cancellation).await;
        if !matches!(result, Ok(CancellableJobOutcome::Completed(_))) {
            let _ = tokio::fs::remove_file(staging_package_path(&state, staging_id)).await;
            let _ = tokio::fs::remove_dir_all(
                state
                    .staging_root
                    .join(format!(".{staging_id}.partial-dir")),
            )
            .await;
            let _ = tokio::fs::remove_file(
                state
                    .staging_root
                    .join(format!(".{staging_id}.wtmpl.partial")),
            )
            .await;
        }
        finish_cancellable_job(&state, operation.id, result).await;
        state
            .operation_cancellations
            .lock()
            .await
            .remove(&operation.id);
        release_resources(&state, &resources).await;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn export_job(
    state: &AppState,
    operation_id: Uuid,
    template: &Template,
    staging_id: Uuid,
    mut cancellation: watch::Receiver<bool>,
) -> Result<CancellableJobOutcome, ApiError> {
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    operation_log(
        state,
        operation_id,
        "[controller] preparing self-contained Docker image payload",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(5)).await?;
    let temporary_dir = state
        .staging_root
        .join(format!(".{staging_id}.partial-dir"));
    tokio::fs::create_dir(&temporary_dir)
        .await
        .map_err(internal_error)?;
    let image_payload = temporary_dir.join("image.tar.zst");
    let image_temporary = temporary_dir.join("image.tar.zst.partial");
    let output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&image_temporary)
        .await
        .map_err(internal_error)?;
    let mut encoder =
        zstd::stream::write::Encoder::new(output.into_std().await, 9).map_err(internal_error)?;
    operation_log(state, operation_id, "[docker] save image layers | zstd -9").await;
    let mut stream = state.docker.export_image(&template.image_reference);
    let mut streamed_bytes = 0_u64;
    let mut last_reported_bytes = 0_u64;
    loop {
        let next = tokio::select! {
            changed = cancellation.changed() => {
                if changed.is_ok() && cancellation_requested(&cancellation) {
                    return Ok(CancellableJobOutcome::Cancelled);
                }
                continue;
            }
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(docker_error)?;
        streamed_bytes = streamed_bytes.saturating_add(chunk.len() as u64);
        encoder.write_all(&chunk).map_err(internal_error)?;
        if streamed_bytes.saturating_sub(last_reported_bytes) >= 128 * 1024 * 1024 {
            last_reported_bytes = streamed_bytes;
            let expected = template.system_size_bytes.max(1);
            let progress = 5 + ((streamed_bytes.saturating_mul(40) / expected).min(40) as u8);
            update_operation(state, operation_id, OperationPhase::Running, Some(progress)).await?;
            operation_log(
                state,
                operation_id,
                &format!(
                    "[docker] streamed {} of image data",
                    human_bytes(streamed_bytes)
                ),
            )
            .await;
        }
    }
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let file = encoder.finish().map_err(internal_error)?;
    file.sync_all().map_err(internal_error)?;
    tokio::fs::rename(&image_temporary, &image_payload)
        .await
        .map_err(internal_error)?;
    operation_log(
        state,
        operation_id,
        &format!(
            "[docker] image stream complete: {}",
            human_bytes(streamed_bytes)
        ),
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(55)).await?;
    operation_log(
        state,
        operation_id,
        "[controller] calculating payload SHA-256 checksums",
    )
    .await;
    let (image_sha, image_size) = hash_file_async(image_payload.clone())
        .await
        .map_err(internal_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let config_source = state.snapshot_root.join(&template.snapshot_path);
    let (config_sha, config_size) = hash_file_async(config_source.clone())
        .await
        .map_err(internal_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    if config_sha != template.snapshot_sha256 {
        let _ = tokio::fs::remove_dir_all(&temporary_dir).await;
        return Err(ApiError {
            code: ErrorCode::SnapshotCorrupt,
            params: BTreeMap::new(),
        });
    }
    let image_for_config_hash = image_payload.clone();
    let image_config_sha256 =
        tokio::task::spawn_blocking(move || saved_image_config_sha256(&image_for_config_hash))
            .await
            .map_err(internal_error)?
            .map_err(internal_error)?;
    let mut lineage = template.external_lineage.clone();
    if let Some(parent) = template.parent_template_id {
        lineage.push(parent);
    }
    let manifest = TemplateManifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        exported_template_id: template.id,
        name: template.name.clone(),
        platform: template.platform.clone(),
        image_reference: template.image_reference.clone(),
        image_id: template.image_id.clone(),
        image_config_sha256,
        source_spec: template.source_spec.clone(),
        official_source: template.official_source.clone(),
        lineage,
        image_payload: TemplatePayload {
            path: "payload/image.tar.zst".into(),
            size_bytes: image_size,
            sha256: image_sha,
        },
        config_payload: TemplatePayload {
            path: "payload/config.tar.zst".into(),
            size_bytes: config_size,
            sha256: config_sha,
        },
        created_at: template.created_at,
    };
    let destination = staging_package_path(state, staging_id);
    let temporary_package = state
        .staging_root
        .join(format!(".{staging_id}.wtmpl.partial"));
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(internal_error)?;
    let image_for_tar = image_payload.clone();
    operation_log(
        state,
        operation_id,
        "[controller] assembling manifest.json and two payloads into .wtmpl",
    )
    .await;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary_package)?;
        let mut archive = tar::Builder::new(file);
        append_bytes(&mut archive, "manifest.json", &manifest_json)?;
        append_file(&mut archive, "payload/image.tar.zst", &image_for_tar)?;
        append_file(&mut archive, "payload/config.tar.zst", &config_source)?;
        let file = archive.into_inner()?;
        file.sync_all()?;
        std::fs::rename(&temporary_package, &destination)?;
        File::open(destination.parent().context("staging root")?)?.sync_all()?;
        Ok(())
    })
    .await
    .map_err(internal_error)?
    .map_err(internal_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let _ = tokio::fs::remove_dir_all(&temporary_dir).await;
    operation_log(
        state,
        operation_id,
        &format!(
            "[controller] export staged: image {}, config {}",
            human_bytes(image_size),
            human_bytes(config_size)
        ),
    )
    .await;
    Ok(CancellableJobOutcome::Completed(
        serde_json::json!({"stagingFileId":staging_id,"suggestedName":format!("{}.wtmpl", safe_filename(&template.name))}),
    ))
}

async fn import_preflight(
    State(state): State<AppState>,
    Json(request): Json<StagingRequest>,
) -> Result<Json<TemplateImportPreflight>, ApiFailure> {
    let manifest = match validate_package(&state, request.staging_file_id).await {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ =
                tokio::fs::remove_file(staging_package_path(&state, request.staging_file_id)).await;
            return Err(package_failure(error));
        }
    };
    let conflict = state
        .database
        .template_name_exists(&manifest.name)
        .await
        .map_err(ApiFailure::internal)?;
    Ok(Json(TemplateImportPreflight {
        staging_file_id: request.staging_file_id,
        manifest,
        name_conflict: conflict,
        sensitive_data_warning: true,
        untrusted_image_warning: true,
    }))
}

async fn import_template(
    State(state): State<AppState>,
    Json(request): Json<ImportTemplateRequest>,
) -> Result<(StatusCode, Json<Operation>), ApiFailure> {
    validate_name(&request.name)?;
    if !request.confirmed_sensitive_data || !request.confirmed_untrusted_image {
        return Err(ApiFailure::invalid_request());
    }
    if state
        .database
        .template_name_exists(request.name.trim())
        .await
        .map_err(ApiFailure::internal)?
    {
        return Err(template_name_conflict());
    }
    validate_package(&state, request.staging_file_id)
        .await
        .map_err(package_failure)?;
    let template_id = Uuid::new_v4();
    let operation = new_operation(OperationKind::ImportTemplate, Some(template_id));
    let resources = vec![
        "docker-images".into(),
        format!("template:{template_id}"),
        format!("staging:{}", request.staging_file_id),
    ];
    reserve_resources(&state, &resources).await?;
    state
        .database
        .insert_operation(&operation)
        .await
        .map_err(ApiFailure::internal)?;
    let (cancellation_sender, cancellation) = watch::channel(false);
    state
        .operation_cancellations
        .lock()
        .await
        .insert(operation.id, cancellation_sender);
    let response = operation.clone();
    tokio::spawn(async move {
        let staging_file_id = request.staging_file_id;
        let result = import_job(&state, operation.id, template_id, request, cancellation).await;
        if !matches!(result, Ok(CancellableJobOutcome::Completed(_))) {
            cleanup_import_artifacts(&state, template_id, staging_file_id).await;
        }
        finish_cancellable_job(&state, operation.id, result).await;
        state
            .operation_cancellations
            .lock()
            .await
            .remove(&operation.id);
        release_resources(&state, &resources).await;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn import_job(
    state: &AppState,
    operation_id: Uuid,
    template_id: Uuid,
    request: ImportTemplateRequest,
    mut cancellation: watch::Receiver<bool>,
) -> Result<CancellableJobOutcome, ApiError> {
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    operation_log(
        state,
        operation_id,
        "[controller] extracting and validating .wtmpl whitelist",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(5)).await?;
    let package = staging_package_path(state, request.staging_file_id);
    let extract_root = state
        .staging_root
        .join(format!(".import-{template_id}.partial-dir"));
    let manifest = extract_package(package.clone(), extract_root.clone())
        .await
        .map_err(package_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let staging_reference = format!("{STAGING_REPOSITORY}:{template_id}");
    let final_reference = format!("{TEMPLATE_REPOSITORY}:{template_id}");
    if state.docker.inspect_image(&staging_reference).await.is_ok()
        || state.docker.inspect_image(&final_reference).await.is_ok()
    {
        let _ = tokio::fs::remove_dir_all(&extract_root).await;
        return Err(invalid_error());
    }
    let rewritten = extract_root.join("image.load.tar");
    operation_log(
        state,
        operation_id,
        "[controller] rewriting imported image to a random staging tag",
    )
    .await;
    rewrite_image_archive(
        extract_root.join("payload/image.tar.zst"),
        rewritten.clone(),
        &manifest.image_reference,
        &staging_reference,
    )
    .await
    .map_err(package_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    validate_rewritten_image_archive(&rewritten, &staging_reference, template_id)
        .await
        .map_err(package_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    operation_log(
        state,
        operation_id,
        "[controller] schema, hashes, platform, labels and image metadata accepted",
    )
    .await;
    update_operation(state, operation_id, OperationPhase::Running, Some(45)).await?;
    let image_file = tokio::fs::File::open(&rewritten)
        .await
        .map_err(internal_error)?;
    let image_stream = futures_util::stream::try_unfold(image_file, |mut file| async move {
        let mut buffer = vec![0_u8; 1024 * 1024];
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            Ok::<_, std::io::Error>(None)
        } else {
            buffer.truncate(read);
            Ok(Some((bytes::Bytes::from(buffer), file)))
        }
    });
    operation_log(
        state,
        operation_id,
        "[docker] load verified image archive without running it",
    )
    .await;
    let mut load = state.docker.import_image(
        ImportImageOptionsBuilder::default().quiet(false).build(),
        bollard::body_try_stream(image_stream),
        None,
    );
    loop {
        let next = tokio::select! {
            changed = cancellation.changed() => {
                if changed.is_ok() && cancellation_requested(&cancellation) {
                    return Ok(CancellableJobOutcome::Cancelled);
                }
                continue;
            }
            next = load.next() => next,
        };
        let Some(item) = next else { break };
        let item = item.map_err(docker_error)?;
        tracing::info!(response = ?item, "Docker load template response");
        if let Some(message) = item
            .stream
            .as_deref()
            .or(item.status.as_deref())
            .or_else(|| item.error_detail.as_ref()?.message.as_deref())
        {
            for line in message.lines().filter(|line| !line.trim().is_empty()) {
                operation_log(state, operation_id, &format!("[docker] {}", line.trim())).await;
            }
        }
    }
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    if let Err(error) = state.docker.inspect_image(&staging_reference).await {
        let _ = tokio::fs::remove_dir_all(&extract_root).await;
        return Err(docker_error(error));
    }
    state
        .docker
        .tag_image(
            &staging_reference,
            Some(
                TagImageOptionsBuilder::default()
                    .repo(TEMPLATE_REPOSITORY)
                    .tag(&template_id.to_string())
                    .build(),
            ),
        )
        .await
        .map_err(docker_error)?;
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    operation_log(
        state,
        operation_id,
        "[docker] staging image retagged to a new app-owned template UUID",
    )
    .await;
    let _ = state
        .docker
        .remove_image(
            &staging_reference,
            Some(
                RemoveImageOptionsBuilder::default()
                    .force(false)
                    .noprune(true)
                    .build(),
            ),
            None,
        )
        .await;
    update_operation(state, operation_id, OperationPhase::Verifying, Some(75)).await?;
    operation_log(
        state,
        operation_id,
        "[controller] verifying imported /init, /config volume and linux/amd64 platform",
    )
    .await;
    let imported = match verify_template_image(state, &final_reference).await {
        Ok(image) => image,
        Err(error) => {
            let _ = remove_owned_image(state, &final_reference).await;
            let _ = tokio::fs::remove_dir_all(&extract_root).await;
            return Err(error);
        }
    };
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let config_source = extract_root.join("payload/config.tar.zst");
    let snapshot_relative = PathBuf::from(template_id.to_string()).join("config.tar.zst");
    let snapshot = state.snapshot_root.join(&snapshot_relative);
    tokio::fs::create_dir_all(snapshot.parent().expect("snapshot parent"))
        .await
        .map_err(internal_error)?;
    let snapshot_copied =
        copy_file_atomic_0600(config_source, snapshot.clone(), cancellation.clone())
            .await
            .map_err(internal_error)?;
    if !snapshot_copied || cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    let mut lineage = manifest.lineage.clone();
    lineage.push(manifest.exported_template_id);
    lineage.sort();
    lineage.dedup();
    let template = Template {
        id: template_id,
        name: request.name.trim().into(),
        image_reference: final_reference.clone(),
        image_id: imported.id.clone().ok_or_else(invalid_error)?,
        platform: image_platform(&imported)?,
        system_size_bytes: imported.size.unwrap_or_default().max(0) as u64,
        system_delta_bytes: 0,
        snapshot_path: snapshot_relative,
        snapshot_sha256: manifest.config_payload.sha256,
        snapshot_size_bytes: manifest.config_payload.size_bytes,
        snapshot_original_bytes: 0,
        source_environment_id: None,
        parent_template_id: None,
        external_lineage: lineage,
        source_spec: manifest.source_spec,
        official_source: manifest.official_source,
        source_check: TemplateSourceCheck::default(),
        integrity: TemplateIntegrity::Complete,
        trust: TemplateTrust::ImportedUntrusted,
        created_at: Utc::now(),
    };
    if cancellation_requested(&cancellation) {
        return Ok(CancellableJobOutcome::Cancelled);
    }
    if let Err(error) = state.database.insert_template(&template).await {
        let _ = remove_owned_image(state, &final_reference).await;
        let _ = tokio::fs::remove_file(&snapshot).await;
        let _ = tokio::fs::remove_dir_all(&extract_root).await;
        return Err(internal_error(error));
    }
    let _ = tokio::fs::remove_dir_all(&extract_root).await;
    let _ = tokio::fs::remove_file(&package).await;
    operation_log(
        state,
        operation_id,
        "[controller] imported template published; staging package removed",
    )
    .await;
    Ok(CancellableJobOutcome::Completed(
        serde_json::json!({"templateId":template_id}),
    ))
}

async fn validate_rewritten_image_archive(
    path: &FsPath,
    expected_reference: &str,
    expected_id: Uuid,
) -> Result<()> {
    let path = path.to_owned();
    let expected_reference = expected_reference.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut archive = tar::Archive::new(File::open(&path)?);
        let mut manifest = None;
        for entry in archive.entries()? {
            let entry = entry?;
            let entry_path = entry.path()?.into_owned();
            ensure_safe_archive_path(&entry_path)?;
            if entry_path == FsPath::new("manifest.json") {
                let mut bytes = Vec::new();
                entry.take(1024 * 1024).read_to_end(&mut bytes)?;
                manifest = Some(serde_json::from_slice::<serde_json::Value>(&bytes)?);
            }
        }
        let manifest = manifest.context("rewritten Docker manifest missing")?;
        let images = manifest
            .as_array()
            .context("rewritten Docker manifest invalid")?;
        if images.len() != 1
            || images[0].get("RepoTags") != Some(&serde_json::json!([expected_reference]))
        {
            bail!("rewritten Docker tag mismatch");
        }
        let config_path = images[0]
            .get("Config")
            .and_then(serde_json::Value::as_str)
            .context("rewritten Docker config path missing")?;
        ensure_safe_archive_path(FsPath::new(config_path))?;
        let config_bytes =
            read_plain_archive_entry(&path, FsPath::new(config_path), 16 * 1024 * 1024)?
                .with_context(|| {
                    format!("rewritten Docker config missing: expected {config_path}")
                })?;
        let config: serde_json::Value = serde_json::from_slice(&config_bytes)?;
        let labels = config
            .get("config")
            .and_then(|value| value.get("Labels"))
            .context("rewritten Docker labels missing")?;
        if labels.get(OWNER_LABEL).and_then(serde_json::Value::as_str) != Some("managed")
            || labels
                .get(RESOURCE_KIND_LABEL)
                .and_then(serde_json::Value::as_str)
                != Some("template")
            || labels
                .get(RESOURCE_ID_LABEL)
                .and_then(serde_json::Value::as_str)
                != Some(expected_id.to_string().as_str())
        {
            bail!("rewritten Docker ownership labels mismatch");
        }
        Ok(())
    })
    .await??;
    Ok(())
}

fn new_operation(kind: OperationKind, resource_id: Option<Uuid>) -> Operation {
    let now = Utc::now();
    let command = match kind {
        OperationKind::CreateTemplate => "$ webtop-manager template create",
        OperationKind::ExportTemplate => "$ webtop-manager template export",
        OperationKind::ImportTemplate => "$ webtop-manager template import",
        OperationKind::RestoreTemplate => "$ webtop-manager template restore",
        _ => "$ webtop-manager operation start",
    };
    let cancellable = matches!(
        &kind,
        OperationKind::ExportTemplate | OperationKind::ImportTemplate
    );
    Operation {
        id: Uuid::new_v4(),
        kind,
        phase: OperationPhase::Queued,
        progress_percent: Some(0),
        cancellable,
        resource_id,
        error: None,
        result: None,
        log_lines: vec![command.into(), "[controller] queued".into()],
        created_at: now,
        updated_at: now,
    }
}

async fn finish_job(state: &AppState, id: Uuid, result: Result<serde_json::Value, ApiError>) {
    match result {
        Ok(result) => {
            operation_log(state, id, "[controller] operation completed").await;
            if let Err(error) = state
                .database
                .update_operation(
                    id,
                    OperationPhase::Succeeded,
                    Some(100),
                    None,
                    Some(&result),
                )
                .await
            {
                tracing::error!(%error, operation_id=%id, "persist operation success");
            }
        }
        Err(error) => {
            operation_log(
                state,
                id,
                &format!("[controller] operation failed: {:?}", error.code),
            )
            .await;
            if let Err(db_error) = state
                .database
                .update_operation(id, OperationPhase::Failed, None, Some(&error), None)
                .await
            {
                tracing::error!(%db_error, operation_id=%id, "persist operation failure");
            }
        }
    }
}

async fn finish_cancellable_job(
    state: &AppState,
    id: Uuid,
    result: Result<CancellableJobOutcome, ApiError>,
) {
    match result {
        Ok(CancellableJobOutcome::Completed(result)) => {
            finish_job(state, id, Ok(result)).await;
        }
        Ok(CancellableJobOutcome::Cancelled) => {
            operation_log(
                state,
                id,
                "[controller] operation cancelled; intermediate artifacts removed",
            )
            .await;
            if let Err(error) = state
                .database
                .update_operation(id, OperationPhase::Cancelled, None, None, None)
                .await
            {
                tracing::error!(%error, operation_id=%id, "persist operation cancellation");
            }
        }
        Err(error) => finish_job(state, id, Err(error)).await,
    }
}

fn cancellation_requested(cancellation: &watch::Receiver<bool>) -> bool {
    *cancellation.borrow()
}

async fn operation_log(state: &AppState, id: Uuid, value: &str) {
    let line: String = value
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .take(512)
        .collect();
    if line.is_empty() {
        return;
    }
    if let Err(error) = state.database.append_operation_log(id, &line).await {
        tracing::warn!(%error, operation_id=%id, "persist operation output");
    }
}

fn human_bytes(value: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", value as f64 / GIB)
    } else {
        format!("{:.1} MiB", value as f64 / MIB)
    }
}

async fn update_operation(
    state: &AppState,
    id: Uuid,
    phase: OperationPhase,
    progress: Option<u8>,
) -> Result<(), ApiError> {
    state
        .database
        .update_operation(id, phase, progress, None, None)
        .await
        .map_err(internal_error)
}

async fn reserve_resources(state: &AppState, resources: &[String]) -> Result<(), ApiFailure> {
    if resources.iter().any(|resource| resource == "docker-images")
        && !state.pull_cancellations.lock().await.is_empty()
    {
        return Err(ApiFailure::resource_busy());
    }
    let mut active = state.active_resources.lock().await;
    if resources.iter().any(|resource| active.contains(resource)) {
        return Err(ApiFailure::resource_busy());
    }
    active.extend(resources.iter().cloned());
    Ok(())
}

async fn release_resources(state: &AppState, resources: &[String]) {
    let mut active = state.active_resources.lock().await;
    for resource in resources {
        active.remove(resource);
    }
}

async fn required_environment(state: &AppState, id: Uuid) -> Result<EnvironmentRecord, ApiFailure> {
    state
        .database
        .get_environment(id)
        .await
        .map_err(ApiFailure::internal)?
        .ok_or_else(ApiFailure::invalid_request)
}

async fn required_environment_value(
    state: &AppState,
    id: Uuid,
) -> Result<EnvironmentRecord, ApiError> {
    state
        .database
        .get_environment(id)
        .await
        .map_err(internal_error)?
        .ok_or_else(invalid_error)
}

async fn required_template(state: &AppState, id: Uuid) -> Result<Template, ApiFailure> {
    state
        .database
        .get_template(id)
        .await
        .map_err(ApiFailure::internal)?
        .ok_or_else(ApiFailure::invalid_request)
}

async fn ensure_stopped_and_owned(
    state: &AppState,
    record: &EnvironmentRecord,
) -> Result<(), ApiFailure> {
    ensure_stopped_and_owned_value(state, record)
        .await
        .map_err(|error| ApiFailure(StatusCode::CONFLICT, error))
}

async fn ensure_stopped_and_owned_value(
    state: &AppState,
    record: &EnvironmentRecord,
) -> Result<(), ApiError> {
    let inspect = state
        .docker
        .inspect_container(&record.container_id, None)
        .await
        .map_err(docker_error)?;
    if inspect.state.as_ref().and_then(|value| value.running) == Some(true) {
        return Err(busy_error());
    }
    let labels = inspect
        .config
        .and_then(|config| config.labels)
        .unwrap_or_default();
    if labels.get(OWNER_LABEL).map(String::as_str) != Some("managed")
        || labels.get(RESOURCE_KIND_LABEL).map(String::as_str) != Some("environment")
        || labels.get(RESOURCE_ID_LABEL).map(String::as_str) != Some(record.id.to_string().as_str())
    {
        return Err(invalid_error());
    }
    Ok(())
}

async fn verify_container_path(
    state: &AppState,
    container_id: &str,
    path: &str,
) -> Result<(), ApiError> {
    let mut stream = state.docker.download_from_container(
        container_id,
        Some(
            DownloadFromContainerOptionsBuilder::default()
                .path(path)
                .build(),
        ),
    );
    match stream.next().await {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(docker_error(error)),
        None => Err(invalid_error()),
    }
}

fn sanitized_commit_config(
    source: ImageConfig,
    template_id: Uuid,
    source_environment_id: Uuid,
) -> ContainerConfig {
    let env = source.env.map(|values| {
        values
            .into_iter()
            .filter(|value| {
                let key = value.split('=').next().unwrap_or_default();
                let upper = key.to_ascii_uppercase();
                !RESERVED_ENVIRONMENT_KEYS.contains(&key)
                    && key != "LC_ALL"
                    && key != "SELKIES_MASTER_TOKEN"
                    && !key.starts_with("FILE__")
                    && !upper.contains("PASSWORD")
                    && !upper.contains("TOKEN")
                    && !upper.contains("SECRET")
                    && !upper.ends_with("_KEY")
            })
            .collect()
    });
    let mut labels = source.labels.unwrap_or_default();
    labels.insert(OWNER_LABEL.into(), "managed".into());
    labels.insert(RESOURCE_KIND_LABEL.into(), "template".into());
    labels.insert(RESOURCE_ID_LABEL.into(), template_id.to_string());
    labels.insert(
        "com.cue.webtop-manager.source-environment-id".into(),
        source_environment_id.to_string(),
    );
    let mut volumes = source.volumes.unwrap_or_default();
    if !volumes.iter().any(|value| value == "/config") {
        volumes.push("/config".into());
    }
    ContainerConfig {
        user: source.user,
        exposed_ports: source.exposed_ports,
        env,
        cmd: source.cmd,
        healthcheck: source.healthcheck,
        args_escaped: source.args_escaped,
        volumes: Some(volumes),
        working_dir: source.working_dir,
        entrypoint: source.entrypoint,
        on_build: source.on_build,
        labels: Some(labels),
        stop_signal: source.stop_signal,
        shell: source.shell,
        ..Default::default()
    }
}

async fn verify_template_image(
    state: &AppState,
    reference: &str,
) -> Result<bollard::models::ImageInspect, ApiError> {
    let image = state
        .docker
        .inspect_image(reference)
        .await
        .map_err(docker_error)?;
    if image.os.as_deref() != Some("linux") || image.architecture.as_deref() != Some("amd64") {
        return Err(invalid_error());
    }
    let config = image.config.as_ref().ok_or_else(invalid_error)?;
    if !config
        .entrypoint
        .as_ref()
        .is_some_and(|values| values.iter().any(|value| value == "/init"))
        || !config
            .volumes
            .as_ref()
            .is_some_and(|values| values.iter().any(|value| value == "/config"))
    {
        return Err(invalid_error());
    }
    let labels = config.labels.as_ref().ok_or_else(invalid_error)?;
    if labels.get(OWNER_LABEL).map(String::as_str) != Some("managed")
        || labels.get(RESOURCE_KIND_LABEL).map(String::as_str) != Some("template")
    {
        return Err(invalid_error());
    }
    Ok(image)
}

fn official_source(
    reference: &str,
    image_id: &str,
    image: &bollard::models::ImageInspect,
) -> Option<OfficialTemplateSource> {
    if !docker::is_official_image(reference) {
        return None;
    }
    let repository = reference.split(':').next().unwrap_or(reference);
    let digest = image.repo_digests.as_ref().and_then(|digests| {
        digests
            .iter()
            .find(|digest| digest.starts_with(repository))
            .and_then(|digest| digest.split_once('@').map(|(_, value)| value.to_owned()))
    });
    let build_version = image
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref())
        .and_then(|labels| {
            [
                "build_version",
                "BUILD_VERSION",
                "org.opencontainers.image.version",
            ]
            .iter()
            .find_map(|key| labels.get(*key).cloned())
        });
    Some(OfficialTemplateSource {
        reference: reference.into(),
        digest,
        image_id: image_id.into(),
        build_version,
    })
}

fn image_platform(image: &bollard::models::ImageInspect) -> Result<String, ApiError> {
    Ok(format!(
        "{}/{}",
        image.os.as_deref().ok_or_else(invalid_error)?,
        image.architecture.as_deref().ok_or_else(invalid_error)?
    ))
}

async fn run_worker(state: &AppState, job: serde_json::Value) -> Result<WorkerReport> {
    let id = Uuid::new_v4();
    let job_path = state.staging_root.join(format!(".worker-{id}.json"));
    let mut options = tokio::fs::OpenOptions::new();
    let mut file = options
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&job_path)
        .await?;
    file.write_all(&serde_json::to_vec(&job)?).await?;
    file.sync_all().await?;
    let worker = std::env::var_os("WEBTOP_MANAGER_WORKER").unwrap_or_else(|| WORKER_DEFAULT.into());
    let output = tokio::process::Command::new(worker)
        .arg(&job_path)
        .output()
        .await;
    let _ = tokio::fs::remove_file(&job_path).await;
    let output = output?;
    if !output.status.success() {
        bail!("worker failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    serde_json::from_slice(&output.stdout).context("parse worker report")
}

async fn allocate_publication_port(
    state: &AppState,
    spec: &mut EnvironmentSpec,
) -> Result<(), ApiError> {
    if !spec.publication.enabled {
        spec.publication.remote_port = None;
        return Ok(());
    }
    let settings = state
        .database
        .get_server_settings()
        .await
        .map_err(internal_error)?;
    settings.validate().map_err(|_| invalid_error())?;
    let allocated = state
        .database
        .list_environments()
        .await
        .map_err(internal_error)?
        .into_iter()
        .filter_map(|value| value.spec.publication.remote_port)
        .collect();
    let port = webtop_contracts::allocate_remote_port(
        settings.remote_port_start,
        settings.remote_port_end,
        &allocated,
    )?;
    spec.publication.remote_port = Some(port);
    spec.publication.automatic_port = true;
    Ok(())
}

fn validate_external_resources(spec: &EnvironmentSpec) -> Result<(), ApiFailure> {
    if spec.mounts.iter().any(|mount| !mount.host_path.exists())
        || spec.security.devices.iter().any(|device| !device.exists())
        || (spec.security.docker_socket && !FsPath::new("/var/run/docker.sock").exists())
    {
        return Err(ApiFailure::invalid_request());
    }
    Ok(())
}

async fn write_secret(path: &FsPath, contents: &[u8]) -> std::io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    let mut file = options
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .await?;
    file.write_all(contents).await?;
    file.sync_all().await
}

async fn remove_owned_image(state: &AppState, reference: &str) -> Result<(), ApiError> {
    if !reference.starts_with(&format!("{TEMPLATE_REPOSITORY}:")) {
        return Err(invalid_error());
    }
    if state.docker.inspect_image(reference).await.is_err() {
        return Ok(());
    }
    state
        .docker
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
        .map_err(docker_error)?;
    Ok(())
}

async fn remove_import_image(state: &AppState, reference: &str) {
    if !reference.starts_with(&format!("{TEMPLATE_REPOSITORY}:"))
        && !reference.starts_with(&format!("{STAGING_REPOSITORY}:"))
    {
        return;
    }
    if state.docker.inspect_image(reference).await.is_err() {
        return;
    }
    let _ = state
        .docker
        .remove_image(
            reference,
            Some(
                RemoveImageOptionsBuilder::default()
                    .force(true)
                    .noprune(false)
                    .build(),
            ),
            None,
        )
        .await;
}

async fn cleanup_import_artifacts(state: &AppState, template_id: Uuid, staging_file_id: Uuid) {
    let extract_root = state
        .staging_root
        .join(format!(".import-{template_id}.partial-dir"));
    let snapshot_parent = state.snapshot_root.join(template_id.to_string());
    let staging_reference = format!("{STAGING_REPOSITORY}:{template_id}");
    let final_reference = format!("{TEMPLATE_REPOSITORY}:{template_id}");
    remove_import_image(state, &staging_reference).await;
    remove_import_image(state, &final_reference).await;
    let _ = tokio::fs::remove_file(staging_package_path(state, staging_file_id)).await;
    let _ = tokio::fs::remove_dir_all(extract_root).await;
    let _ = tokio::fs::remove_dir_all(snapshot_parent).await;
}

fn staging_package_path(state: &AppState, id: Uuid) -> PathBuf {
    state.staging_root.join(format!("{id}.wtmpl"))
}

async fn validate_package(state: &AppState, id: Uuid) -> Result<TemplateManifest> {
    let package = staging_package_path(state, id);
    if !package.is_file() {
        bail!("staging package is missing");
    }
    let extract_root = state
        .staging_root
        .join(format!(".preflight-{id}.partial-dir"));
    let manifest = extract_package(package, extract_root.clone()).await?;
    let validation = validate_saved_image_archive(
        &extract_root.join("payload/image.tar.zst"),
        &manifest.image_reference,
        &manifest.image_config_sha256,
    )
    .await;
    let cleanup = tokio::fs::remove_dir_all(extract_root).await;
    validation?;
    cleanup?;
    Ok(manifest)
}

async fn extract_package(package: PathBuf, destination: PathBuf) -> Result<TemplateManifest> {
    let manifest = tokio::task::spawn_blocking(move || -> Result<TemplateManifest> {
        if destination.exists() {
            bail!("partial extraction already exists");
        }
        std::fs::create_dir(&destination)?;
        std::fs::create_dir(destination.join("payload"))?;
        let result = (|| -> Result<TemplateManifest> {
            let mut archive = tar::Archive::new(File::open(&package)?);
            let allowed = HashSet::from([
                "manifest.json".to_string(),
                "payload/image.tar.zst".to_string(),
                "payload/config.tar.zst".to_string(),
            ]);
            let mut seen = HashSet::new();
            for entry in archive.entries()? {
                let mut entry = entry?;
                if !entry.header().entry_type().is_file() {
                    bail!("package entries must be regular files");
                }
                let path = entry.path()?.to_string_lossy().to_string();
                if !allowed.contains(path.as_str()) || !seen.insert(path.clone()) {
                    bail!("unexpected or duplicate package entry");
                }
                let output_path = destination.join(&path);
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(output_path)?;
                std::io::copy(&mut entry, &mut output)?;
                output.sync_all()?;
            }
            if seen != allowed {
                bail!("package entries are incomplete");
            }
            let manifest_bytes = std::fs::read(destination.join("manifest.json"))?;
            if manifest_bytes.len() > 1024 * 1024 {
                bail!("manifest is too large");
            }
            let manifest: TemplateManifest = serde_json::from_slice(&manifest_bytes)?;
            validate_manifest(&manifest)?;
            let (image_hash, image_size) = hash_file(&destination.join("payload/image.tar.zst"))?;
            let (config_hash, config_size) =
                hash_file(&destination.join("payload/config.tar.zst"))?;
            if image_hash != manifest.image_payload.sha256
                || image_size != manifest.image_payload.size_bytes
                || config_hash != manifest.config_payload.sha256
                || config_size != manifest.config_payload.size_bytes
            {
                bail!("payload hash or size mismatch");
            }
            Ok(manifest)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&destination);
        }
        result
    })
    .await??;
    Ok(manifest)
}

fn validate_manifest(manifest: &TemplateManifest) -> Result<()> {
    if manifest.schema_version != PACKAGE_SCHEMA_VERSION
        || manifest.platform != "linux/amd64"
        || manifest.image_payload.path != "payload/image.tar.zst"
        || manifest.config_payload.path != "payload/config.tar.zst"
        || !manifest
            .image_reference
            .starts_with(&format!("{TEMPLATE_REPOSITORY}:"))
    {
        bail!("unsupported template manifest");
    }
    manifest.source_spec.validate()?;
    Ok(())
}

async fn validate_saved_image_archive(
    path: &FsPath,
    expected_reference: &str,
    expected_config_sha: &str,
) -> Result<()> {
    let path = path.to_owned();
    let expected = expected_reference.to_owned();
    let expected_config_sha = expected_config_sha.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let (value, config) = read_saved_manifest_and_config(&path)?;
        let images = value
            .as_array()
            .context("Docker save manifest is not an array")?;
        if images.len() != 1 {
            bail!("template must contain exactly one image");
        }
        let tags = images[0]
            .get("RepoTags")
            .and_then(|value| value.as_array())
            .context("Docker save image has no tags")?;
        if tags.len() != 1 || tags[0].as_str() != Some(expected.as_str()) {
            bail!("Docker save tag mismatch");
        }
        let config_summary = config
            .get("config")
            .context("Docker image config summary missing")?;
        if json_sha256(config_summary)? != expected_config_sha {
            bail!("Docker image config summary mismatch");
        }
        Ok(())
    })
    .await??;
    Ok(())
}

async fn rewrite_image_archive(
    source: PathBuf,
    destination: PathBuf,
    old_ref: &str,
    new_ref: &str,
) -> Result<()> {
    let old_ref = old_ref.to_owned();
    let new_ref = new_ref.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let (saved_manifest, mut config) = read_saved_manifest_and_config(&source)?;
        let images = saved_manifest
            .as_array()
            .context("invalid image manifest")?;
        let old_config_path = images
            .first()
            .and_then(|image| image.get("Config"))
            .and_then(serde_json::Value::as_str)
            .context("Docker image config path missing")?
            .to_owned();
        sanitize_imported_image_config(&mut config, &new_ref)?;
        let config_bytes = serde_json::to_vec(&config)?;
        let config_digest = hex::encode(Sha256::digest(&config_bytes));
        let new_config_path = if old_config_path.starts_with("blobs/sha256/") {
            format!("blobs/sha256/{config_digest}")
        } else {
            format!("{config_digest}.json")
        };
        let oci = prepare_oci_archive_rewrite(&source, &config_bytes, &new_ref)?;
        let decoder = zstd::Decoder::new(File::open(source)?)?;
        let mut source = tar::Archive::new(decoder);
        let output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&destination)?;
        let mut output = tar::Builder::new(output);
        let mut wrote_oci_config = false;
        for entry in source.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            ensure_safe_archive_path(&path)?;
            if path == FsPath::new("manifest.json") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
                let images = value.as_array_mut().context("invalid image manifest")?;
                if images.len() != 1 {
                    bail!("expected one image");
                }
                images[0]["RepoTags"] = serde_json::json!([new_ref]);
                images[0]["Config"] = serde_json::Value::String(new_config_path.clone());
                append_bytes(&mut output, "manifest.json", &serde_json::to_vec(&value)?)?;
            } else if path == FsPath::new("index.json") && oci.is_some() {
                append_bytes(
                    &mut output,
                    "index.json",
                    &oci.as_ref().expect("checked above").index_bytes,
                )?;
            } else if oci
                .as_ref()
                .is_some_and(|rewrite| path == rewrite.old_manifest_path)
            {
                let rewrite = oci.as_ref().expect("checked above");
                append_bytes(
                    &mut output,
                    rewrite.new_manifest_path.to_string_lossy().as_ref(),
                    &rewrite.manifest_bytes,
                )?;
            } else if oci
                .as_ref()
                .is_some_and(|rewrite| path == rewrite.old_config_path)
            {
                let rewrite = oci.as_ref().expect("checked above");
                append_bytes(
                    &mut output,
                    rewrite.new_config_path.to_string_lossy().as_ref(),
                    &config_bytes,
                )?;
                wrote_oci_config = true;
            } else if path == FsPath::new(&old_config_path) {
                append_bytes(&mut output, &new_config_path, &config_bytes)?;
            } else if path == FsPath::new("repositories") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
                let old_repo = old_ref
                    .rsplit_once(':')
                    .map(|(repo, _)| repo)
                    .unwrap_or(&old_ref);
                if let (Some(old), Some((repo, tag))) = (
                    value
                        .as_object_mut()
                        .and_then(|object| object.remove(old_repo)),
                    new_ref.rsplit_once(':'),
                ) {
                    let layer = old
                        .as_object()
                        .and_then(|object| object.values().next())
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let mut tags = serde_json::Map::new();
                    tags.insert(tag.into(), layer);
                    let mut repositories = serde_json::Map::new();
                    repositories.insert(repo.into(), serde_json::Value::Object(tags));
                    value = serde_json::Value::Object(repositories);
                }
                append_bytes(&mut output, "repositories", &serde_json::to_vec(&value)?)?;
            } else if oci.as_ref().is_some_and(|rewrite| {
                path == rewrite.new_manifest_path || path == rewrite.new_config_path
            }) {
                bail!("rewritten OCI archive path already exists");
            } else {
                let header = entry.header().clone();
                output.append(&header, &mut entry)?;
            }
        }
        if let Some(rewrite) = &oci {
            if !wrote_oci_config {
                append_bytes(
                    &mut output,
                    rewrite.new_config_path.to_string_lossy().as_ref(),
                    &config_bytes,
                )?;
            }
        }
        let file = output.into_inner()?;
        file.sync_all()?;
        Ok(())
    })
    .await??;
    Ok(())
}

struct OciArchiveRewrite {
    old_manifest_path: PathBuf,
    new_manifest_path: PathBuf,
    old_config_path: PathBuf,
    new_config_path: PathBuf,
    index_bytes: Vec<u8>,
    manifest_bytes: Vec<u8>,
}

fn prepare_oci_archive_rewrite(
    source: &FsPath,
    config_bytes: &[u8],
    new_ref: &str,
) -> Result<Option<OciArchiveRewrite>> {
    let Some(index_bytes) =
        read_compressed_archive_entry(source, FsPath::new("index.json"), 1024 * 1024)?
    else {
        return Ok(None);
    };
    let mut index: serde_json::Value = serde_json::from_slice(&index_bytes)?;
    let descriptors = index
        .get_mut("manifests")
        .and_then(serde_json::Value::as_array_mut)
        .context("OCI image index has no manifests")?;
    if descriptors.len() != 1 {
        bail!("OCI image index must contain exactly one image");
    }
    let descriptor = descriptors.first_mut().context("OCI descriptor missing")?;
    let old_manifest_path = oci_blob_path(
        descriptor
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .context("OCI manifest digest missing")?,
    )?;
    let manifest_bytes =
        read_compressed_archive_entry(source, &old_manifest_path, 16 * 1024 * 1024)?
            .context("OCI manifest blob missing")?;
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
    let config_descriptor = manifest
        .get_mut("config")
        .and_then(serde_json::Value::as_object_mut)
        .context("OCI config descriptor missing")?;
    let old_config_path = oci_blob_path(
        config_descriptor
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .context("OCI config digest missing")?,
    )?;
    let config_digest = hex::encode(Sha256::digest(config_bytes));
    let new_config_path = PathBuf::from(format!("blobs/sha256/{config_digest}"));
    config_descriptor.insert(
        "digest".into(),
        serde_json::Value::String(format!("sha256:{config_digest}")),
    );
    config_descriptor.insert("size".into(), serde_json::json!(config_bytes.len()));
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_digest = hex::encode(Sha256::digest(&manifest_bytes));
    let new_manifest_path = PathBuf::from(format!("blobs/sha256/{manifest_digest}"));
    descriptor["digest"] = serde_json::Value::String(format!("sha256:{manifest_digest}"));
    descriptor["size"] = serde_json::json!(manifest_bytes.len());
    let annotations = descriptor
        .as_object_mut()
        .context("OCI manifest descriptor invalid")?
        .entry("annotations")
        .or_insert_with(|| serde_json::json!({}));
    if !annotations.is_object() {
        *annotations = serde_json::json!({});
    }
    let annotations = annotations
        .as_object_mut()
        .context("OCI annotations invalid")?;
    annotations.insert(
        "io.containerd.image.name".into(),
        serde_json::Value::String(new_ref.into()),
    );
    annotations.insert(
        "org.opencontainers.image.ref.name".into(),
        serde_json::Value::String(
            new_ref
                .rsplit_once(':')
                .map(|(_, tag)| tag)
                .unwrap_or(new_ref)
                .into(),
        ),
    );
    Ok(Some(OciArchiveRewrite {
        old_manifest_path,
        new_manifest_path,
        old_config_path,
        new_config_path,
        index_bytes: serde_json::to_vec(&index)?,
        manifest_bytes,
    }))
}

fn oci_blob_path(digest: &str) -> Result<PathBuf> {
    let value = digest
        .strip_prefix("sha256:")
        .context("only OCI sha256 digests are supported")?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid OCI digest");
    }
    Ok(PathBuf::from(format!("blobs/sha256/{value}")))
}

fn read_compressed_archive_entry(
    source: &FsPath,
    expected: &FsPath,
    max_size: u64,
) -> Result<Option<Vec<u8>>> {
    let decoder = zstd::Decoder::new(File::open(source)?)?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.into_owned();
        ensure_safe_archive_path(&path)?;
        if path == expected {
            let mut bytes = Vec::new();
            entry.take(max_size + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > max_size {
                bail!("archive metadata entry is too large");
            }
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

fn read_plain_archive_entry(
    source: &FsPath,
    expected: &FsPath,
    max_size: u64,
) -> Result<Option<Vec<u8>>> {
    let mut archive = tar::Archive::new(File::open(source)?);
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.into_owned();
        ensure_safe_archive_path(&path)?;
        if path == expected {
            let mut bytes = Vec::new();
            entry.take(max_size + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > max_size {
                bail!("archive metadata entry is too large");
            }
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

fn read_saved_manifest_and_config(path: &FsPath) -> Result<(serde_json::Value, serde_json::Value)> {
    let decoder = zstd::Decoder::new(File::open(path)?)?;
    let mut archive = tar::Archive::new(decoder);
    let mut manifest_bytes = None;
    for entry in archive.entries()? {
        let entry = entry?;
        let entry_path = entry.path()?.into_owned();
        ensure_safe_archive_path(&entry_path)?;
        if entry_path == FsPath::new("manifest.json") {
            let mut bytes = Vec::new();
            entry.take(1024 * 1024).read_to_end(&mut bytes)?;
            manifest_bytes = Some(bytes);
        }
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes.context("Docker save manifest missing")?)?;
    let config_path = manifest
        .as_array()
        .and_then(|images| images.first())
        .and_then(|image| image.get("Config"))
        .and_then(serde_json::Value::as_str)
        .context("Docker image config path missing")?
        .to_owned();
    ensure_safe_archive_path(FsPath::new(&config_path))?;
    let decoder = zstd::Decoder::new(File::open(path)?)?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let entry = entry?;
        if entry.path()?.as_ref() == FsPath::new(&config_path) {
            let mut bytes = Vec::new();
            entry.take(16 * 1024 * 1024).read_to_end(&mut bytes)?;
            return Ok((manifest, serde_json::from_slice(&bytes)?));
        }
    }
    bail!("Docker image config is missing")
}

fn saved_image_config_sha256(path: &FsPath) -> Result<String> {
    let (_, config) = read_saved_manifest_and_config(path)?;
    let config_summary = config
        .get("config")
        .context("Docker image config summary missing")?;
    json_sha256(config_summary)
}

fn sanitize_imported_image_config(config: &mut serde_json::Value, new_ref: &str) -> Result<()> {
    let local_id = new_ref
        .rsplit_once(':')
        .map(|(_, tag)| tag)
        .context("staging tag missing")?;
    Uuid::parse_str(local_id).context("staging tag must be a UUID")?;
    let defaults = config
        .get_mut("config")
        .and_then(serde_json::Value::as_object_mut)
        .context("Docker image defaults missing")?;
    let labels = defaults
        .entry("Labels")
        .or_insert_with(|| serde_json::json!({}));
    if !labels.is_object() {
        *labels = serde_json::json!({});
    }
    let labels = labels
        .as_object_mut()
        .context("Docker image labels invalid")?;
    labels.retain(|key, _| !key.starts_with("com.cue.webtop-manager."));
    labels.insert(
        OWNER_LABEL.into(),
        serde_json::Value::String("managed".into()),
    );
    labels.insert(
        RESOURCE_KIND_LABEL.into(),
        serde_json::Value::String("template".into()),
    );
    labels.insert(
        RESOURCE_ID_LABEL.into(),
        serde_json::Value::String(local_id.into()),
    );
    if let Some(environment) = defaults
        .get_mut("Env")
        .and_then(serde_json::Value::as_array_mut)
    {
        environment.retain(|value| value.as_str().is_some_and(safe_image_environment));
    }
    Ok(())
}

fn safe_image_environment(value: &str) -> bool {
    let key = value.split('=').next().unwrap_or_default();
    let upper = key.to_ascii_uppercase();
    !RESERVED_ENVIRONMENT_KEYS.contains(&key)
        && key != "LC_ALL"
        && key != "SELKIES_MASTER_TOKEN"
        && !key.starts_with("FILE__")
        && !upper.contains("PASSWORD")
        && !upper.contains("TOKEN")
        && !upper.contains("SECRET")
        && !upper.ends_with("_KEY")
}

fn json_sha256(value: &serde_json::Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn ensure_safe_archive_path(path: &FsPath) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("unsafe archive path");
    }
    Ok(())
}

fn append_bytes<W: Write>(archive: &mut tar::Builder<W>, path: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o600);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    archive.append_data(&mut header, path, bytes)?;
    Ok(())
}

fn append_file(archive: &mut tar::Builder<File>, path: &str, source: &FsPath) -> Result<()> {
    let mut file = File::open(source)?;
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o600);
    header.set_size(file.metadata()?.len());
    header.set_cksum();
    archive.append_data(&mut header, path, &mut file)?;
    Ok(())
}

fn hash_file(path: &FsPath) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let bytes = std::io::copy(&mut file, &mut hasher)?;
    Ok((hex::encode(hasher.finalize()), bytes))
}

async fn hash_file_async(path: PathBuf) -> Result<(String, u64)> {
    tokio::task::spawn_blocking(move || hash_file(&path)).await?
}

async fn copy_file_atomic_0600(
    source: PathBuf,
    destination: PathBuf,
    cancellation: watch::Receiver<bool>,
) -> Result<bool> {
    tokio::task::spawn_blocking(move || -> Result<bool> {
        let parent = destination.parent().context("snapshot parent")?;
        let temporary = parent.join(format!(".config.tar.zst.partial-{}", Uuid::new_v4()));
        let result = (|| -> Result<bool> {
            let mut input = File::open(&source)?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                if cancellation_requested(&cancellation) {
                    return Ok(false);
                }
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                output.write_all(&buffer[..read])?;
            }
            output.sync_all()?;
            if cancellation_requested(&cancellation) {
                return Ok(false);
            }
            std::fs::rename(&temporary, &destination)?;
            File::open(parent)?.sync_all()?;
            Ok(true)
        })();
        if !matches!(result, Ok(true)) {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    })
    .await?
}

fn validate_name(name: &str) -> Result<(), ApiFailure> {
    if name.trim().is_empty() || name.len() > 64 {
        Err(ApiFailure::invalid_request())
    } else {
        Ok(())
    }
}

fn safe_filename(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "webtop-template".into()
    } else {
        value.into()
    }
}

fn template_name_conflict() -> ApiFailure {
    ApiFailure(
        StatusCode::CONFLICT,
        ApiError {
            code: ErrorCode::TemplateNameConflict,
            params: BTreeMap::new(),
        },
    )
}

fn package_failure(error: impl std::fmt::Display) -> ApiFailure {
    tracing::warn!(%error, "template package validation failed");
    ApiFailure(StatusCode::BAD_REQUEST, package_error(error))
}

fn package_error(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(%error, "template package rejected");
    ApiError {
        code: ErrorCode::TemplatePackageInvalid,
        params: BTreeMap::new(),
    }
}

fn invalid_error() -> ApiError {
    ApiError {
        code: ErrorCode::InvalidRequest,
        params: BTreeMap::new(),
    }
}
fn busy_error() -> ApiError {
    ApiError {
        code: ErrorCode::ResourceBusy,
        params: BTreeMap::new(),
    }
}
fn internal_error(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "template internal error");
    ApiError {
        code: ErrorCode::Internal,
        params: BTreeMap::new(),
    }
}
fn docker_error(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(%error, "template Docker operation failed");
    ApiError {
        code: ErrorCode::DockerUnavailable,
        params: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn commit_config_strips_instance_secrets_and_keeps_boot_metadata() {
        let source = ImageConfig {
            entrypoint: Some(vec!["/init".into()]),
            env: Some(vec![
                "PATH=/bin".into(),
                "PASSWORD=nope".into(),
                "API_TOKEN=nope".into(),
            ]),
            volumes: Some(vec!["/data".into()]),
            ..Default::default()
        };
        let config = sanitized_commit_config(source, Uuid::nil(), Uuid::nil());
        assert_eq!(config.entrypoint, Some(vec!["/init".into()]));
        assert_eq!(config.env, Some(vec!["PATH=/bin".into()]));
        assert!(config.volumes.unwrap().contains(&"/config".into()));
    }

    #[test]
    fn rejects_archive_traversal() {
        assert!(ensure_safe_archive_path(FsPath::new("../../etc/passwd")).is_err());
        assert!(ensure_safe_archive_path(FsPath::new("layers/sha256.tar")).is_ok());
    }

    #[tokio::test]
    async fn snapshot_copy_is_atomic_0600_and_honors_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.tar.zst");
        let destination = directory.path().join("snapshot/config.tar.zst");
        std::fs::create_dir(destination.parent().unwrap()).unwrap();
        let contents = vec![0x4f; 2 * 1024 * 1024];
        std::fs::write(&source, &contents).unwrap();
        let (_sender, cancellation) = watch::channel(false);

        assert!(
            copy_file_atomic_0600(source.clone(), destination.clone(), cancellation)
                .await
                .unwrap()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), contents);
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        std::fs::remove_file(&destination).unwrap();
        let (sender, cancellation) = watch::channel(false);
        sender.send(true).unwrap();
        assert!(
            !copy_file_atomic_0600(source, destination.clone(), cancellation)
                .await
                .unwrap()
        );
        assert!(!destination.exists());
        assert!(std::fs::read_dir(destination.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("partial")));
    }

    #[test]
    fn imported_config_cannot_inject_ownership_or_secrets() {
        let local_id = Uuid::new_v4();
        let mut config = serde_json::json!({
            "config": {
                "Labels": {
                    "com.cue.webtop-manager.owner": "attacker",
                    "com.cue.webtop-manager.resource-kind": "environment",
                    "upstream.label": "kept"
                },
                "Env": ["PATH=/bin", "PASSWORD=secret", "FILE__TOKEN=/tmp/token"]
            }
        });
        sanitize_imported_image_config(&mut config, &format!("{STAGING_REPOSITORY}:{local_id}"))
            .unwrap();
        let defaults = &config["config"];
        assert_eq!(defaults["Labels"][OWNER_LABEL], "managed");
        assert_eq!(defaults["Labels"][RESOURCE_KIND_LABEL], "template");
        assert_eq!(defaults["Labels"][RESOURCE_ID_LABEL], local_id.to_string());
        assert_eq!(defaults["Labels"]["upstream.label"], "kept");
        assert_eq!(defaults["Env"], serde_json::json!(["PATH=/bin"]));
    }

    #[tokio::test]
    async fn rewrites_legacy_and_oci_image_references_together() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("image.tar.zst");
        let destination = directory.path().join("image.load.tar");
        let old_ref = format!("{TEMPLATE_REPOSITORY}:{}", Uuid::new_v4());
        let local_id = Uuid::new_v4();
        let new_ref = format!("{STAGING_REPOSITORY}:{local_id}");
        let config = serde_json::json!({
            "architecture": "amd64",
            "os": "linux",
            "config": {
                "Entrypoint": ["/init"],
                "Volumes": {"/config": {}},
                "Env": ["PATH=/bin", "PASSWORD=secret"],
                "Labels": {"com.cue.webtop-manager.owner": "managed"}
            }
        });
        let config_bytes = serde_json::to_vec(&config).unwrap();
        let config_digest = hex::encode(Sha256::digest(&config_bytes));
        let oci_manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": format!("sha256:{config_digest}"),
                "size": config_bytes.len()
            },
            "layers": []
        });
        let oci_manifest_bytes = serde_json::to_vec(&oci_manifest).unwrap();
        let oci_manifest_digest = hex::encode(Sha256::digest(&oci_manifest_bytes));
        let index = serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": format!("sha256:{oci_manifest_digest}"),
                "size": oci_manifest_bytes.len(),
                "annotations": {"io.containerd.image.name": old_ref}
            }]
        });
        let legacy_manifest = serde_json::json!([{
            "Config": format!("blobs/sha256/{config_digest}"),
            "RepoTags": [old_ref],
            "Layers": []
        }]);
        {
            let output = File::create(&source).unwrap();
            let encoder = zstd::Encoder::new(output, 1).unwrap();
            let mut archive = tar::Builder::new(encoder);
            append_bytes(
                &mut archive,
                &format!("blobs/sha256/{config_digest}"),
                &config_bytes,
            )
            .unwrap();
            append_bytes(
                &mut archive,
                &format!("blobs/sha256/{oci_manifest_digest}"),
                &oci_manifest_bytes,
            )
            .unwrap();
            append_bytes(
                &mut archive,
                "index.json",
                &serde_json::to_vec(&index).unwrap(),
            )
            .unwrap();
            append_bytes(
                &mut archive,
                "manifest.json",
                &serde_json::to_vec(&legacy_manifest).unwrap(),
            )
            .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        assert_eq!(
            saved_image_config_sha256(&source).unwrap(),
            json_sha256(&config["config"]).unwrap()
        );

        rewrite_image_archive(source, destination.clone(), &old_ref, &new_ref)
            .await
            .unwrap();
        validate_rewritten_image_archive(&destination, &new_ref, local_id)
            .await
            .unwrap();
        let index_bytes =
            read_plain_archive_entry(&destination, FsPath::new("index.json"), 1024 * 1024)
                .unwrap()
                .unwrap();
        let index: serde_json::Value = serde_json::from_slice(&index_bytes).unwrap();
        assert_eq!(
            index["manifests"][0]["annotations"]["io.containerd.image.name"],
            new_ref
        );
    }
}
