// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import "../../lib/i18n";
import i18n from "../../lib/i18n";
import { GuidePage } from "./GuidePage";

afterEach(cleanup);

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("GuidePage", () => {
  it("explains local creation, complete FRP setup, and token recovery with working shortcuts", () => {
    const onNavigate = vi.fn();
    render(<GuidePage onNavigate={onNavigate} />);

    expect(screen.getByRole("heading", { name: "从零创建第一个环境" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "第二阶段：配置 frps 与 frpc（仅公网访问需要）" })).toBeTruthy();
    expect(screen.getByText(/“frps 主机名或 IP”填写本机能够连接的远端地址/)).toBeTruthy();
    expect(screen.getByText(/确认地址、端口和认证都成功后/)).toBeTruthy();
    expect(screen.getByRole("heading", { name: "本地令牌文件误删后如何恢复" })).toBeTruthy();
    expect(screen.getByText(/本地认证材料已丢失/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "去创建环境" }));
    fireEvent.click(screen.getByRole("button", { name: "打开服务器设置" }));
    fireEvent.click(screen.getByRole("button", { name: "前往服务器设置恢复" }));
    expect(onNavigate.mock.calls).toEqual([["environments"], ["settings"], ["settings"]]);
  });
});
