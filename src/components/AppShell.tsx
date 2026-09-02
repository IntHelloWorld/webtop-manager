import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { ThemeSwitcher } from "./ThemeSwitcher";
import type { ActiveOperation } from "./OperationFeedbackContext";

export type Section = "environments" | "images" | "templates" | "settings";

interface AppShellProps {
  section: Section;
  onSectionChange: (section: Section) => void;
  children: ReactNode;
  activeOperation: ActiveOperation | null;
}

const sections: Section[] = ["environments", "images", "templates", "settings"];

export function AppShell({ section, onSectionChange, children, activeOperation }: AppShellProps) {
  const { t } = useTranslation();
  return <>
    <div className={activeOperation ? "app-shell app-shell-locked" : "app-shell"} aria-busy={Boolean(activeOperation)} inert={activeOperation ? true : undefined}>
      <aside className="sidebar">
        <header className="brand">
          <div className="brand-mark" aria-hidden="true">W</div>
          <div>
            <h1>{t("app.title")}</h1>
            <p>{t("app.subtitle")}</p>
          </div>
        </header>
        <nav aria-label="Primary">
          {sections.map((item) => (
            <button
              key={item}
              type="button"
              className={item === section ? "nav-item active" : "nav-item"}
              aria-current={item === section ? "page" : undefined}
              onClick={() => onSectionChange(item)}
            >
              <span className="nav-dot" aria-hidden="true" />
              {t(`nav.${item}`)}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer"><ThemeSwitcher /><LanguageSwitcher /></div>
      </aside>
      <main className="content">{children}</main>
    </div>
    {activeOperation ? <section className="operation-lock" role="status" aria-live="assertive" aria-label={t("operationFeedback.title")}>
      <span className="operation-spinner" aria-hidden="true" />
      <div><strong>{t("operationFeedback.title")}</strong><p>{t(`operationFeedback.actions.${activeOperation.kind}`, { target: activeOperation.target })}</p></div>
    </section> : null}
  </>;
}
