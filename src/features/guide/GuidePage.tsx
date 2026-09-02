import { useTranslation } from "react-i18next";
import type { Section } from "../../components/AppShell";

interface GuidePageProps {
  onNavigate: (section: Section) => void;
}

const firstEnvironmentSteps = ["check", "open", "image", "configure", "finish"] as const;
const remoteSetupSteps = ["prepare", "settings", "generate", "deploy", "firewall", "connect", "publish"] as const;
const recoverySteps = ["detect", "recover", "remote", "test"] as const;

export function GuidePage({ onNavigate }: GuidePageProps) {
  const { t } = useTranslation();

  return (
    <section className="guide-page" aria-labelledby="app-guide-title">
      <header className="page-header">
        <div>
          <p className="eyebrow">GETTING STARTED</p>
          <h2 id="app-guide-title">{t("appGuide.title")}</h2>
          <p>{t("appGuide.description")}</p>
        </div>
      </header>

      <article className="guide-feature-card" aria-labelledby="first-environment-title">
        <header className="guide-card-header">
          <span className="guide-card-number" aria-hidden="true">01</span>
          <div>
            <p className="eyebrow">{t("appGuide.start.eyebrow")}</p>
            <h3 id="first-environment-title">{t("appGuide.start.title")}</h3>
            <p>{t("appGuide.start.description")}</p>
          </div>
        </header>
        <h4 className="guide-phase-title">{t("appGuide.start.localTitle")}</h4>
        <ol className="app-guide-steps">
          {firstEnvironmentSteps.map((step) => (
            <li key={step}>
              <span className="app-guide-step-marker" aria-hidden="true" />
              <div><strong>{t(`appGuide.start.steps.${step}.title`)}</strong><p>{t(`appGuide.start.steps.${step}.description`)}</p></div>
            </li>
          ))}
        </ol>
        <div className="guide-card-actions">
          <button type="button" className="primary-button" onClick={() => onNavigate("environments")}>{t("appGuide.actions.create")}</button>
          <button type="button" className="secondary-button" onClick={() => onNavigate("images")}>{t("appGuide.actions.images")}</button>
        </div>
        <section className="guide-publication-setup" aria-labelledby="frp-setup-title">
          <header className="guide-card-header">
            <span className="guide-card-number" aria-hidden="true">02</span>
            <div>
              <p className="eyebrow">FRP</p>
              <h4 id="frp-setup-title">{t("appGuide.start.remoteTitle")}</h4>
              <p>{t("appGuide.start.remoteDescription")}</p>
            </div>
          </header>
          <ol className="guide-publication-steps">
            {remoteSetupSteps.map((step, index) => (
              <li key={step}>
                <span aria-hidden="true">{String(index + 1).padStart(2, "0")}</span>
                <div><strong>{t(`appGuide.start.remoteSteps.${step}.title`)}</strong><p>{t(`appGuide.start.remoteSteps.${step}.description`)}</p></div>
              </li>
            ))}
          </ol>
          <p className="guide-security-note">{t("appGuide.start.remoteNote")}</p>
          <button type="button" className="secondary-button" onClick={() => onNavigate("settings")}>{t("appGuide.actions.settings")}</button>
        </section>
      </article>

      <div className="guide-topic-grid">
        <article className="guide-topic-card">
          <span className="guide-topic-icon" aria-hidden="true">⌁</span>
          <h3>{t("appGuide.local.title")}</h3>
          <p>{t("appGuide.local.description")}</p>
          <ul><li>{t("appGuide.local.credentials")}</li><li>{t("appGuide.local.certificate")}</li><li>{t("appGuide.local.data")}</li></ul>
          <button type="button" className="secondary-button small-button" onClick={() => onNavigate("environments")}>{t("appGuide.actions.environments")}</button>
        </article>

        <article className="guide-topic-card">
          <span className="guide-topic-icon" aria-hidden="true">◇</span>
          <h3>{t("appGuide.templates.title")}</h3>
          <p>{t("appGuide.templates.description")}</p>
          <ul><li>{t("appGuide.templates.stop")}</li><li>{t("appGuide.templates.export")}</li><li>{t("appGuide.templates.secret")}</li></ul>
          <button type="button" className="secondary-button small-button" onClick={() => onNavigate("templates")}>{t("appGuide.actions.templates")}</button>
        </article>

        <article className="guide-topic-card guide-recovery-card" aria-labelledby="token-recovery-title">
          <span className="guide-topic-icon warning-icon" aria-hidden="true">!</span>
          <h3 id="token-recovery-title">{t("appGuide.recovery.title")}</h3>
          <p>{t("appGuide.recovery.description")}</p>
          <ol className="recovery-steps">
            {recoverySteps.map((step) => <li key={step}>{t(`appGuide.recovery.steps.${step}`)}</li>)}
          </ol>
          <p className="guide-recovery-note">{t("appGuide.recovery.note")}</p>
          <button type="button" className="secondary-button small-button" onClick={() => onNavigate("settings")}>{t("appGuide.actions.recovery")}</button>
        </article>
      </div>
    </section>
  );
}
