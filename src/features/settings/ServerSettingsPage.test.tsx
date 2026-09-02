// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../../lib/i18n";
import i18n from "../../lib/i18n";
import { ServerSettingsPage } from "./ServerSettingsPage";

const defaultFrpcImage = "ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22";

const readySettings = {
  frpsHost: "frps.example.com",
  frpsPort: 7000,
  publicIp: "203.0.113.10",
  remotePortStart: 41000,
  remotePortEnd: 42000,
  tokenConfigured: true,
  tokenState: "ready" as const,
  frpcImage: defaultFrpcImage,
};

const apiMocks = vi.hoisted(() => ({
  getServerSettings: vi.fn(),
  getFrpcStatus: vi.fn(),
  recoverServerToken: vi.fn(),
  runFrpcAction: vi.fn(),
  saveServerSettings: vi.fn(),
  testFrpcConnectivity: vi.fn(),
  getFrpsSetupGuide: vi.fn(),
}));

vi.mock("../../lib/api", () => apiMocks);

beforeEach(() => {
  apiMocks.getServerSettings.mockReset().mockResolvedValue(readySettings);
  apiMocks.getFrpcStatus.mockReset().mockResolvedValue({
    state: "not_created",
    connected: false,
    image: null,
    startedAt: null,
    exitCode: null,
  });
  apiMocks.recoverServerToken.mockReset();
  apiMocks.runFrpcAction.mockReset();
  apiMocks.saveServerSettings.mockReset();
  apiMocks.testFrpcConnectivity.mockReset();
  apiMocks.getFrpsSetupGuide.mockReset();
});

afterEach(cleanup);

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("ServerSettingsPage", () => {
  it("prefills and restores the frpc image while keeping token entry app-managed", async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ServerSettingsPage />
      </QueryClientProvider>,
    );

    const imageMatches = await screen.findAllByLabelText(/frpc 镜像/);
    const imageInput = imageMatches.find((element) => element instanceof HTMLInputElement) as HTMLInputElement;
    expect(imageInput.value).toBe(defaultFrpcImage);
    expect(screen.queryByLabelText("认证令牌")).toBeNull();
    expect(screen.queryByRole("button", { name: "重新生成令牌" })).toBeNull();
    expect(screen.queryByRole("button", { name: "恢复远程连接" })).toBeNull();
    expect(screen.getByText("认证令牌由应用自动管理").closest(".security-banner")?.classList.contains("settings-width")).toBe(true);
    expect(screen.getByRole("button", { name: "查看 frps 部署教程" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "测试连通性" })).toBeTruthy();

    fireEvent.change(imageInput, { target: { value: "registry.example/frpc:custom" } });
    fireEvent.click(screen.getByRole("button", { name: "恢复默认" }));
    expect(imageInput.value).toBe(defaultFrpcImage);
  });

  it("offers a gated recovery flow only when the local token is missing", async () => {
    const missingSettings = {
      ...readySettings,
      tokenConfigured: false,
      tokenState: "missing" as const,
    };
    const pendingSettings = {
      ...missingSettings,
      tokenConfigured: true,
      tokenState: "recovery_pending" as const,
    };
    apiMocks.getServerSettings
      .mockReset()
      .mockResolvedValueOnce(missingSettings)
      .mockResolvedValue(pendingSettings);
    apiMocks.recoverServerToken.mockResolvedValue(pendingSettings);
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ServerSettingsPage />
      </QueryClientProvider>,
    );

    expect((await screen.findByRole("alert")).textContent).toContain("本地认证材料已丢失");
    expect(screen.getByRole("button", { name: "启动" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "测试连通性" }).hasAttribute("disabled")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "恢复远程连接" }));

    await waitFor(() => expect(apiMocks.recoverServerToken).toHaveBeenCalledOnce());
    expect(await screen.findByRole("heading", { name: "部署远端 frps" })).toBeTruthy();
    expect(screen.getByText("此命令用于恢复远端认证")).toBeTruthy();
  });
});
