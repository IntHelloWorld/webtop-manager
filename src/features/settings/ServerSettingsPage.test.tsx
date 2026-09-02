// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import "../../lib/i18n";
import i18n from "../../lib/i18n";
import { ServerSettingsPage } from "./ServerSettingsPage";

const defaultFrpcImage = "ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22";

vi.mock("../../lib/api", () => ({
  getServerSettings: vi.fn().mockResolvedValue({
    frpsHost: "frps.example.com",
    frpsPort: 7000,
    publicIp: "203.0.113.10",
    remotePortStart: 41000,
    remotePortEnd: 42000,
    tokenConfigured: true,
    frpcImage: "ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22",
  }),
  getFrpcStatus: vi.fn().mockResolvedValue({
    state: "not_created",
    connected: false,
    image: null,
    startedAt: null,
    exitCode: null,
  }),
  regenerateServerToken: vi.fn(),
  runFrpcAction: vi.fn(),
  saveServerSettings: vi.fn(),
  testFrpcConnectivity: vi.fn(),
  getFrpsSetupGuide: vi.fn(),
}));

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
    expect(screen.getByRole("button", { name: "查看 frps 部署教程" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "测试连通性" })).toBeTruthy();

    fireEvent.change(imageInput, { target: { value: "registry.example/frpc:custom" } });
    fireEvent.click(screen.getByRole("button", { name: "恢复默认" }));
    expect(imageInput.value).toBe(defaultFrpcImage);
  });
});
