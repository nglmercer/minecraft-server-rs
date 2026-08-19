import { createContext } from "preact";
import { useContext, useEffect, useMemo, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { en, type Dictionary } from "./en";
import { es } from "./es";

/** Every language the panel ships with. */
export const LANGUAGES = {
  en: { label: "English", dictionary: en },
  es: { label: "Español", dictionary: es },
} as const;

export type Language = keyof typeof LANGUAGES;

const STORAGE_KEY = "mcpanel.language";

/** The language to start in: a stored choice, else the browser's, else English. */
function initialLanguage(): Language {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && stored in LANGUAGES) return stored as Language;

  for (const tag of navigator.languages ?? [navigator.language]) {
    const base = tag.split("-")[0];
    if (base in LANGUAGES) return base as Language;
  }
  return "en";
}

/**
 * Dotted key into the dictionary, e.g. `"server.tabs.console"`.
 *
 * Typing this against the English dictionary means a typo or a key removed from
 * `en.ts` is a compile error rather than a `{missing}` in the UI.
 */
type Leaves<T> = T extends string
  ? ""
  : {
      [K in keyof T & string]: Leaves<T[K]> extends "" ? K : `${K}.${Leaves<T[K]>}`;
    }[keyof T & string];

export type TranslationKey = Leaves<Dictionary>;

/** Values substituted into `{placeholders}`. */
export type Params = Record<string, string | number>;

function lookup(dictionary: Dictionary, key: string): string {
  let node: unknown = dictionary;
  for (const part of key.split(".")) {
    if (typeof node !== "object" || node === null) return key;
    node = (node as Record<string, unknown>)[part];
  }
  // Falling back to the key makes a missing string obvious rather than blank.
  return typeof node === "string" ? node : key;
}

function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

interface I18n {
  language: Language;
  setLanguage: (language: Language) => void;
  t: (key: TranslationKey, params?: Params) => string;
}

const I18nContext = createContext<I18n | null>(null);

export function I18nProvider({ children }: { children: ComponentChildren }) {
  const [language, setLanguageState] = useState<Language>(initialLanguage);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, language);
    // Screen readers and `lang`-scoped CSS both rely on this being accurate.
    document.documentElement.lang = language;
  }, [language]);

  const value = useMemo<I18n>(() => {
    const dictionary = LANGUAGES[language].dictionary;
    return {
      language,
      setLanguage: setLanguageState,
      t: (key, params) => interpolate(lookup(dictionary, key), params),
    };
  }, [language]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** Translate, and read or change the active language. */
export function useI18n(): I18n {
  const context = useContext(I18nContext);
  if (!context) throw new Error("useI18n must be used inside an I18nProvider");
  return context;
}

/** Just the translate function, which is what most components need. */
export function useT() {
  return useI18n().t;
}
