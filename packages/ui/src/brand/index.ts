/**
 * Pegoles brand geometry — the vector trace of the logo PNG
 * (assets/brand/source/pegoles-mark-source.png), fitted and verified by
 * scripts/brand (see assets/brand/README.md for IoU numbers).
 *
 * Do not redesign, recolor, or replace the mark with an icon-library
 * symbol. Use `PegolesMark` (presence) for the living mark and these
 * constants for static/derived uses (masks, favicons, placeholders).
 */
import { MARK_GEOMETRY } from "./markGeometry.generated.js";

export { MARK_GEOMETRY, type MarkGeometry } from "./markGeometry.generated.js";

/** Square artboard size (units = source-PNG pixels). */
export const MARK_ARTBOARD = MARK_GEOMETRY.size;

/** Ring as one even-odd path (outer contour + inner opening). */
export const MARK_RING_PATH = `${MARK_GEOMETRY.paths.outer}${MARK_GEOMETRY.paths.inner}`;

/** Both eyes as one path. */
export const MARK_EYES_PATH = `${MARK_GEOMETRY.paths.eyeLeft}${MARK_GEOMETRY.paths.eyeRight}`;

export interface MarkGlyphOptions {
  /** Fill color. Default black (right for masks and template icons). */
  readonly color?: string;
  /** Include the eyes (default true). */
  readonly eyes?: boolean;
  /** Include the ring (default true). */
  readonly ring?: boolean;
}

/** Flat single-color SVG document of the mark (artboard viewBox). */
export function markGlyphSvg(options: MarkGlyphOptions = {}): string {
  const color = options.color ?? "#000";
  const parts: string[] = [];
  if (options.ring ?? true) parts.push(`<path fill-rule="evenodd" d="${MARK_RING_PATH}"/>`);
  if (options.eyes ?? true) parts.push(`<path d="${MARK_EYES_PATH}"/>`);
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${MARK_ARTBOARD} ${MARK_ARTBOARD}" fill="${color}">` +
    parts.join("") +
    "</svg>"
  );
}

/** `url("data:image/svg+xml,…")` for CSS `mask-image` / `background-image`. */
export function svgDataUrl(svg: string): string {
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
}

/** CSS mask for effects that must stay inside the ring. */
export const MARK_RING_MASK = svgDataUrl(markGlyphSvg({ eyes: false }));
