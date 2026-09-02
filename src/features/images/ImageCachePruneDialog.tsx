import { useTranslation } from "react-i18next";

interface ImageCachePruneDialogProps {
  open: boolean;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}

export function ImageCachePruneDialog({ open, pending, onClose, onConfirm }: ImageCachePruneDialogProps) {
  const { t } = useTranslation();
  if (!open) return null;

  return (
    <div className="dialog-backdrop cache-prune-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !pending) onClose(); }}>
      <section className="dialog small" role="alertdialog" aria-modal="true" aria-labelledby="cache-prune-title" aria-describedby="cache-prune-description">
        <header className="dialog-header">
          <div><p className="eyebrow">DOCKER CACHE</p><h2 id="cache-prune-title">{t("images.clearCacheTitle")}</h2></div>
          <button type="button" className="icon-button" disabled={pending} onClick={onClose} aria-label={t("common.close")}>×</button>
        </header>
        <p id="cache-prune-description" className="dialog-copy">{t("images.clearCachePrompt")}</p>
        <div className="warning" role="note">{t("images.clearCacheScope")}</div>
        <footer className="dialog-actions">
          <button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button>
          <button type="button" className="danger-button" disabled={pending} onClick={onConfirm}>{t(pending ? "images.clearingCache" : "images.confirmClearCache")}</button>
        </footer>
      </section>
    </div>
  );
}
