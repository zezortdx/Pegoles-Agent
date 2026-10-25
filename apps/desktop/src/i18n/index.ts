import { useSyncExternalStore } from "react";
import type { Locale } from "../preferences/types";
import { ptBR, type CopyKey } from "./catalog";

let locale: Locale = "en";
const listeners = new Set<() => void>();
export function activateLocale(next: Locale) {
  locale = next;
  document.documentElement.lang = next;
  for (const listener of listeners) listener();
}
export function currentLocale(): Locale { return locale; }
export function useLocale(): Locale {
  return useSyncExternalStore((listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, currentLocale, () => "en");
}
/** Only compile-time product keys are accepted; interpolation remains plain text. */
export function t(key: CopyKey, values: Readonly<Record<string, string | number>> = {}): string {
  const text: string = locale === "pt-BR" ? ptBR[key] : key;
  return text.replace(/\{(\w+)\}/g, (placeholder, name: string) => String(values[name] ?? placeholder));
}
