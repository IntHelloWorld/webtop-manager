// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../lib/i18n";
import { OperationFeedbackProvider } from "../../components/OperationFeedbackContext";
import { ImageCatalog } from "./ImageCatalog";

const apiMocks = vi.hoisted(() => ({
  deleteOfficialImage: vi.fn(),
  listOfficialImages: vi.fn(),
  pruneImageCache: vi.fn(),
}));
const pullMock = vi.hoisted(() => ({
  isPending: false,
  reference: null,
  latest: null,
  logs: [] as string[],
  isCancelling: false,
  outcome: null,
  isError: false,
  start: vi.fn(),
  cancel: vi.fn(),
}));

vi.mock("../../lib/api", () => apiMocks);
vi.mock("./useOfficialImagePull", () => ({ useOfficialImagePull: () => pullMock }));

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  Object.values(apiMocks).forEach((mock) => mock.mockReset());
  pullMock.start.mockReset();
  pullMock.cancel.mockReset();
  apiMocks.listOfficialImages.mockResolvedValue([{
    reference: "lscr.io/linuxserver/webtop:debian-xfce",
    tag: "debian-xfce",
    distribution: "Debian",
    desktop: "XFCE",
    waylandSupport: false,
    waylandOnly: false,
    installed: true,
    imageId: "sha256:image",
    sizeBytes: 1024,
  }]);
});

describe("ImageCatalog", () => {
  it("confirms official-image deletion and shows an in-progress button", async () => {
    let finishDeletion: (() => void) | undefined;
    apiMocks.deleteOfficialImage.mockImplementation(() => new Promise<void>((resolve) => { finishDeletion = resolve; }));
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    render(<OperationFeedbackProvider><QueryClientProvider client={queryClient}><ImageCatalog /></QueryClientProvider></OperationFeedbackProvider>);

    fireEvent.click(await screen.findByRole("button", { name: "删除镜像" }));
    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText("lscr.io/linuxserver/webtop:debian-xfce")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "确认删除镜像" }));

    const pendingButton = within(dialog).getByRole("button", { name: "正在删除…" });
    expect(pendingButton.hasAttribute("disabled")).toBe(true);
    expect(pendingButton.className).toContain("is-working");
    await waitFor(() => expect(apiMocks.deleteOfficialImage).toHaveBeenCalledWith("lscr.io/linuxserver/webtop:debian-xfce"));

    finishDeletion?.();
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });
});
