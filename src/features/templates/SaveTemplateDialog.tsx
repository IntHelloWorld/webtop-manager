import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { EnvironmentRecord, TemplatePreflight } from "../../lib/types";

function bytes(value: number | null): string {
  if (value === null) return "—";
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(0)} KiB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MiB`;
  return `${(value / 1024 ** 3).toFixed(2)} GiB`;
}

export function SaveTemplateDialog({ environment, preflight, pending, onClose, onConfirm }: {
  environment: EnvironmentRecord;
  preflight: TemplatePreflight;
  pending: boolean;
  onClose: () => void;
  onConfirm: (name: string, confirmSpace: boolean) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(`${environment.name} template`);
  const [sensitive, setSensitive] = useState(false);
  const [space, setSpace] = useState(false);
  return <div className="dialog-backdrop" role="presentation">
    <section className="dialog" role="dialog" aria-modal="true" aria-labelledby="save-template-title">
      <header className="dialog-header"><div><p className="eyebrow">PORTABLE TEMPLATE</p><h2 id="save-template-title">{t("templates.saveTitle")}</h2></div><button type="button" className="icon-button" aria-label={t("common.close")} disabled={pending} onClick={onClose}>×</button></header>
      <label className="field"><span>{t("templates.name")}</span><input value={name} maxLength={64} onChange={(event) => setName(event.target.value)} autoFocus /></label>
      <dl className="preflight-grid">
        <div><dt>{t("templates.systemChanges")}</dt><dd>{bytes(preflight.systemChangeBytes)}</dd></div>
        <div><dt>{t("templates.configSize")}</dt><dd>{bytes(preflight.configOriginalBytes)}</dd></div>
        <div><dt>{t("templates.conservativeEstimate")}</dt><dd>{bytes(preflight.conservativeTotalBytes)}</dd></div>
        <div><dt>{t("templates.availableSpace")}</dt><dd>{bytes(preflight.availableBytes)}</dd></div>
        <div><dt>{t("templates.fileCount")}</dt><dd>{preflight.fileCount}</dd></div>
        <div><dt>{t("templates.skippedSpecial")}</dt><dd>{preflight.skippedSpecialFiles}</dd></div>
      </dl>
      {preflight.sensitivePaths.length ? <div className="warning"><strong>{t("templates.sensitivePaths")}</strong><p>{preflight.sensitivePaths.slice(0, 8).join(", ")}</p></div> : null}
      <label className="check-row"><input type="checkbox" checked={sensitive} onChange={(event) => setSensitive(event.target.checked)} /><span>{t("templates.sensitiveConfirm")}<small>{t("templates.unencryptedWarning")}</small></span></label>
      {preflight.insufficientSpaceWarning ? <label className="check-row warning"><input type="checkbox" checked={space} onChange={(event) => setSpace(event.target.checked)} /><span>{t("templates.spaceWarning")}</span></label> : null}
      <p className="muted">{t("templates.uncancellableStage")}</p>
      <footer className="dialog-actions"><button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button><button type="button" className={pending ? "primary-button is-working" : "primary-button"} disabled={pending || !name.trim() || !sensitive || (preflight.insufficientSpaceWarning && !space)} onClick={() => onConfirm(name.trim(), space)}>{t(pending ? "templates.saving" : "templates.save")}</button></footer>
    </section>
  </div>;
}
