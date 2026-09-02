// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../lib/i18n";
import { FrpsSetupGuide } from "./FrpsSetupGuide";

const apiMocks = vi.hoisted(() => ({
  getFrpsSetupGuide: vi.fn(),
  getServerSettings: vi.fn(),
  saveServerSettings: vi.fn(),
}));

vi.mock("../../lib/api", () => apiMocks);

const savedSettings = {
  frpsHost: "frps.example.com",
  frpsPort: 7000,
  publicIp: "203.0.113.10",
  remotePortStart: 41000,
  remotePortEnd: 42000,
  tokenConfigured: true,
  frpcImage: "ghcr.io/fatedier/frpc:v0.70.1",
};

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  apiMocks.getServerSettings.mockReset().mockResolvedValue(savedSettings);
  apiMocks.saveServerSettings.mockReset().mockImplementation(async (settings) => settings);
  apiMocks.getFrpsSetupGuide.mockReset().mockResolvedValue({
    dockerSetupScript: "sudo docker pull example/frps --bind-port 7100",
    nativeSetupScript: "sudo systemctl enable --now webtop-manager-frps.service",
    publicAddress: "203.0.113.10",
    bindPort: 7100,
    remotePortStart: 43000,
    remotePortEnd: 43100,
  });
});

describe("FrpsSetupGuide", () => {
  it("saves editable ports before generating isolated deployment commands", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <FrpsSetupGuide onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    const bindPort = await screen.findByLabelText("frps bindPort") as HTMLInputElement;
    const portStart = screen.getByLabelText("起始端口") as HTMLInputElement;
    const portEnd = screen.getByLabelText("结束端口") as HTMLInputElement;

    expect(bindPort.value).toBe("7000");
    expect(portStart.value).toBe("41000");
    expect(portEnd.value).toBe("42000");
    expect(screen.queryByText(/sudo docker pull/)).toBeNull();

    fireEvent.change(bindPort, { target: { value: "7100" } });
    fireEvent.change(portStart, { target: { value: "43000" } });
    fireEvent.change(portEnd, { target: { value: "43100" } });
    fireEvent.click(screen.getByRole("button", { name: "生成命令" }));

    expect(await screen.findByText("sudo docker pull example/frps --bind-port 7100")).toBeTruthy();
    expect(apiMocks.saveServerSettings).toHaveBeenCalledWith({
      ...savedSettings,
      frpsPort: 7100,
      remotePortStart: 43000,
      remotePortEnd: 43100,
    });
    expect(apiMocks.getFrpsSetupGuide).toHaveBeenCalledOnce();
    expect(screen.getByText(/与现有 frps 完全隔离/)).toBeTruthy();
    expect(screen.queryByRole("tab", { name: /已有 frps/ })).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: /不使用 Docker/ }));
    expect(screen.getByText("sudo systemctl enable --now webtop-manager-frps.service")).toBeTruthy();

    fireEvent.change(bindPort, { target: { value: "7200" } });
    expect(screen.queryByText("sudo systemctl enable --now webtop-manager-frps.service")).toBeNull();
    expect(screen.getByText(/旧命令已隐藏/)).toBeTruthy();
  });
});
