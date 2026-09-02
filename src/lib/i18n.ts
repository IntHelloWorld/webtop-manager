import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { en } from "../locales/en";
import { zhCN } from "../locales/zh-CN";

const initialLanguage = "zh-CN";

void i18n.use(initReactI18next).init({
  resources: { en, "zh-CN": zhCN },
  lng: initialLanguage,
  fallbackLng: "zh-CN",
  interpolation: { escapeValue: false },
});

export default i18n;
