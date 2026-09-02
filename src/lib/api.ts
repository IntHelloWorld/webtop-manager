import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  BootStatus,
  EnvironmentCredentials,
  EnvironmentRecord,
  EnvironmentSpec,
  FrpcServiceStatus,
  FrpcTestResult,
  FrpsSetupGuide,
  ImageCachePruneResult,
  ImagePullProgress,
  ImagePullResult,
  OfficialImage,
  ServerSettings,
  PersistentOperation,
  TemplateImportPreflight,
  TemplatePreflight,
  TemplateRecord,
  TemplateTransferProgress,
} from "./types";

export async function initializeBackend(): Promise<BootStatus> {
  const status = await invoke<BootStatus>("docker_diagnostics");
  if (status.state === "controller_unavailable") {
    return invoke<BootStatus>("bootstrap_controller");
  }
  return status;
}

export function listEnvironments(): Promise<EnvironmentRecord[]> {
  return invoke("list_environments");
}

export function listTemplates(): Promise<TemplateRecord[]> {
  return invoke("list_templates");
}

export function getTemplatePreflight(id: string): Promise<TemplatePreflight> {
  return invoke("template_preflight", { id });
}

export function createTemplate(requestBody: {
  environmentId: string;
  name: string;
  confirmedSensitiveData: boolean;
  confirmedSpaceWarning: boolean;
}): Promise<PersistentOperation> {
  return invoke("create_template", { requestBody });
}

export function createEnvironmentFromTemplate(id: string, spec: EnvironmentSpec): Promise<PersistentOperation> {
  return invoke("create_environment_from_template", { id, spec });
}

export function checkTemplateSources(templateIds: string[] = []): Promise<PersistentOperation> {
  return invoke("check_template_sources", { templateIds });
}

export function startTemplateExport(id: string): Promise<PersistentOperation> {
  return invoke("export_template", { id });
}

export function getOperation(id: string): Promise<PersistentOperation> {
  return invoke("get_operation", { id });
}

export function cancelOperation(id: string): Promise<PersistentOperation> {
  return invoke("cancel_operation", { id });
}

export function selectTemplateImport(): Promise<string | null> {
  return invoke("select_template_import");
}

export function stageTemplateImport(
  sourceId: string,
  transferId: string,
  onProgress: (progress: TemplateTransferProgress) => void,
): Promise<string | null> {
  const channel = new Channel<TemplateTransferProgress>();
  channel.onmessage = onProgress;
  return invoke("stage_template_import", { sourceId, transferId, onProgress: channel });
}

export function selectTemplateExport(suggestedName: string): Promise<string | null> {
  return invoke("select_template_export", { suggestedName });
}

export function getTemplateImportPreflight(stagingFileId: string): Promise<TemplateImportPreflight> {
  return invoke("import_template_preflight", { stagingFileId });
}

export function startTemplateImport(requestBody: {
  stagingFileId: string;
  name: string;
  confirmedSensitiveData: boolean;
  confirmedUntrustedImage: boolean;
}): Promise<PersistentOperation> {
  return invoke("import_template", { requestBody });
}

export function removeTemplate(id: string, confirmationName: string): Promise<PersistentOperation> {
  return invoke("delete_template", { id, confirmationName });
}

export function saveTemplateExport(
  stagingFileId: string,
  destinationId: string,
  transferId: string,
  onProgress: (progress: TemplateTransferProgress) => void,
): Promise<boolean> {
  const channel = new Channel<TemplateTransferProgress>();
  channel.onmessage = onProgress;
  return invoke("save_template_export", { stagingFileId, destinationId, transferId, onProgress: channel });
}

export function cancelTemplateTransfer(transferId: string): Promise<void> {
  return invoke("cancel_template_transfer", { transferId });
}

export function discardTemplateExportDestination(destinationId: string): Promise<void> {
  return invoke("discard_template_export_destination", { destinationId });
}

export function discardTemplateStaging(stagingFileId: string): Promise<void> {
  return invoke("discard_template_staging", { stagingFileId });
}

export async function listOfficialImages(): Promise<OfficialImage[]> {
  const images = await invoke<OfficialImage[]>("list_official_images");
  return [...images].sort((left, right) => Number(right.installed) - Number(left.installed));
}

export function deleteOfficialImage(reference: string): Promise<void> {
  return invoke("delete_official_image", { reference });
}

export function pullOfficialImage(
  reference: string,
  pullId: string,
  onProgress: (progress: ImagePullProgress) => void,
): Promise<ImagePullResult> {
  const channel = new Channel<ImagePullProgress>();
  channel.onmessage = onProgress;
  return invoke("pull_official_image", { reference, pullId, onProgress: channel });
}

export function cancelOfficialImagePull(pullId: string): Promise<void> {
  return invoke("cancel_official_image_pull", { pullId });
}

export function pruneImageCache(): Promise<ImageCachePruneResult> {
  return invoke("prune_image_cache");
}

export function getServerSettings(): Promise<ServerSettings> {
  return invoke("get_server_settings");
}

export function saveServerSettings(
  settings: ServerSettings,
): Promise<ServerSettings> {
  return invoke("save_server_settings", {
    requestBody: { settings },
  });
}

export function regenerateServerToken(): Promise<ServerSettings> {
  return invoke("regenerate_server_token");
}

export function getFrpsSetupGuide(): Promise<FrpsSetupGuide> {
  return invoke("get_frps_setup_guide");
}

export function getFrpcStatus(): Promise<FrpcServiceStatus> {
  return invoke("get_frpc_status");
}

export function runFrpcAction(action: "start" | "stop" | "restart"): Promise<FrpcServiceStatus> {
  return invoke("frpc_action", { action });
}

export function testFrpcConnectivity(): Promise<FrpcTestResult> {
  return invoke("test_frpc_connectivity");
}

export function createEnvironment(spec: EnvironmentSpec): Promise<EnvironmentRecord> {
  return invoke("create_environment", { spec });
}

export function runEnvironmentAction(
  id: string,
  action: "start" | "stop" | "restart",
): Promise<void> {
  return invoke("environment_action", { id, action });
}

export function runEnvironmentPublicationAction(
  id: string,
  action: "publish" | "unpublish",
): Promise<EnvironmentRecord> {
  return invoke("environment_publication_action", { id, action });
}

export function getEnvironmentCredentials(id: string): Promise<EnvironmentCredentials> {
  return invoke("get_environment_credentials", { id });
}

export function removeEnvironment(
  id: string,
  confirmationName: string,
  deleteData: boolean,
): Promise<void> {
  return invoke("delete_environment", {
    id,
    requestBody: { confirmationName, deleteData },
  });
}

export function openLocalEnvironment(localPort: number): Promise<void> {
  return invoke("open_local_environment", { localPort });
}

export function openEnvironmentDataDirectory(id: string): Promise<void> {
  return invoke("open_environment_data_directory", { id });
}

export function openPublicEnvironment(id: string): Promise<void> {
  return invoke("open_public_environment", { id });
}
