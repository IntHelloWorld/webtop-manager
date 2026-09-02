// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../../lib/i18n";
import type { ImagePullProgress } from "../../lib/types";
import { ImagePullStatus } from "./ImagePullStatus";

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("ImagePullStatus", () => {
  it("shows aggregate progress, Docker output, and a working cancel control", () => {
    const onCancel = vi.fn();
    const progress: ImagePullProgress = {
      pullId: "pull-id",
      reference: "lscr.io/linuxserver/webtop:ubuntu-mate",
      phase: "progress",
      layerId: "abc123",
      status: "Downloading",
      currentBytes: 25 * 1024 ** 2,
      totalBytes: 50 * 1024 ** 2,
      aggregateCurrentBytes: 40 * 1024 ** 2,
      aggregateTotalBytes: 80 * 1024 ** 2,
    };

    render(<ImagePullStatus latest={progress} logs={[progress]} isCancelling={false} onCancel={onCancel} />);

    const progressbar = screen.getByRole("progressbar") as HTMLProgressElement;
    expect(progressbar.value).toBe(40 * 1024 ** 2);
    expect(progressbar.max).toBe(80 * 1024 ** 2);
    expect(screen.getByText("Downloading · 50%")).toBeTruthy();
    expect(screen.getByText(/abc123: Downloading/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "停止下载" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
