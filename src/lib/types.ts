export type BootState =
  | "docker_missing"
  | "permission_denied"
  | "docker_unavailable"
  | "controller_unavailable"
  | "ready";

export interface BootStatus {
  state: BootState;
  dockerVersion: string | null;
  dockerApiVersion: string | null;
  socketMode: number | null;
  socketWorldWritable: boolean;
  controllerVersion: string | null;
  hostUid: number;
  hostGid: number;
}

export interface ApiError {
  code: string;
  params?: Record<string, string>;
}

export interface OfficialImage {
  reference: string;
  tag: string;
  distribution: string;
  desktop: string;
  waylandSupport: boolean;
  waylandOnly: boolean;
  installed: boolean;
  imageId: string | null;
  sizeBytes: number | null;
}

export type ImagePullPhase = "starting" | "progress" | "complete" | "cancelled" | "error";

export interface ImagePullProgress {
  pullId: string;
  reference: string;
  phase: ImagePullPhase;
  layerId: string | null;
  status: string;
  currentBytes: number | null;
  totalBytes: number | null;
  aggregateCurrentBytes: number | null;
  aggregateTotalBytes: number | null;
}

export interface ImagePullResult {
  cancelled: boolean;
}

export interface ImageCachePruneResult {
  deletedItems: number;
  spaceReclaimedBytes: number;
}

export interface ServerSettings {
  frpsHost: string;
  frpsPort: number;
  publicIp: string;
  remotePortStart: number;
  remotePortEnd: number;
  tokenConfigured: boolean;
  tokenState: "ready" | "missing" | "recovery_pending";
  frpcImage: string;
}

export type FrpcServiceState = "not_created" | "running" | "stopped" | "error";

export interface FrpcServiceStatus {
  state: FrpcServiceState;
  connected: boolean;
  image: string | null;
  startedAt: string | null;
  exitCode: number | null;
}

export type FrpcTestCode =
  | "connected"
  | "authentication_failed"
  | "dns_failed"
  | "connection_refused"
  | "timed_out"
  | "client_exited"
  | "unknown";

export interface FrpcTestResult {
  success: boolean;
  code: FrpcTestCode;
}

export interface FrpsSetupGuide {
  dockerSetupScript: string;
  nativeSetupScript: string;
  publicAddress: string;
  bindPort: number;
  remotePortStart: number;
  remotePortEnd: number;
}

export interface EnvironmentRecord {
  id: string;
  name: string;
  containerId: string;
  configPath: string;
  desiredRunning: boolean;
  localPort: number | null;
  templateId: string | null;
  spec: EnvironmentSpec;
  createdAt: string;
}

export type TemplateIntegrity = "complete" | "missing_image" | "missing_snapshot" | "corrupt_snapshot";
export type TemplateTrust = "local" | "imported_untrusted";
export type TemplateSourceStatus = "not_checked" | "current" | "updated" | "unavailable";

export interface TemplateRecord {
  id: string;
  name: string;
  imageReference: string;
  imageId: string;
  platform: string;
  systemSizeBytes: number;
  systemDeltaBytes: number;
  snapshotPath: string;
  snapshotSha256: string;
  snapshotSizeBytes: number;
  snapshotOriginalBytes: number;
  sourceEnvironmentId: string | null;
  parentTemplateId: string | null;
  externalLineage: string[];
  sourceSpec: EnvironmentSpec;
  officialSource: {
    reference: string;
    digest: string | null;
    imageId: string;
    buildVersion: string | null;
  } | null;
  sourceCheck: {
    status: TemplateSourceStatus;
    checkedAt: string | null;
    currentDigest: string | null;
  };
  integrity: TemplateIntegrity;
  trust: TemplateTrust;
  createdAt: string;
}

export interface TemplatePreflight {
  environmentId: string;
  systemChangeBytes: number;
  configOriginalBytes: number;
  fileCount: number;
  directoryCount: number;
  symlinkCount: number;
  skippedSpecialFiles: number;
  sensitivePaths: string[];
  imageUpperBoundBytes: number;
  snapshotUpperBoundBytes: number;
  conservativeTotalBytes: number;
  availableBytes: number | null;
  insufficientSpaceWarning: boolean;
}

export type OperationPhase = "queued" | "preflight" | "running" | "verifying" | "rolling_back" | "succeeded" | "failed" | "cancelled" | "retryable";
export interface PersistentOperation {
  id: string;
  kind: string;
  phase: OperationPhase;
  progressPercent: number | null;
  cancellable: boolean;
  resourceId: string | null;
  error: ApiError | null;
  result: Record<string, unknown> | null;
  logLines: string[];
  createdAt: string;
  updatedAt: string;
}

export interface TemplateTransferProgress {
  phase: "dialog" | "copying" | "complete" | "cancelled";
  message: string;
  currentBytes: number;
  totalBytes: number;
}

export interface TemplateManifest {
  schemaVersion: number;
  exportedTemplateId: string;
  name: string;
  platform: string;
  imageReference: string;
  imageId: string;
  sourceSpec: EnvironmentSpec;
  lineage: string[];
  imagePayload: { path: string; sizeBytes: number; sha256: string };
  configPayload: { path: string; sizeBytes: number; sha256: string };
  createdAt: string;
}

export interface TemplateImportPreflight {
  stagingFileId: string;
  manifest: TemplateManifest;
  nameConflict: boolean;
  sensitiveDataWarning: boolean;
  untrustedImageWarning: boolean;
}

export interface EnvironmentCredentials {
  username: string;
  password: string;
}

export interface EnvironmentSpec {
  name: string;
  image: string;
  identity: {
    uid: number;
    gid: number;
    timezone: string;
    locale: string;
  };
  resources: {
    cpuLimit: number | null;
    memoryBytes: number | null;
    shmBytes: number;
  };
  display: {
    width: number | null;
    height: number | null;
    wayland: boolean | null;
    gpu: "disabled" | "dri" | "nvidia";
    audio: boolean;
    clipboard: boolean;
    fileTransfer: boolean;
    fileTransferMode: "upload_download" | "upload" | "download" | "none" | null;
  };
  mounts: Array<{
    hostPath: string;
    containerPath: string;
    readOnly: boolean;
  }>;
  security: {
    dockerSocket: boolean;
    dockerSocketGid: number | null;
    privileged: false;
    seccomp: "default" | "unconfined";
    devices: string[];
  };
  extraEnvironment: Record<string, string>;
  publication: {
    enabled: boolean;
    remotePort: number | null;
    automaticPort: boolean;
  };
}
