// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../lib/i18n";
import { TemplatesPage } from "./TemplatesPage";

const api = vi.hoisted(() => ({
  cancelOperation: vi.fn(), cancelTemplateTransfer: vi.fn(), checkTemplateSources: vi.fn(), discardTemplateExportDestination: vi.fn(), discardTemplateStaging: vi.fn(),
  getOperation: vi.fn(), getServerSettings: vi.fn(), getTemplateImportPreflight: vi.fn(), listTemplates: vi.fn(),
  removeTemplate: vi.fn(), saveTemplateExport: vi.fn(), selectTemplateExport: vi.fn(), selectTemplateImport: vi.fn(), stageTemplateImport: vi.fn(), startTemplateExport: vi.fn(), startTemplateImport: vi.fn(),
}));
vi.mock("../../lib/api", () => api);

const sourceSpec = {
  name: "source", image: "lscr.io/linuxserver/webtop:ubuntu-mate",
  identity: { uid: 1000, gid: 1000, timezone: "Asia/Shanghai", locale: "zh_CN.UTF-8" },
  resources: { cpuLimit: 2, memoryBytes: 4294967296, shmBytes: 1073741824 },
  display: { width: 1920, height: 1080, wayland: false, gpu: "disabled", audio: true, clipboard: true, fileTransfer: true, fileTransferMode: "upload_download" },
  mounts: [{ hostPath: "/home/tester/data", containerPath: "/data", readOnly: true }],
  security: { dockerSocket: false, dockerSocketGid: null, privileged: false, seccomp: "default", devices: [] },
  extraEnvironment: { SELKIES_GAMEPAD_ENABLED: "true" }, publication: { enabled: false, remotePort: null, automaticPort: true },
} as const;

const importPreflight = {
  stagingFileId: "77777777-7777-4777-8777-777777777777",
  manifest: {
    schemaVersion: 1,
    exportedTemplateId: "88888888-8888-4888-8888-888888888888",
    name: "导入模板测试",
    platform: "linux/amd64",
    imageReference: "com.cue.webtop-manager/template:88888888-8888-4888-8888-888888888888",
    imageId: "sha256:imported",
    sourceSpec,
    lineage: [],
    imagePayload: { path: "payload/image.tar.zst", sizeBytes: 2048, sha256: "b".repeat(64) },
    configPayload: { path: "payload/config.tar.zst", sizeBytes: 1024, sha256: "c".repeat(64) },
    createdAt: "2026-09-01T00:00:00Z",
  },
  nameConflict: false,
  sensitiveDataWarning: true,
  untrustedImageWarning: true,
};

beforeAll(async () => { await i18n.changeLanguage("zh-CN"); });
afterEach(() => cleanup());
beforeEach(() => {
  localStorage.clear();
  Object.values(api).forEach((mock) => mock.mockReset());
  api.getServerSettings.mockResolvedValue({ frpsHost: "", publicIp: "", frpsPort: 7000, remotePortStart: 41000, remotePortEnd: 42000, tokenConfigured: true, frpcImage: "frpc" });
  api.listTemplates.mockResolvedValue([{
    id: "11111111-1111-4111-8111-111111111111", name: "工作模板", imageReference: "com.cue.webtop-manager/template:11111111-1111-4111-8111-111111111111", imageId: "sha256:image", platform: "linux/amd64",
    systemSizeBytes: 1000, systemDeltaBytes: 500, snapshotPath: "opaque/config.tar.zst", snapshotSha256: "a".repeat(64), snapshotSizeBytes: 700, snapshotOriginalBytes: 900,
    sourceEnvironmentId: null, parentTemplateId: null, externalLineage: [], sourceSpec,
    officialSource: { reference: "lscr.io/linuxserver/webtop:ubuntu-mate", digest: "sha256:old", imageId: "sha256:base", buildVersion: "2026.08" },
    sourceCheck: { status: "updated", checkedAt: "2026-09-01T00:00:00Z", currentDigest: "sha256:new" }, integrity: "complete", trust: "local", createdAt: "2026-09-01T00:00:00Z",
  }]);
});

function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}><TemplatesPage /></QueryClientProvider>);
}

describe("TemplatesPage", () => {
  it("only exposes template-management actions", async () => {
    mount();
    expect(await screen.findByText("源镜像已有更新")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "创建环境" })).toBeNull();
    expect(screen.getByRole("button", { name: "导出" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "删除" })).toBeTruthy();
  });

  it("restores a tracked controller operation without rendering command output", async () => {
    localStorage.setItem("webtop-manager.operations.v1", JSON.stringify([{ id: "22222222-2222-4222-8222-222222222222", kind: "create_template" }]));
    api.getOperation.mockResolvedValue({ id: "22222222-2222-4222-8222-222222222222", kind: "create_template", phase: "running", progressPercent: 55, cancellable: false, resourceId: null, error: null, result: null, logLines: ["$ webtop-manager template create", "[worker] archiving complete /config as tar.zst"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:01Z" });
    mount();
    await waitFor(() => expect(api.getOperation).toHaveBeenCalled());
    expect(screen.queryByText(/55%/)).toBeNull();
    expect(screen.queryByText(/archiving complete/)).toBeNull();
  });

  it("streams sanitized controller output inside an importing template card", async () => {
    localStorage.setItem("webtop-manager.operations.v1", JSON.stringify([{
      id: "99999999-9999-4999-8999-999999999999",
      kind: "import_template",
      displayName: "正在导入的模板",
    }]));
    api.getOperation.mockResolvedValue({
      id: "99999999-9999-4999-8999-999999999999",
      kind: "import_template",
      phase: "running",
      progressPercent: 42,
      cancellable: true,
      resourceId: null,
      error: null,
      result: null,
      logLines: ["$ webtop-manager template import", "[controller] loading image layers"],
      createdAt: "2026-09-01T00:00:00Z",
      updatedAt: "2026-09-01T00:00:01Z",
    });
    mount();
    expect(await screen.findByRole("heading", { name: "正在导入的模板" })).toBeTruthy();
    const output = screen.getByRole("region", { name: "终端输出" });
    expect(within(output).getByText(/loading image layers/)).toBeTruthy();
    expect(within(output).getByText("执行中 · 42%")).toBeTruthy();
  });

  it("does not render native import command output", async () => {
    api.selectTemplateImport.mockResolvedValue("66666666-6666-4666-8666-666666666666");
    api.stageTemplateImport.mockImplementation(async (_sourceId: string, _transferId: string, onProgress: (value: unknown) => void) => {
      onProgress({ phase: "copying", message: "[desktop] staging import package", currentBytes: 16 * 1024 * 1024, totalBytes: 64 * 1024 * 1024 });
      return null;
    });
    mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: /导入模板/ }));
    await waitFor(() => expect(api.stageTemplateImport).toHaveBeenCalled());
    expect(screen.queryByText("[desktop] staging import package")).toBeNull();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows the validation dialog without a processing card and stops the native staging copy", async () => {
    api.selectTemplateImport.mockResolvedValue("66666666-6666-4666-8666-666666666666");
    api.stageTemplateImport.mockImplementation(async (_sourceId: string, _transferId: string, onProgress: (value: unknown) => void) => {
      onProgress({ phase: "copying", message: "[desktop] staging import package", currentBytes: 1024, totalBytes: 4096 });
      return new Promise<string | null>(() => undefined);
    });
    const { container } = mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: /导入模板/ }));
    const dialog = await screen.findByRole("dialog", { name: "验证及导入模板" });
    expect(within(dialog).getByText("正在读取模板")).toBeTruthy();
    expect(container.querySelector(".importing-template-card")).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "停止" }));
    await waitFor(() => expect(api.cancelTemplateTransfer).toHaveBeenCalledWith(expect.any(String)));
  });

  it("creates the processing card only after the user confirms the validated import", async () => {
    api.selectTemplateImport.mockResolvedValue("66666666-6666-4666-8666-666666666666");
    api.stageTemplateImport.mockResolvedValue(importPreflight.stagingFileId);
    api.getTemplateImportPreflight.mockResolvedValue(importPreflight);
    api.startTemplateImport.mockImplementation(() => new Promise(() => undefined));
    const { container } = mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: /导入模板/ }));
    const dialog = await screen.findByRole("dialog", { name: "验证及导入模板" });
    expect(container.querySelector(".importing-template-card")).toBeNull();
    expect(api.startTemplateImport).not.toHaveBeenCalled();
    for (const checkbox of within(dialog).getAllByRole("checkbox")) fireEvent.click(checkbox);
    fireEvent.click(within(dialog).getByRole("button", { name: "导入模板" }));
    expect(await screen.findByRole("heading", { name: "导入模板测试" })).toBeTruthy();
    expect(container.querySelector(".importing-template-card")).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "验证及导入模板" })).toBeNull();
    expect(api.startTemplateImport).toHaveBeenCalledWith({
      stagingFileId: importPreflight.stagingFileId,
      name: "导入模板测试",
      confirmedSensitiveData: true,
      confirmedUntrustedImage: true,
    });
  });

  it("selects an export destination before starting the controller export", async () => {
    api.selectTemplateExport.mockResolvedValue("33333333-3333-4333-8333-333333333333");
    api.startTemplateExport.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "queued", progressPercent: 0, cancellable: true, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: null, logLines: ["$ webtop-manager template export"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:00Z" });
    api.getOperation.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "running", progressPercent: 20, cancellable: true, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: null, logLines: ["[controller] preparing export"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:01Z" });
    mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: "导出" }));
    await waitFor(() => expect(api.startTemplateExport).toHaveBeenCalledWith("11111111-1111-4111-8111-111111111111"));
    expect(api.selectTemplateExport).toHaveBeenCalledWith("工作模板.wtmpl");
    expect(api.selectTemplateExport.mock.invocationCallOrder[0]).toBeLessThan(api.startTemplateExport.mock.invocationCallOrder[0]);
    expect(screen.queryByText(/\[controller\] preparing export/)).toBeNull();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("does not start exporting when the save dialog is cancelled", async () => {
    api.selectTemplateExport.mockResolvedValue(null);
    mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: "导出" }));
    await waitFor(() => expect(api.selectTemplateExport).toHaveBeenCalled());
    expect(api.startTemplateExport).not.toHaveBeenCalled();
  });

  it("stops a running controller export", async () => {
    api.selectTemplateExport.mockResolvedValue("33333333-3333-4333-8333-333333333333");
    api.startTemplateExport.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "queued", progressPercent: 0, cancellable: true, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: null, logLines: ["$ webtop-manager template export"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:00Z" });
    api.getOperation.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "running", progressPercent: 20, cancellable: true, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: null, logLines: ["[controller] preparing export"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:01Z" });
    mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: "导出" }));
    fireEvent.click(await screen.findByRole("button", { name: "停止" }));
    await waitFor(() => expect(api.cancelOperation).toHaveBeenCalledWith("44444444-4444-4444-8444-444444444444"));
  });

  it("saves a completed export without rendering controller or desktop output", async () => {
    api.selectTemplateExport.mockResolvedValue("33333333-3333-4333-8333-333333333333");
    api.startTemplateExport.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "queued", progressPercent: 0, cancellable: false, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: null, logLines: ["$ webtop-manager template export"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:00Z" });
    api.getOperation.mockResolvedValue({ id: "44444444-4444-4444-8444-444444444444", kind: "export_template", phase: "succeeded", progressPercent: 100, cancellable: false, resourceId: "11111111-1111-4111-8111-111111111111", error: null, result: { stagingFileId: "55555555-5555-4555-8555-555555555555", suggestedName: "工作模板.wtmpl" }, logLines: ["[controller] export complete"], createdAt: "2026-09-01T00:00:00Z", updatedAt: "2026-09-01T00:00:01Z" });
    api.saveTemplateExport.mockImplementation(async (_stagingId: string, _destinationId: string, _transferId: string, onProgress: (value: unknown) => void) => {
      onProgress({ phase: "complete", message: "[desktop] export saved atomically", currentBytes: 64, totalBytes: 64 });
      return true;
    });
    mount();
    await screen.findByText("工作模板");
    fireEvent.click(screen.getByRole("button", { name: "导出" }));
    await waitFor(() => expect(api.saveTemplateExport).toHaveBeenCalled());
    expect(screen.queryByText(/\[controller\] export complete/)).toBeNull();
    expect(screen.queryByText(/\[desktop\] export saved atomically/)).toBeNull();
  });
});
