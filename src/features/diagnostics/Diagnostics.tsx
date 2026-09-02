import { useTranslation } from "react-i18next";
import type { ApiError, BootStatus } from "../../lib/types";

interface DiagnosticsProps {
  status?: BootStatus;
  error?: ApiError | null;
  loading: boolean;
  onRetry: () => void;
}

export function Diagnostics({ status, error, loading, onRetry }: DiagnosticsProps) {
  const { t } = useTranslation();
  const errorCode = error?.code;
  const message = errorCode === "CONTROLLER_IMAGE_MISSING"
    ? t("diagnostics.controller_image_missing")
    : status
      ? t(`diagnostics.${status.state}`)
      : t("diagnostics.generic");

  return (
    <section className="diagnostic-page" aria-labelledby="diagnostic-title">
      <div className="diagnostic-card">
        <div className="status-orbit" aria-hidden="true"><span /></div>
        <p className="eyebrow">SYSTEM CHECK</p>
        <h2 id="diagnostic-title">{t("diagnostics.title")}</h2>
        <p className="diagnostic-message">{loading ? t("common.loading") : message}</p>
        {status?.dockerVersion ? (
          <p className="technical-detail">
            {t("diagnostics.versions", {
              docker: status.dockerVersion,
              api: status.dockerApiVersion ?? "—",
            })}
          </p>
        ) : null}
        {status?.socketWorldWritable ? (
          <div className="warning" role="alert">{t("diagnostics.worldWritable")}</div>
        ) : null}
        <p className="muted">{t("diagnostics.noAutoFix")}</p>
        <button type="button" className="primary-button" onClick={onRetry} disabled={loading}>
          {t("common.retry")}
        </button>
      </div>
    </section>
  );
}
