import { useTranslation } from "react-i18next";
import type { ImagePullProgress } from "../../lib/types";

interface ImagePullStatusProps {
  latest: ImagePullProgress | null;
  logs: ImagePullProgress[];
  isCancelling: boolean;
  onCancel: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} KiB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}

function formatLogLine(progress: ImagePullProgress): string {
  const layer = progress.layerId ? `${progress.layerId}: ` : "";
  const bytes = progress.currentBytes !== null && progress.totalBytes !== null
    ? ` ${formatBytes(progress.currentBytes)} / ${formatBytes(progress.totalBytes)}`
    : "";
  return `${layer}${progress.status}${bytes}`;
}

export function ImagePullStatus({ latest, logs, isCancelling, onCancel }: ImagePullStatusProps) {
  const { t } = useTranslation();
  const current = latest?.aggregateCurrentBytes ?? latest?.currentBytes ?? null;
  const total = latest?.aggregateTotalBytes ?? latest?.totalBytes ?? null;
  const percent = current !== null && total !== null && total > 0
    ? Math.min(100, Math.max(0, Math.round((current / total) * 100)))
    : null;

  return (
    <div className="image-pull-progress">
      <div className="image-pull-progress-header">
        <div>
          <strong>{t("images.downloadProgress")}</strong>
          <p aria-live="polite">{latest?.status ?? t("images.preparingPull")}{percent !== null ? ` · ${percent}%` : ""}</p>
        </div>
        <button type="button" className="danger-button small-button" disabled={isCancelling} onClick={onCancel}>
          {t(isCancelling ? "images.stoppingPull" : "images.stopPull")}
        </button>
      </div>
      <progress max={total ?? 1} value={current ?? undefined} aria-label={t("images.downloadProgress")} />
      {current !== null && total !== null ? <small>{formatBytes(current)} / {formatBytes(total)}</small> : null}
      <details className="pull-output" open>
        <summary>{t("images.commandOutput")}</summary>
        <pre aria-live="polite">{logs.length > 0 ? logs.map(formatLogLine).join("\n") : t("images.waitingForOutput")}</pre>
      </details>
    </div>
  );
}
