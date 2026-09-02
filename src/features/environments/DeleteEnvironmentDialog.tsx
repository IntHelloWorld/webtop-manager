import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { EnvironmentRecord } from "../../lib/types";

interface DeleteEnvironmentDialogProps {
  environment: EnvironmentRecord | null;
  onClose: () => void;
  onConfirm: (deleteData: boolean) => void;
}

export function DeleteEnvironmentDialog({ environment, onClose, onConfirm }: DeleteEnvironmentDialogProps) {
  const { t } = useTranslation();
  const [confirmation, setConfirmation] = useState("");
  const [keepData, setKeepData] = useState(false);
  useEffect(() => {
    if (!environment) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [environment, onClose]);
  if (!environment) return null;
  const matches = confirmation === environment.name;

  return (
    <div className="dialog-backdrop">
      <section className="dialog small" role="alertdialog" aria-modal="true" aria-labelledby="delete-title">
        <header className="dialog-header"><h2 id="delete-title">{t("environments.deleteTitle")}</h2></header>
        <p>{t("environments.deletePrompt", { name: environment.name })}</p>
        <label className="field full"><span>{environment.name}</span><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoFocus /></label>
        {confirmation && !matches ? <p className="field-error">{t("environments.nameMismatch")}</p> : null}
        <label className="check-row"><input type="checkbox" checked={keepData} onChange={(event) => setKeepData(event.target.checked)} /><span>{t("environments.keepData")}</span></label>
        <footer className="dialog-actions">
          <button type="button" className="secondary-button" onClick={onClose}>{t("common.cancel")}</button>
          <button type="button" className="danger-button" disabled={!matches} onClick={() => onConfirm(!keepData)}>{t("common.delete")}</button>
        </footer>
      </section>
    </div>
  );
}
