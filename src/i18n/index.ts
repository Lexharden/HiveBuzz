import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import es from "./es.json";

/** Último idioma usado: el splash del arranque sale ya en él (las preferencias aún no se han leído). */
const LANG_KEY = "hivebuzz.lang";
function lastLanguage(): string {
  try {
    const saved = localStorage.getItem(LANG_KEY);
    return saved === "en" || saved === "es" ? saved : "es";
  } catch {
    return "es";
  }
}

void i18n.use(initReactI18next).init({
  resources: { es: { translation: es }, en: { translation: en } },
  lng: lastLanguage(),
  fallbackLng: "es",
  interpolation: { escapeValue: false },
});

i18n.on("languageChanged", (lng) => {
  try {
    localStorage.setItem(LANG_KEY, lng);
  } catch {
    // Sin almacenamiento solo se pierde el idioma del splash.
  }
});

export default i18n;
