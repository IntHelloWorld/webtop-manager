// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../lib/i18n";
import { EnvironmentList } from "./EnvironmentList";
import { OperationFeedbackProvider, useOperationFeedback } from "../../components/OperationFeedbackContext";

const apiMocks = vi.hoisted(() => ({
  cancelOfficialImagePull: vi.fn(),
  createEnvironment: vi.fn(),
  createEnvironmentFromTemplate: vi.fn(),
  getEnvironmentCredentials: vi.fn(),
  getFrpcStatus: vi.fn(),
  getOperation: vi.fn(),
  getTemplatePreflight: vi.fn(),
  getServerSettings: vi.fn(),
  listEnvironments: vi.fn(),
  listOfficialImages: vi.fn(),
  listTemplates: vi.fn(),
  openEnvironmentDataDirectory: vi.fn(),
  openLocalEnvironment: vi.fn(),
  openPublicEnvironment: vi.fn(),
  pruneImageCache: vi.fn(),
  pullOfficialImage: vi.fn(),
  removeEnvironment: vi.fn(),
  runEnvironmentAction: vi.fn(),
  runEnvironmentPublicationAction: vi.fn(),
  createTemplate: vi.fn(),
}));

vi.mock("../../lib/api", () => apiMocks);

function ActiveOperationProbe() {
  const { activeOperation } = useOperationFeedback();
  return <output data-testid="active-operation">{activeOperation?.kind ?? "none"}</output>;
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});
afterEach(() => cleanup());

beforeEach(() => {
  localStorage.clear();
  Object.values(apiMocks).forEach((mock) => mock.mockReset());
  apiMocks.listOfficialImages.mockResolvedValue([]);
  apiMocks.listTemplates.mockResolvedValue([]);
  apiMocks.getServerSettings.mockResolvedValue({
    frpsHost: "frps.example.com",
    frpsPort: 7000,
    publicIp: "desktop.example.com",
    remotePortStart: 41000,
    remotePortEnd: 42000,
    tokenConfigured: true,
    tokenState: "ready",
    frpcImage: "ghcr.io/fatedier/frpc:v0.70.1",
  });
  apiMocks.getFrpcStatus.mockResolvedValue({
    state: "running",
    connected: true,
    image: "ghcr.io/fatedier/frpc:v0.70.1",
    startedAt: "2026-09-01T00:00:00Z",
    exitCode: null,
  });
  apiMocks.getEnvironmentCredentials.mockResolvedValue({
    username: "webtop-11111111-1111-4111-8111-111111111111",
    password: "forty-character-random-webtop-password-value",
  });
  apiMocks.listEnvironments.mockResolvedValue([{
    id: "11111111-1111-4111-8111-111111111111",
    name: "公网桌面",
    containerId: "container-id",
    configPath: "/data/environments/11111111-1111-4111-8111-111111111111/config",
    desiredRunning: true,
    localPort: 49152,
    createdAt: "2026-09-01T00:00:00Z",
    spec: {
      name: "公网桌面",
      image: "lscr.io/linuxserver/webtop:ubuntu-mate",
      identity: { uid: 1000, gid: 1000, timezone: "Asia/Shanghai", locale: "zh_CN.UTF-8" },
      resources: { cpuLimit: null, memoryBytes: null, shmBytes: 1073741824 },
      display: { width: null, height: null, wayland: null, gpu: "disabled", audio: true, clipboard: true, fileTransfer: true, fileTransferMode: "upload_download" },
      mounts: [],
      security: { dockerSocket: false, dockerSocketGid: null, privileged: false, seccomp: "default", devices: [] },
      extraEnvironment: {},
      publication: { enabled: true, remotePort: 41000, automaticPort: true },
    },
  }]);
  apiMocks.openEnvironmentDataDirectory.mockResolvedValue(undefined);
  apiMocks.openLocalEnvironment.mockResolvedValue(undefined);
  apiMocks.openPublicEnvironment.mockResolvedValue(undefined);
  apiMocks.removeEnvironment.mockResolvedValue(undefined);
});

describe("EnvironmentList", () => {
  it("shows a public URL and opens the verified host data directory", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <OperationFeedbackProvider>
        <ActiveOperationProbe />
        <QueryClientProvider client={queryClient}>
          <EnvironmentList hostUid={1000} hostGid={1000} />
        </QueryClientProvider>
      </OperationFeedbackProvider>,
    );

    const publicLink = await screen.findByRole("link", {
      name: "https://desktop.example.com:41000/",
    });
    expect(publicLink.getAttribute("href")).toBe("https://desktop.example.com:41000/");
    expect(screen.getByText(/frpc 已连接/)).toBeTruthy();
    expect(await screen.findByText("webtop-11111111-1111-4111-8111-111111111111")).toBeTruthy();
    const password = await screen.findByLabelText("密码") as HTMLInputElement;
    expect(password.type).toBe("password");
    expect(password.value).toBe("forty-character-random-webtop-password-value");

    fireEvent.click(screen.getByRole("button", { name: "显示密码" }));
    expect(password.type).toBe("text");

    fireEvent.click(publicLink);
    await waitFor(() => expect(apiMocks.openPublicEnvironment.mock.calls[0]?.[0]).toBe(
      "11111111-1111-4111-8111-111111111111",
    ));

    fireEvent.click(screen.getByRole("button", { name: "打开桌面" }));
    await waitFor(() => expect(apiMocks.openLocalEnvironment).toHaveBeenCalledTimes(1));
    expect(apiMocks.openLocalEnvironment.mock.calls[0]?.[0]).toBe(49152);

    fireEvent.click(screen.getByRole("button", { name: "打开数据目录" }));
    await waitFor(() => expect(apiMocks.openEnvironmentDataDirectory.mock.calls[0]?.[0]).toBe(
      "11111111-1111-4111-8111-111111111111",
    ));

    let finishPublication: (() => void) | undefined;
    apiMocks.runEnvironmentPublicationAction.mockImplementation(() => new Promise<void>((resolve) => { finishPublication = resolve; }));
    fireEvent.click(screen.getByRole("button", { name: "停止公网发布" }));
    const publishingButton = await screen.findByRole("button", { name: "正在停止发布…" });
    expect(publishingButton.hasAttribute("disabled")).toBe(true);
    expect(publishingButton.className).toContain("is-working");
    await waitFor(() => expect(apiMocks.runEnvironmentPublicationAction).toHaveBeenCalledWith(
      "11111111-1111-4111-8111-111111111111",
      "unpublish",
    ));
    const [publishedEnvironment] = await apiMocks.listEnvironments.mock.results[0].value;
    apiMocks.listEnvironments.mockResolvedValue([{
      ...publishedEnvironment,
      spec: {
        ...publishedEnvironment.spec,
        publication: { enabled: false, remotePort: null, automaticPort: true },
      },
    }]);
    finishPublication?.();
    await screen.findByRole("button", { name: "发布到公网" });
    expect(screen.getByText("webtop-11111111-1111-4111-8111-111111111111")).toBeTruthy();
    expect((screen.getByLabelText("密码") as HTMLInputElement).value).toBe("forty-character-random-webtop-password-value");
  });

  it("moves deletion feedback to the target card without locking the rest of the page", async () => {
    let finishRemoval: (() => void) | undefined;
    apiMocks.removeEnvironment.mockImplementation(() => new Promise<void>((resolve) => { finishRemoval = resolve; }));
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const { container } = render(
      <OperationFeedbackProvider>
        <ActiveOperationProbe />
        <QueryClientProvider client={queryClient}>
          <EnvironmentList hostUid={1000} hostGid={1000} />
        </QueryClientProvider>
      </OperationFeedbackProvider>,
    );

    await screen.findByText("公网桌面");
    fireEvent.click(screen.getByRole("button", { name: "删除" }));
    const dialog = screen.getByRole("alertdialog");
    fireEvent.change(within(dialog).getByLabelText("公网桌面"), { target: { value: "公网桌面" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "删除" }));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    const environmentCard = screen.getByRole("heading", { name: "公网桌面" }).closest("article")!;
    const pendingButton = within(environmentCard).getByRole("button", { name: "正在删除…" });
    expect(pendingButton.hasAttribute("disabled")).toBe(true);
    expect(pendingButton.className).toContain("environment-delete-working");
    expect(environmentCard.hasAttribute("inert")).toBe(true);
    expect(container.querySelector(".operation-lock")).toBeNull();
    expect(screen.getByTestId("active-operation").textContent).toBe("none");
    expect(screen.getByRole("button", { name: /创建环境/ }).hasAttribute("disabled")).toBe(false);
    await waitFor(() => expect(apiMocks.removeEnvironment).toHaveBeenCalledWith(
      "11111111-1111-4111-8111-111111111111",
      "公网桌面",
      true,
    ));

    finishRemoval?.();
  });

  it("creates from a template through the environment creation dialog", async () => {
    const [sourceEnvironment] = await apiMocks.listEnvironments();
    apiMocks.listTemplates.mockResolvedValue([{
      id: "22222222-2222-4222-8222-222222222222",
      name: "工作模板",
      imageReference: "com.cue.webtop-manager/template:22222222-2222-4222-8222-222222222222",
      imageId: "sha256:template",
      platform: "linux/amd64",
      systemSizeBytes: 1000,
      systemDeltaBytes: 500,
      snapshotPath: "opaque/config.tar.zst",
      snapshotSha256: "a".repeat(64),
      snapshotSizeBytes: 700,
      snapshotOriginalBytes: 900,
      sourceEnvironmentId: sourceEnvironment.id,
      parentTemplateId: null,
      externalLineage: [],
      sourceSpec: sourceEnvironment.spec,
      officialSource: null,
      sourceCheck: { status: "not_checked", checkedAt: null, currentDigest: null },
      integrity: "complete",
      trust: "local",
      createdAt: "2026-09-01T00:00:00Z",
    }]);
    apiMocks.createEnvironmentFromTemplate.mockResolvedValue({
      id: "33333333-3333-4333-8333-333333333333",
      kind: "restore_template",
      phase: "running",
      progressPercent: 35,
      cancellable: true,
      resourceId: "44444444-4444-4444-8444-444444444444",
      error: null,
      result: { environmentId: "44444444-4444-4444-8444-444444444444" },
      logLines: ["$ webtop-manager environment restore", "[controller] restoring /config"],
      createdAt: "2026-09-01T00:00:00Z",
      updatedAt: "2026-09-01T00:00:01Z",
    });
    apiMocks.getOperation.mockImplementation(() => new Promise(() => undefined));
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <OperationFeedbackProvider>
        <ActiveOperationProbe />
        <QueryClientProvider client={queryClient}>
          <EnvironmentList hostUid={1000} hostGid={1000} />
        </QueryClientProvider>
      </OperationFeedbackProvider>,
    );

    await screen.findByText("公网桌面");
    fireEvent.click(screen.getByRole("button", { name: /创建环境/ }));
    const dialog = await screen.findByRole("dialog", { name: "创建 Webtop 环境" });
    fireEvent.click(await within(dialog).findByRole("radio", { name: /工作模板/ }));
    fireEvent.change(within(dialog).getByLabelText("环境名称"), { target: { value: "模板桌面" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "创建环境" }));

    await waitFor(() => expect(apiMocks.createEnvironmentFromTemplate).toHaveBeenCalled());
    expect(screen.queryByRole("dialog", { name: "创建 Webtop 环境" })).toBeNull();
    expect(await screen.findByRole("heading", { name: "模板桌面" })).toBeTruthy();
    const processingCard = screen.getByRole("heading", { name: "模板桌面" }).closest("article")!;
    expect(processingCard.className).toContain("creating-environment-card");
    expect(within(processingCard).getByText(/restoring \/config/)).toBeTruthy();
    expect(screen.getByTestId("active-operation").textContent).toBe("none");
    expect(screen.getByRole("button", { name: "打开数据目录" }).hasAttribute("disabled")).toBe(false);
    expect(apiMocks.createEnvironmentFromTemplate.mock.calls[0]?.[0]).toBe("22222222-2222-4222-8222-222222222222");
    expect(apiMocks.createEnvironmentFromTemplate.mock.calls[0]?.[1]).toMatchObject({
      name: "模板桌面",
      image: "com.cue.webtop-manager/template:22222222-2222-4222-8222-222222222222",
    });
    expect(apiMocks.createEnvironment).not.toHaveBeenCalled();
  });
});
