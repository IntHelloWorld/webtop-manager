// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import "../../lib/i18n";
import i18n from "../../lib/i18n";
import type { EnvironmentSpec, TemplateRecord } from "../../lib/types";
import { CreateEnvironmentDialog } from "./CreateEnvironmentDialog";

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});
afterEach(() => cleanup());

const templateSourceSpec: EnvironmentSpec = {
  name: "source",
  image: "lscr.io/linuxserver/webtop:ubuntu-mate",
  identity: { uid: 1000, gid: 1000, timezone: "Asia/Shanghai", locale: "zh_CN.UTF-8" },
  resources: { cpuLimit: 4, memoryBytes: 8 * 1024 ** 3, shmBytes: 2 * 1024 ** 3 },
  display: { width: 1920, height: 1080, wayland: false, gpu: "disabled", audio: true, clipboard: true, fileTransfer: true, fileTransferMode: "upload_download" },
  mounts: [{ hostPath: "/home/tester/template-data", containerPath: "/data", readOnly: true }],
  security: { dockerSocket: false, dockerSocketGid: null, privileged: false, seccomp: "default", devices: [] },
  extraEnvironment: {},
  publication: { enabled: false, remotePort: null, automaticPort: true },
};

const templateRecord: TemplateRecord = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "工作模板",
  imageReference: "com.cue.webtop-manager/template:11111111-1111-4111-8111-111111111111",
  imageId: "sha256:template",
  platform: "linux/amd64",
  systemSizeBytes: 1000,
  systemDeltaBytes: 500,
  snapshotPath: "opaque/config.tar.zst",
  snapshotSha256: "a".repeat(64),
  snapshotSizeBytes: 700,
  snapshotOriginalBytes: 900,
  sourceEnvironmentId: null,
  parentTemplateId: null,
  externalLineage: [],
  sourceSpec: templateSourceSpec,
  officialSource: null,
  sourceCheck: { status: "not_checked", checkedAt: null, currentDigest: null },
  integrity: "complete",
  trust: "local",
  createdAt: "2026-09-01T00:00:00Z",
};

describe("CreateEnvironmentDialog", () => {
  it("renders guided controls and submits official defaults without hard-coded display overrides", async () => {
    const onSubmit = vi.fn();
    render(
      <CreateEnvironmentDialog
        open
        pending={false}
        onClose={vi.fn()}
        onSubmit={onSubmit}
        hostUid={1000}
        hostGid={1000}
        officialImages={[{
          reference: "lscr.io/linuxserver/webtop:ubuntu-mate",
          tag: "ubuntu-mate",
          distribution: "Ubuntu",
          desktop: "MATE",
          waylandSupport: false,
          waylandOnly: false,
          installed: true,
          imageId: "sha256:test",
          sizeBytes: 1,
        }]}
        templates={[]}
        imagesLoading={false}
        pullingImage={null}
        pullProgress={null}
        pullLogs={[]}
        pullCancelling={false}
        pullCancelled={false}
        pullFailed={false}
        cachePruning={false}
        cachePruneResult={null}
        cachePruneFailed={false}
        publicationAvailable
        onPullImage={vi.fn()}
        onCancelPull={vi.fn()}
        onClearCache={vi.fn()}
      />,
    );

    expect(document.querySelectorAll(".help-trigger").length).toBeGreaterThan(10);
    expect(screen.getAllByRole("combobox").length).toBeGreaterThan(10);
    expect(screen.getByText("官方高级配置")).toBeTruthy();
    expect(screen.getByText("Webtop 官方镜像")).toBeTruthy();
    expect(screen.getByText("模板镜像")).toBeTruthy();

    const cpuHelp = screen.getByRole("button", { name: /CPU 上限（核）:/ });
    fireEvent.mouseEnter(cpuHelp);
    expect((await screen.findByRole("tooltip")).textContent).toContain("Docker 可使用的 CPU 核数上限");
    fireEvent.mouseLeave(cpuHelp);

    fireEvent.click(screen.getByRole("combobox", { name: "CPU 上限（核）" }));
    expect(await screen.findByRole("listbox", { name: "CPU 上限（核）" })).toBeTruthy();
    fireEvent.click(screen.getByRole("option", { name: "2" }));

    fireEvent.change(screen.getByPlaceholderText("例如：设计工作台"), { target: { value: "测试桌面" } });
    fireEvent.click(screen.getByRole("button", { name: "创建环境" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      name: "测试桌面",
      display: {
        width: null,
        height: null,
        wayland: null,
        fileTransferMode: "upload_download",
      },
      resources: { cpuLimit: 2, memoryBytes: null, shmBytes: 1024 ** 3 },
      publication: { enabled: false, remotePort: null, automaticPort: true },
    });
  });

  it("lists template images separately and applies the selected template settings", async () => {
    const onSubmit = vi.fn();
    render(
      <CreateEnvironmentDialog
        open pending={false} onClose={vi.fn()} onSubmit={onSubmit} hostUid={1000} hostGid={1000}
        officialImages={[]}
        templates={[templateRecord]}
        imagesLoading={false} pullingImage={null} pullProgress={null} pullLogs={[]} pullCancelling={false}
        pullCancelled={false} pullFailed={false} cachePruning={false} cachePruneResult={null} cachePruneFailed={false}
        publicationAvailable onPullImage={vi.fn()} onCancelPull={vi.fn()} onClearCache={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("radio", { name: /工作模板/ }));
    expect(screen.getByText(/将从模板“工作模板”创建/)).toBeTruthy();
    expect(screen.getByDisplayValue("/home/tester/template-data")).toBeTruthy();
    expect(screen.getByText("1920 × 1080")).toBeTruthy();

    fireEvent.change(screen.getByPlaceholderText("例如：设计工作台"), { target: { value: "模板桌面" } });
    fireEvent.click(screen.getByRole("button", { name: "创建环境" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      name: "模板桌面",
      image: templateRecord.imageReference,
      resources: { cpuLimit: 4, memoryBytes: 8 * 1024 ** 3, shmBytes: 2 * 1024 ** 3 },
      mounts: [{ hostPath: "/home/tester/template-data", containerPath: "/data", readOnly: true }],
    });
  });
});
