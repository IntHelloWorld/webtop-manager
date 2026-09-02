import { useState } from "react";
import { useTranslation } from "react-i18next";
import { getThemePreference, setThemePreference, type ThemePreference } from "../lib/theme";
import { AppSelect } from "./AppSelect";

export function ThemeSwitcher() {
  const { t } = useTranslation();
  const [theme, setTheme] = useState<ThemePreference>(getThemePreference);

  const changeTheme = (preference: ThemePreference) => {
    setTheme(preference);
    setThemePreference(preference);
  };

  return (
    <div className="theme-switcher">
      <span>{t("appearance.theme")}</span>
      <AppSelect
        value={theme}
        onChange={(value) => changeTheme(value as ThemePreference)}
        ariaLabel={t("appearance.theme")}
        options={[
          { value: "system", label: t("appearance.system") },
          { value: "light", label: t("appearance.light") },
          { value: "dark", label: t("appearance.dark") },
        ]}
      />
    </div>
  );
}
