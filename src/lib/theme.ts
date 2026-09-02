export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "webtop-manager.theme.v1";

function isThemePreference(value: string | null): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function getThemePreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return isThemePreference(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

export function applyThemePreference(preference = getThemePreference()): void {
  const resolved = preference === "system"
    ? (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark")
    : preference;
  document.documentElement.dataset.theme = resolved;
  document.documentElement.style.colorScheme = resolved;
}

export function setThemePreference(preference: ThemePreference): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Theme changes still apply when storage is unavailable.
  }
  applyThemePreference(preference);
}

export function watchSystemTheme(): () => void {
  const media = window.matchMedia("(prefers-color-scheme: light)");
  const update = () => {
    if (getThemePreference() === "system") applyThemePreference("system");
  };
  media.addEventListener("change", update);
  return () => media.removeEventListener("change", update);
}
