import { fluxGlassStyleSheet } from "../tokens/css.js";

export const TOKEN_STYLE_ID = "pegoles-flux-glass-tokens";

/**
 * Inject the generated token stylesheet once per document (idempotent).
 * Returns the <style> element.
 */
export function ensureFluxGlassStyleSheet(doc: Document = document): HTMLStyleElement {
  const existing = doc.getElementById(TOKEN_STYLE_ID);
  if (existing instanceof HTMLStyleElement) return existing;
  const style = doc.createElement("style");
  style.id = TOKEN_STYLE_ID;
  style.textContent = fluxGlassStyleSheet();
  doc.head.prepend(style);
  return style;
}
