// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../lib/i18n";
import { AppShell } from "./AppShell";
import { OperationFeedbackProvider, useOperationFeedback } from "./OperationFeedbackContext";

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

const cancelOperation = vi.fn();

beforeEach(() => cancelOperation.mockReset());
afterEach(cleanup);

function FeedbackHarness() {
  const { activeOperation, beginOperation, finishOperation } = useOperationFeedback();
  return <>
    <AppShell section="environments" onSectionChange={() => undefined} activeOperation={activeOperation}>
      <button type="button" onClick={() => beginOperation("publish", "公网桌面")}>开始发布</button>
      <button type="button" onClick={() => beginOperation("imagePull", "webtop:latest", cancelOperation)}>开始拉取</button>
    </AppShell>
    {activeOperation ? <button type="button" onClick={() => finishOperation(activeOperation.id)}>完成操作</button> : null}
  </>;
}

describe("OperationFeedbackProvider", () => {
  it("locks the application and removes the task navigation while an operation is active", async () => {
    const { container } = render(<OperationFeedbackProvider><FeedbackHarness /></OperationFeedbackProvider>);

    expect(screen.queryByRole("button", { name: "任务" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "开始发布" }));

    const shell = container.querySelector(".app-shell");
    expect(shell?.getAttribute("aria-busy")).toBe("true");
    expect(shell?.hasAttribute("inert")).toBe(true);
    expect(screen.getByRole("status").textContent).toContain("正在将环境“公网桌面”发布到公网");

    fireEvent.click(screen.getByRole("button", { name: "完成操作" }));
    expect(container.querySelector(".operation-lock")).toBeNull();
    expect(shell?.getAttribute("aria-busy")).toBe("false");
    expect(shell?.hasAttribute("inert")).toBe(false);
  });

  it("keeps an explicit cancellation control outside the inert application", () => {
    const view = render(<OperationFeedbackProvider><FeedbackHarness /></OperationFeedbackProvider>);

    fireEvent.click(view.getByRole("button", { name: "开始拉取" }));
    fireEvent.click(view.getByRole("button", { name: "取消" }));

    expect(cancelOperation).toHaveBeenCalledOnce();
  });
});
