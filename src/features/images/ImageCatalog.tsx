import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteOfficialImage, listOfficialImages, pruneImageCache } from "../../lib/api";
import type { OfficialImage } from "../../lib/types";
import { ImagePullStatus } from "./ImagePullStatus";
import { useOfficialImagePull } from "./useOfficialImagePull";
import { ImageCachePruneDialog } from "./ImageCachePruneDialog";
import { ImageDeleteDialog } from "./ImageDeleteDialog";
import { useOperationFeedback } from "../../components/OperationFeedbackContext";

function formatBytes(bytes: number | null): string {
  if (bytes === null) return "—";
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}

export function ImageCatalog() {
  const { t } = useTranslation();
  const [showCacheConfirm, setShowCacheConfirm] = useState(false);
  const [imageToDelete, setImageToDelete] = useState<OfficialImage | null>(null);
  const { activeOperation, beginOperation, finishOperation } = useOperationFeedback();
  const images = useQuery({ queryKey: ["official-images"], queryFn: listOfficialImages });
  const pull = useOfficialImagePull();
  const cachePrune = useMutation({
    mutationFn: ({ operationId: _operationId }: { operationId: string }) => pruneImageCache(),
    onSuccess: () => {
      setShowCacheConfirm(false);
      void images.refetch();
    },
    onSettled: (_result, _error, variables) => finishOperation(variables.operationId),
  });
  const imageDelete = useMutation({
    mutationFn: ({ reference }: { reference: string; operationId: string }) => deleteOfficialImage(reference),
    onSuccess: () => {
      setImageToDelete(null);
      void images.refetch();
    },
    onSettled: (_result, _error, variables) => finishOperation(variables.operationId),
  });
  const records = images.data ?? [];
  const installedCount = records.filter((image) => image.installed).length;

  return (
    <section aria-labelledby="image-title">
      <header className="page-header">
        <div>
          <p className="eyebrow">LINUXSERVER.IO</p>
          <h2 id="image-title">{t("images.title")}</h2>
          <p>{t("images.summary", { installed: installedCount, total: records.length })}</p>
        </div>
        <div className="page-actions">
          <button type="button" className="secondary-button" disabled={images.isFetching || Boolean(activeOperation)} onClick={() => void images.refetch()}>{t("images.refresh")}</button>
          <button type="button" className="danger-outline-button" disabled={pull.isPending || Boolean(activeOperation)} onClick={() => { cachePrune.reset(); setShowCacheConfirm(true); }}>{t("images.clearCache")}</button>
        </div>
      </header>
      <div className="security-banner">
        <span aria-hidden="true">◇</span>
        <div><strong>{t("images.officialOnly")}</strong><p>{t("images.pullHint")}</p></div>
      </div>
      {cachePrune.isSuccess ? <p className="cache-prune-success" role="status">{t("images.cacheCleared", { count: cachePrune.data.deletedItems, size: formatBytes(cachePrune.data.spaceReclaimedBytes) })}</p> : null}
      {cachePrune.isError ? <div className="inline-error" role="alert">{t("images.cacheClearFailed")}</div> : null}
      {images.isLoading ? <p className="muted" aria-live="polite">{t("images.checking")}</p> : null}
      {images.isError ? <div className="inline-error" role="alert">{t("images.loadFailed")}</div> : null}
      <div className="image-list" role="list">
        {records.map((image) => {
          const isPulling = pull.isPending && pull.reference === image.reference;
          return (
            <article className="image-card" key={image.reference} role="listitem">
              <header>
                <div>
                  <p className="image-family">{image.distribution}</p>
                  <h3>{image.desktop}</h3>
                </div>
                <span className={image.installed ? "status-badge installed" : "status-badge"}>
                  {t(image.installed ? "images.installed" : "images.notInstalled")}
                </span>
              </header>
              <code>{image.reference}</code>
              <dl>
                <div><dt>{t("images.wayland")}</dt><dd>{t(image.waylandOnly ? "images.required" : image.waylandSupport ? "images.supported" : "images.notMarked")}</dd></div>
                <div><dt>{t("images.size")}</dt><dd>{formatBytes(image.sizeBytes)}</dd></div>
              </dl>
              <div className="image-actions">
                {image.installed ? <>
                  <button type="button" className="secondary-button" disabled>{t("images.ready")}</button>
                  <button type="button" className="danger-outline-button" disabled={pull.isPending || Boolean(activeOperation)} onClick={() => { imageDelete.reset(); setImageToDelete(image); }}>{t("images.deleteImage")}</button>
                </> : <button type="button" className="primary-button" disabled={pull.isPending || Boolean(activeOperation)} onClick={() => pull.start(image.reference)}>{t(isPulling ? "images.pulling" : "images.pull")}</button>}
              </div>
              {isPulling ? <ImagePullStatus latest={pull.latest} logs={pull.logs} isCancelling={pull.isCancelling} onCancel={() => void pull.cancel()} /> : null}
              {!pull.isPending && pull.outcome === "cancelled" && pull.reference === image.reference ? <p className="pull-cancelled" role="status">{t("images.pullCancelled")}</p> : null}
            </article>
          );
        })}
      </div>
      {pull.isError ? <div className="toast-error" role="alert">{t("images.pullFailed")}</div> : null}
      <ImageCachePruneDialog open={showCacheConfirm} pending={cachePrune.isPending} onClose={() => setShowCacheConfirm(false)} onConfirm={() => {
        const operationId = beginOperation("cachePrune", t("images.cacheTarget"));
        if (operationId) cachePrune.mutate({ operationId });
      }} />
      <ImageDeleteDialog image={imageToDelete} pending={imageDelete.isPending} error={imageDelete.isError} onClose={() => setImageToDelete(null)} onConfirm={() => {
        if (!imageToDelete) return;
        const operationId = beginOperation("imageDelete", `${imageToDelete.distribution} ${imageToDelete.desktop}`);
        if (operationId) imageDelete.mutate({ reference: imageToDelete.reference, operationId });
      }} />
    </section>
  );
}
