import { useTranslation } from "react-i18next";
import type { OfficialImage } from "../../lib/types";

interface ImageDeleteDialogProps {
  image: OfficialImage | null;
  pending: boolean;
  error: boolean;
  onClose: () => void;
  onConfirm: () => void;
}

export function ImageDeleteDialog({ image, pending, error, onClose, onConfirm }: ImageDeleteDialogProps) {
  const { t } = useTranslation();
  if (!image) return null;

  return <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !pending) onClose(); }}>
    <section className="dialog small" role="alertdialog" aria-modal="true" aria-labelledby="image-delete-title" aria-describedby="image-delete-description">
      <header className="dialog-header">
        <div><p className="eyebrow">DOCKER IMAGE</p><h2 id="image-delete-title">{t("images.deleteTitle")}</h2></div>
        <button type="button" className="icon-button" disabled={pending} onClick={onClose} aria-label={t("common.close")}>×</button>
      </header>
      <p id="image-delete-description" className="dialog-copy">{t("images.deletePrompt", { image: `${image.distribution} ${image.desktop}` })}</p>
      <code className="delete-target">{image.reference}</code>
      <div className="warning" role="note">{t("images.deleteScope")}</div>
      {error ? <div className="inline-error" role="alert">{t("images.deleteFailed")}</div> : null}
      <footer className="dialog-actions">
        <button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button>
        <button type="button" className={pending ? "danger-button is-working" : "danger-button"} disabled={pending} onClick={onConfirm}>{t(pending ? "images.deleting" : "images.confirmDelete")}</button>
      </footer>
    </section>
  </div>;
}
