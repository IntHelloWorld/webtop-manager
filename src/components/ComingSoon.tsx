import { useTranslation } from "react-i18next";
import type { Section } from "./AppShell";

export function ComingSoon({ section }: { section: Exclude<Section, "environments"> }) {
  const { t } = useTranslation();
  return <section><header className="page-header"><div><p className="eyebrow">V1 CONTRACT</p><h2>{t(`nav.${section}`)}</h2></div></header><div className="empty-state"><div className="empty-glyph" aria-hidden="true">◇</div><p>{t("common.availableLater")}</p></div></section>;
}
