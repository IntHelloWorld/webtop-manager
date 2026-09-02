import { useSyncExternalStore } from "react";
import { cancelOfficialImagePull, pullOfficialImage } from "../../lib/api";
import type { ImagePullProgress } from "../../lib/types";

const MAX_LOG_LINES = 80;
const PROGRESS_UPDATE_INTERVAL_MS = 100;

interface ImagePullStoreState {
  reference: string | null;
  pullId: string | null;
  latest: ImagePullProgress | null;
  logs: ImagePullProgress[];
  isPending: boolean;
  isCancelling: boolean;
  isError: boolean;
  outcome: "completed" | "cancelled" | null;
}

export interface OfficialImagePullController extends ImagePullStoreState {
  start: (reference: string) => void;
  cancel: () => Promise<void>;
}

const initialState: ImagePullStoreState = {
  reference: null,
  pullId: null,
  latest: null,
  logs: [],
  isPending: false,
  isCancelling: false,
  isError: false,
  outcome: null,
};

let store = initialState;
let activePull: Promise<void> | null = null;
let bufferedLogs: ImagePullProgress[] = [];
let pendingProgress: ImagePullProgress | null = null;
let progressTimer: number | null = null;
const listeners = new Set<() => void>();

function emit(next: Partial<ImagePullStoreState>) {
  store = { ...store, ...next };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot() {
  return store;
}

function flushProgress() {
  if (progressTimer !== null) window.clearTimeout(progressTimer);
  progressTimer = null;
  if (!pendingProgress) return;
  emit({ latest: pendingProgress, logs: [...bufferedLogs] });
  pendingProgress = null;
}

function appendProgress(progress: ImagePullProgress) {
  pendingProgress = progress;
  const previous = bufferedLogs.at(-1);
  if (previous && previous.layerId === progress.layerId && previous.status === progress.status && previous.phase === progress.phase) {
    bufferedLogs[bufferedLogs.length - 1] = progress;
  } else {
    bufferedLogs = [...bufferedLogs.slice(-(MAX_LOG_LINES - 1)), progress];
  }
  if (progress.phase === "complete" || progress.phase === "cancelled" || progress.phase === "error") {
    flushProgress();
    return;
  }
  if (progressTimer === null) {
    progressTimer = window.setTimeout(flushProgress, PROGRESS_UPDATE_INTERVAL_MS);
  }
}

function start(reference: string) {
  if (activePull) return;
  const pullId = crypto.randomUUID();
  if (progressTimer !== null) window.clearTimeout(progressTimer);
  progressTimer = null;
  pendingProgress = null;
  bufferedLogs = [];
  emit({
    reference,
    pullId,
    latest: null,
    logs: [],
    isPending: true,
    isCancelling: false,
    isError: false,
    outcome: null,
  });

  activePull = pullOfficialImage(reference, pullId, appendProgress)
    .then((result) => {
      flushProgress();
      emit({
        isPending: false,
        isCancelling: false,
        outcome: result.cancelled ? "cancelled" : "completed",
      });
    })
    .catch(() => {
      flushProgress();
      emit({ isPending: false, isCancelling: false, isError: true, outcome: null });
    })
    .finally(() => {
      activePull = null;
    });
}

async function cancel() {
  if (!store.pullId || !store.isPending || store.isCancelling) return;
  emit({ isCancelling: true });
  try {
    await cancelOfficialImagePull(store.pullId);
  } catch {
    if (store.isPending) emit({ isCancelling: false, isError: true });
  }
}

export function useOfficialImagePull(): OfficialImagePullController {
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return { ...state, start, cancel };
}
