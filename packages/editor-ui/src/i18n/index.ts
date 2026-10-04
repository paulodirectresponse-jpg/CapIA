import { createContext, createElement, useContext, type ReactNode } from "react";
import type { Language } from "../lib/prefs";
import { en, type MessageKey } from "./en";
import { ptBR } from "./ptBR";

export type { MessageKey };

const TABLES: Record<Language, Record<MessageKey, string>> = { en, "pt-BR": ptBR };

export type Translate = (key: MessageKey, params?: Record<string, string | number>) => string;

/** `{nome}` → valor; chaves ausentes aparecem como `{nome}` (visível em teste). */
function interpolate(text: string, params?: Record<string, string | number>): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
}

export function createTranslator(lang: Language): Translate {
  const table = TABLES[lang];
  return (key, params) => interpolate(table[key], params);
}

/** Mensagem de erro da API: usa `err.<CODE>` quando existe; senão a mensagem técnica do engine. */
export function describeError(t: Translate, code: string, message: string): string {
  const key = `err.${code}` as MessageKey;
  if (key in en) return t(key, { message });
  return message.length > 0 ? message : t("err.generic");
}

interface I18nValue {
  lang: Language;
  t: Translate;
}

const Ctx = createContext<I18nValue>({ lang: "en", t: createTranslator("en") });

export function I18nProvider({ lang, children }: { lang: Language; children: ReactNode }) {
  return createElement(Ctx.Provider, { value: { lang, t: createTranslator(lang) } }, children);
}

export function useT(): Translate {
  return useContext(Ctx).t;
}

export function useLanguage(): Language {
  return useContext(Ctx).lang;
}

export const LANGUAGES: { id: Language; label: string }[] = [
  { id: "en", label: "English" },
  { id: "pt-BR", label: "Português (Brasil)" },
];
