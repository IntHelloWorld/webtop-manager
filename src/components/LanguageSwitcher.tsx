import { useTranslation } from "react-i18next";
import { AppSelect } from "./AppSelect";

export function LanguageSwitcher() {
  const { i18n } = useTranslation();
  const language = i18n.resolvedLanguage === "zh-CN" ? "zh-CN" : "en";

  return (
    <div className="language-switcher">
      <span className="sr-only">Language</span>
      <AppSelect
        value={language}
        onChange={(value) => void i18n.changeLanguage(value)}
        ariaLabel="Language"
        options={[{ value: "zh-CN", label: "简体中文" }, { value: "en", label: "English" }]}
      />
    </div>
  );
}
