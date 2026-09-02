import { useEffect, useRef, useSyncExternalStore } from "react";
import { useOperationFeedback } from "../../components/OperationFeedbackContext";
import { cancelOfficialImagePull, getOperation, pullOfficialImage } from "../../lib/api";
import type { ImagePullProgress } from "../../lib/types";

const MAX_LOG_LINES = 80;
const PROGRESS_UPDATE_INTERVAL_MS = 100;
const ACTIVE_PULL_STORAGE_KEY = "webtop-manager.image-pull.v1";

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
let resumeStarted = false;
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
  localStorage.setItem(ACTIVE_PULL_STORAGE_KEY, JSON.stringify({ pullId, reference }));

  activePull = pullOfficialImage(reference, pullId, appendProgress)
    .then((result) => {
      flushProgress();
      emit({
        isPending: false,
        isCancelling: false,
        outcome: result.cancelled ? "cancelled" : "completed",
      });
    })
    .catch(async () => {
      const outcome = await followDurablePull(pullId, reference);
      flushProgress();
      if (outcome === "completed" || outcome === "cancelled") {
        emit({ isPending: false, isCancelling: false, isError: false, outcome });
      } else {
        emit({ isPending: false, isCancelling: false, isError: true, outcome: null });
      }
    })
    .finally(() => {
      localStorage.removeItem(ACTIVE_PULL_STORAGE_KEY);
      activePull = null;
    });
}

async function followDurablePull(
  pullId: string,
  reference: string,
): Promise<"completed" | "cancelled" | "error"> {
  let unavailableAttempts = 0;
  while (unavailableAttempts < 240) {
    try {
      const operation = await getOperation(pullId);
      unavailableAttempts = 0;
      appendProgress({
        pullId,
        reference,
        phase: "progress",
        layerId: null,
        status: "Image pull is running in the persistent controller",
        currentBytes: operation.progressPercent,
        totalBytes: 100,
        aggregateCurrentBytes: operation.progressPercent,
        aggregateTotalBytes: 100,
      });
      if (operation.phase === "succeeded") return "completed";
      if (operation.phase === "cancelled") return "cancelled";
      if (operation.phase === "failed" || operation.phase === "retryable") return "error";
    } catch (error) {
      if (
        error
        && typeof error === "object"
        && "code" in error
        && error.code === "INVALID_REQUEST"
      ) return "error";
      unavailableAttempts += 1;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 500));
  }
  return "error";
}

function resumeStoredPull() {
  if (resumeStarted || activePull) return;
  resumeStarted = true;
  try {
    const stored = JSON.parse(localStorage.getItem(ACTIVE_PULL_STORAGE_KEY) ?? "null") as unknown;
    if (!stored || typeof stored !== "object") return;
    const { pullId, reference } = stored as { pullId?: unknown; reference?: unknown };
    if (typeof pullId !== "string" || typeof reference !== "string") return;
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
    activePull = followDurablePull(pullId, reference)
      .then((outcome) => {
        flushProgress();
        if (outcome === "completed" || outcome === "cancelled") {
          emit({ isPending: false, isCancelling: false, isError: false, outcome });
        } else {
          emit({ isPending: false, isCancelling: false, isError: true, outcome: null });
        }
      })
      .finally(() => {
        localStorage.removeItem(ACTIVE_PULL_STORAGE_KEY);
        activePull = null;
      });
  } catch {
    localStorage.removeItem(ACTIVE_PULL_STORAGE_KEY);
  }
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
  const { activeOperation, beginOperation, finishOperation } = useOperationFeedback();
  const feedbackId = useRef<string | null>(null);
  useEffect(resumeStoredPull, []);
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  useEffect(() => {
    if (state.isPending && state.reference && !feedbackId.current && !activeOperation) {
      feedbackId.current = beginOperation("imagePull", state.reference, () => void cancel());
    } else if (!state.isPending && feedbackId.current) {
      finishOperation(feedbackId.current);
      feedbackId.current = null;
    }
  }, [activeOperation, beginOperation, finishOperation, state.isPending, state.reference]);
  return { ...state, start, cancel };
}
