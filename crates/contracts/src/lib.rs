//! Versioned, secret-free contracts shared by the desktop, controller and workers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const API_VERSION: &str = "v1";
pub const OWNER_LABEL: &str = "com.cue.webtop-manager.owner";
pub const RESOURCE_ID_LABEL: &str = "com.cue.webtop-manager.resource-id";
pub const RESOURCE_KIND_LABEL: &str = "com.cue.webtop-manager.resource-kind";
pub const DEFAULT_WEBTOP_IMAGE: &str = "lscr.io/linuxserver/webtop:ubuntu-mate";
pub const DEFAULT_FRPC_IMAGE: &str = "ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22";
pub const DEFAULT_FRPS_IMAGE: &str = "ghcr.io/fatedier/frps:v0.70.1@sha256:dab4febe235a24ddda5c20b1971ce34a31dc9f33983db3b126d278b932650408";
pub const WEBTOP_HTTPS_PORT: u16 = 3001;
pub const DEFAULT_SHM_BYTES: i64 = 1024 * 1024 * 1024;

pub const RESERVED_ENVIRONMENT_KEYS: &[&str] = &[
    "CUSTOM_USER",
    "PASSWORD",
    "FILE__PASSWORD",
    "PUID",
    "PGID",
    "TZ",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSpec {
    pub name: String,
    pub image: String,
    pub identity: IdentitySpec,
    pub resources: ResourceSpec,
    pub display: DisplaySpec,
    pub mounts: Vec<MountSpec>,
    pub security: SecuritySpec,
    #[serde(default)]
    pub extra_environment: BTreeMap<String, String>,
    pub publication: PublicationSpec,
}

impl EnvironmentSpec {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.name.trim().is_empty() || self.name.len() > 64 {
            return Err(ContractError::InvalidName);
        }
        if self.image.trim().is_empty() || self.image.chars().any(char::is_whitespace) {
            return Err(ContractError::InvalidImageReference);
        }
        if self.resources.shm_bytes < 64 * 1024 * 1024 {
            return Err(ContractError::ShmTooSmall);
        }
        if self.display.width.is_some() != self.display.height.is_some() {
            return Err(ContractError::InvalidDisplayResolution);
        }
        if self.publication.enabled {
            if !self.publication.automatic_port && self.publication.remote_port.is_none() {
                return Err(ContractError::InvalidEnvironmentPort(
                    "publication.remotePort".into(),
                ));
            }
            if self.publication.remote_port == Some(0) {
                return Err(ContractError::InvalidEnvironmentPort(
                    "publication.remotePort".into(),
                ));
            }
        } else if self.publication.remote_port.is_some() {
            return Err(ContractError::InvalidEnvironmentPort(
                "publication.remotePort".into(),
            ));
        }
        for key in self.extra_environment.keys() {
            let upper = key.to_ascii_uppercase();
            if RESERVED_ENVIRONMENT_KEYS.contains(&key.as_str())
                || key.starts_with("FILE__")
                || upper.contains("PASSWORD")
                || upper.contains("TOKEN")
                || upper.contains("SECRET")
                || upper.ends_with("_KEY")
            {
                return Err(ContractError::ReservedEnvironmentKey(key.clone()));
            }
        }
        for port_key in [
            "CUSTOM_PORT",
            "CUSTOM_HTTPS_PORT",
            "CUSTOM_WS_PORT",
            "PIXELFLUX_CU",
            "SELKIES_CONTROL_PORT",
        ] {
            if let Some(value) = self.extra_environment.get(port_key) {
                if value.parse::<u16>().ok().filter(|port| *port > 0).is_none() {
                    return Err(ContractError::InvalidEnvironmentPort(port_key.into()));
                }
            }
        }
        let mut targets = BTreeSet::new();
        for mount in &self.mounts {
            if !mount.host_path.is_absolute() {
                return Err(ContractError::InvalidHostPath(mount.host_path.clone()));
            }
            if mount.host_path == Path::new("/var/run/docker.sock") {
                return Err(ContractError::ReservedHostPath(mount.host_path.clone()));
            }
            if mount.container_path == Path::new("/config")
                || mount.container_path.starts_with("/run/webtop-manager")
            {
                return Err(ContractError::ReservedMountTarget(
                    mount.container_path.clone(),
                ));
            }
            if !mount.container_path.is_absolute() || !targets.insert(&mount.container_path) {
                return Err(ContractError::InvalidMountTarget(
                    mount.container_path.clone(),
                ));
            }
        }
        if self.security.privileged {
            return Err(ContractError::PrivilegedForbidden);
        }
        if self
            .security
            .devices
            .iter()
            .any(|device| !device.is_absolute() || !device.starts_with("/dev"))
        {
            return Err(ContractError::InvalidDevicePath);
        }
        Ok(())
    }
}

impl Default for EnvironmentSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            image: DEFAULT_WEBTOP_IMAGE.into(),
            identity: IdentitySpec::default(),
            resources: ResourceSpec::default(),
            display: DisplaySpec::default(),
            mounts: Vec::new(),
            security: SecuritySpec::default(),
            extra_environment: BTreeMap::new(),
            publication: PublicationSpec::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySpec {
    pub uid: u32,
    pub gid: u32,
    pub timezone: String,
    pub locale: String,
}

impl Default for IdentitySpec {
    fn default() -> Self {
        Self {
            uid: 1000,
            gid: 1000,
            timezone: "Etc/UTC".into(),
            locale: "zh_CN.UTF-8".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    pub cpu_limit: Option<f64>,
    pub memory_bytes: Option<i64>,
    pub shm_bytes: i64,
}

impl Eq for ResourceSpec {}

impl Default for ResourceSpec {
    fn default() -> Self {
        Self {
            cpu_limit: None,
            memory_bytes: None,
            shm_bytes: DEFAULT_SHM_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySpec {
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub wayland: Option<bool>,
    pub gpu: GpuMode,
    pub audio: bool,
    pub clipboard: bool,
    pub file_transfer: bool,
    #[serde(default)]
    pub file_transfer_mode: Option<FileTransferMode>,
}

impl Default for DisplaySpec {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            wayland: None,
            gpu: GpuMode::Disabled,
            audio: true,
            clipboard: true,
            file_transfer: true,
            file_transfer_mode: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferMode {
    UploadDownload,
    Upload,
    Download,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    Disabled,
    Dri,
    Nvidia,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MountSpec {
    pub host_path: PathBuf,
    pub container_path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySpec {
    pub docker_socket: bool,
    pub docker_socket_gid: Option<u32>,
    pub privileged: bool,
    pub seccomp: SeccompMode,
    #[serde(default)]
    pub devices: Vec<PathBuf>,
}

impl Default for SecuritySpec {
    fn default() -> Self {
        Self {
            docker_socket: false,
            docker_socket_gid: None,
            privileged: false,
            seccomp: SeccompMode::Default,
            devices: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeccompMode {
    Default,
    Unconfined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PublicationSpec {
    pub enabled: bool,
    pub remote_port: Option<u16>,
    pub automatic_port: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub frps_host: String,
    pub frps_port: u16,
    pub public_ip: String,
    pub remote_port_start: u16,
    pub remote_port_end: u16,
    pub token_configured: bool,
    #[serde(default)]
    pub token_state: ServerTokenState,
    pub frpc_image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServerTokenState {
    Ready,
    #[default]
    Missing,
    RecoveryPending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrpcServiceState {
    NotCreated,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrpcServiceStatus {
    pub state: FrpcServiceState,
    pub connected: bool,
    pub image: Option<String>,
    pub started_at: Option<String>,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrpcTestCode {
    Connected,
    AuthenticationFailed,
    DnsFailed,
    ConnectionRefused,
    TimedOut,
    ClientExited,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrpcTestResult {
    pub success: bool,
    pub code: FrpcTestCode,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            frps_host: String::new(),
            frps_port: 7000,
            public_ip: String::new(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: false,
            token_state: ServerTokenState::Missing,
            frpc_image: DEFAULT_FRPC_IMAGE.into(),
        }
    }
}

impl ServerSettings {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.frps_host.trim().is_empty()
            || self.public_ip.trim().is_empty()
            || self.frps_port == 0
        {
            return Err(ContractError::InvalidServerAddress);
        }
        if self.remote_port_start == 0 || self.remote_port_start > self.remote_port_end {
            return Err(ContractError::InvalidPortRange);
        }
        if self.frpc_image.trim().is_empty() || self.frpc_image.chars().any(char::is_whitespace) {
            return Err(ContractError::InvalidImageReference);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialImage {
    pub reference: String,
    pub tag: String,
    pub distribution: String,
    pub desktop: String,
    pub wayland_support: bool,
    pub wayland_only: bool,
    pub installed: bool,
    pub image_id: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImagePullPhase {
    Starting,
    Progress,
    Complete,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImagePullProgress {
    pub pull_id: Uuid,
    pub reference: String,
    pub phase: ImagePullPhase,
    pub layer_id: Option<String>,
    pub status: String,
    pub current_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
    pub aggregate_current_bytes: Option<i64>,
    pub aggregate_total_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageCachePruneResult {
    pub deleted_items: usize,
    pub space_reclaimed_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub image_reference: String,
    pub image_id: String,
    pub platform: String,
    pub system_size_bytes: u64,
    pub system_delta_bytes: u64,
    pub snapshot_path: PathBuf,
    pub snapshot_sha256: String,
    pub snapshot_size_bytes: u64,
    pub snapshot_original_bytes: u64,
    pub source_environment_id: Option<Uuid>,
    pub parent_template_id: Option<Uuid>,
    #[serde(default)]
    pub external_lineage: Vec<Uuid>,
    pub source_spec: EnvironmentSpec,
    pub official_source: Option<OfficialTemplateSource>,
    pub source_check: TemplateSourceCheck,
    pub integrity: TemplateIntegrity,
    pub trust: TemplateTrust,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialTemplateSource {
    pub reference: String,
    pub digest: Option<String>,
    pub image_id: String,
    pub build_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSourceStatus {
    NotChecked,
    Current,
    Updated,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSourceCheck {
    pub status: TemplateSourceStatus,
    pub checked_at: Option<DateTime<Utc>>,
    pub current_digest: Option<String>,
}

impl Default for TemplateSourceCheck {
    fn default() -> Self {
        Self {
            status: TemplateSourceStatus::NotChecked,
            checked_at: None,
            current_digest: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateIntegrity {
    Complete,
    MissingImage,
    MissingSnapshot,
    CorruptSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateTrust {
    Local,
    ImportedUntrusted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePreflight {
    pub environment_id: Uuid,
    pub system_change_bytes: u64,
    pub config_original_bytes: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub symlink_count: u64,
    pub skipped_special_files: u64,
    pub sensitive_paths: Vec<String>,
    pub image_upper_bound_bytes: u64,
    pub snapshot_upper_bound_bytes: u64,
    pub conservative_total_bytes: u64,
    pub available_bytes: Option<u64>,
    pub insufficient_space_warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateManifest {
    pub schema_version: u32,
    pub exported_template_id: Uuid,
    pub name: String,
    pub platform: String,
    pub image_reference: String,
    pub image_id: String,
    pub image_config_sha256: String,
    pub source_spec: EnvironmentSpec,
    pub official_source: Option<OfficialTemplateSource>,
    pub lineage: Vec<Uuid>,
    pub image_payload: TemplatePayload,
    pub config_payload: TemplatePayload,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePayload {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateImportPreflight {
    pub staging_file_id: Uuid,
    pub manifest: TemplateManifest,
    pub name_conflict: bool,
    pub sensitive_data_warning: bool,
    pub untrusted_image_warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: Uuid,
    pub kind: OperationKind,
    pub phase: OperationPhase,
    pub progress_percent: Option<u8>,
    pub cancellable: bool,
    pub resource_id: Option<Uuid>,
    pub error: Option<ApiError>,
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub log_lines: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    PullImage,
    CreateEnvironment,
    RebuildEnvironment,
    CreateTemplate,
    RestoreTemplate,
    ExportTemplate,
    ImportTemplate,
    CheckTemplateSource,
    DeleteTemplate,
    DeleteEnvironment,
    Reconcile,
    UpgradeController,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    Queued,
    Preflight,
    Running,
    Verifying,
    RollingBack,
    Succeeded,
    Failed,
    Cancelled,
    Retryable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: ErrorCode,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    DockerSocketMissing,
    DockerPermissionDenied,
    DockerUnavailable,
    ControllerUnavailable,
    ControllerImageMissing,
    InvalidRequest,
    ReservedField,
    ResourceBusy,
    PortConflict,
    PathOutsideDataRoot,
    InsufficientDiskSpace,
    ImageInUse,
    SnapshotCorrupt,
    TemplateDependency,
    TemplateImageMissing,
    TemplatePackageInvalid,
    TemplateNameConflict,
    OperationNotCancellable,
    FrpTokenRecoveryRequired,
    Internal,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("invalid environment name")]
    InvalidName,
    #[error("invalid image reference")]
    InvalidImageReference,
    #[error("shm size is too small")]
    ShmTooSmall,
    #[error("display width and height must both be set or both be automatic")]
    InvalidDisplayResolution,
    #[error("reserved environment key: {0}")]
    ReservedEnvironmentKey(String),
    #[error("invalid environment port: {0}")]
    InvalidEnvironmentPort(String),
    #[error("reserved mount target: {0:?}")]
    ReservedMountTarget(PathBuf),
    #[error("invalid mount target: {0:?}")]
    InvalidMountTarget(PathBuf),
    #[error("invalid host path: {0:?}")]
    InvalidHostPath(PathBuf),
    #[error("reserved host path: {0:?}")]
    ReservedHostPath(PathBuf),
    #[error("invalid device path")]
    InvalidDevicePath,
    #[error("privileged containers are forbidden")]
    PrivilegedForbidden,
    #[error("invalid server address")]
    InvalidServerAddress,
    #[error("invalid port range")]
    InvalidPortRange,
}

/// Validates a destructive target after both paths have been canonicalized.
pub fn require_contained_path(root: &Path, target: &Path) -> Result<(), ApiError> {
    if target == root || !target.starts_with(root) {
        return Err(ApiError {
            code: ErrorCode::PathOutsideDataRoot,
            params: BTreeMap::new(),
        });
    }
    Ok(())
}

/// Picks the first unallocated remote port without probing or changing frps.
pub fn allocate_remote_port(
    start: u16,
    end: u16,
    allocated: &BTreeSet<u16>,
) -> Result<u16, ApiError> {
    (start..=end)
        .find(|port| !allocated.contains(port))
        .ok_or_else(|| ApiError {
            code: ErrorCode::PortConflict,
            params: BTreeMap::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reserved_secret_fields() {
        let mut spec = EnvironmentSpec::default();
        spec.name = "desk".into();
        spec.extra_environment
            .insert("FILE__PASSWORD".into(), "/tmp/nope".into());
        assert!(matches!(
            spec.validate(),
            Err(ContractError::ReservedEnvironmentKey(_))
        ));
    }

    #[test]
    fn rejects_config_override_and_privileged_mode() {
        let mut spec = EnvironmentSpec::default();
        spec.name = "desk".into();
        spec.mounts.push(MountSpec {
            host_path: "/tmp/data".into(),
            container_path: "/config".into(),
            read_only: false,
        });
        assert!(matches!(
            spec.validate(),
            Err(ContractError::ReservedMountTarget(_))
        ));
        spec.mounts.clear();
        spec.security.privileged = true;
        assert_eq!(spec.validate(), Err(ContractError::PrivilegedForbidden));
    }

    #[test]
    fn validates_automatic_display_and_custom_web_ports() {
        let mut spec = EnvironmentSpec::default();
        spec.name = "desk".into();
        assert!(spec.validate().is_ok());

        spec.display.width = Some(1920);
        assert_eq!(
            spec.validate(),
            Err(ContractError::InvalidDisplayResolution)
        );
        spec.display.height = Some(1080);
        spec.extra_environment
            .insert("CUSTOM_HTTPS_PORT".into(), "70000".into());
        assert!(matches!(
            spec.validate(),
            Err(ContractError::InvalidEnvironmentPort(_))
        ));
        spec.extra_environment
            .insert("CUSTOM_HTTPS_PORT".into(), "3443".into());
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn publication_requires_explicit_enablement_and_a_valid_manual_port() {
        let mut spec = EnvironmentSpec::default();
        spec.name = "desk".into();
        spec.publication.remote_port = Some(41000);
        assert!(spec.validate().is_err());

        spec.publication.enabled = true;
        spec.publication.automatic_port = false;
        spec.publication.remote_port = None;
        assert!(spec.validate().is_err());

        spec.publication.remote_port = Some(41000);
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn image_pull_progress_serializes_for_the_frontend_channel() {
        let progress = ImagePullProgress {
            pull_id: Uuid::nil(),
            reference: "lscr.io/linuxserver/webtop:ubuntu-mate".into(),
            phase: ImagePullPhase::Progress,
            layer_id: Some("layer".into()),
            status: "Downloading".into(),
            current_bytes: Some(50),
            total_bytes: Some(100),
            aggregate_current_bytes: Some(150),
            aggregate_total_bytes: Some(300),
        };
        let value = serde_json::to_value(progress).unwrap();
        assert_eq!(value["pullId"], Uuid::nil().to_string());
        assert_eq!(value["phase"], "progress");
        assert_eq!(value["aggregateCurrentBytes"], 150);
    }

    #[test]
    fn destructive_path_must_be_below_root() {
        assert!(require_contained_path(
            Path::new("/data/environments"),
            Path::new("/data/environments/abc/config")
        )
        .is_ok());
        assert!(
            require_contained_path(Path::new("/data/environments"), Path::new("/data/other"))
                .is_err()
        );
        assert!(require_contained_path(
            Path::new("/data/environments"),
            Path::new("/data/environments")
        )
        .is_err());
    }

    #[test]
    fn allocation_skips_recorded_ports() {
        let ports = BTreeSet::from([41000, 41001]);
        assert_eq!(allocate_remote_port(41000, 41003, &ports).unwrap(), 41002);
    }
}
