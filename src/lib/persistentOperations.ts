import type { PersistentOperation } from "./types";

const storageKey = "webtop-manager.operations.v1";

export interface TrackedOperation {
  id: string;
  kind: string;
  exportDestinationId?: string;
  displayName?: string;
}

export function trackedOperations(): TrackedOperation[] {
  try {
    const value = JSON.parse(localStorage.getItem(storageKey) ?? "[]") as unknown;
    return Array.isArray(value)
      ? value.filter((item): item is TrackedOperation => Boolean(
        item
        && typeof item.id === "string"
        && typeof item.kind === "string"
        && (item.exportDestinationId === undefined || typeof item.exportDestinationId === "string")
        && (item.displayName === undefined || typeof item.displayName === "string"),
      ))
      : [];
  } catch {
    return [];
  }
}

export function trackOperation(operation: PersistentOperation, kind = operation.kind, details: Pick<TrackedOperation, "exportDestinationId" | "displayName"> = {}): void {
  const current = trackedOperations().filter((item) => item.id !== operation.id);
  current.push({ id: operation.id, kind, ...details });
  localStorage.setItem(storageKey, JSON.stringify(current));
  window.dispatchEvent(new Event("webtop-operations-changed"));
}

export function forgetOperation(id: string): void {
  localStorage.setItem(storageKey, JSON.stringify(trackedOperations().filter((item) => item.id !== id)));
  window.dispatchEvent(new Event("webtop-operations-changed"));
}
