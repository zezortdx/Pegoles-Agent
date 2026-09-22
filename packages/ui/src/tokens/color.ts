/**
 * Pegoles Flux Glass — tiny color math used by the token layer and its
 * tests (contrast guarantees, "status is never blue" checks). Pure, no DOM.
 */

export interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

const HEX_RE = /^#([0-9a-f]{6})$/i;

export function hexToRgb(hex: string): Rgb {
  const match = HEX_RE.exec(hex);
  if (!match || match[1] === undefined) {
    throw new Error(`Invalid hex color: ${hex}`);
  }
  const value = match[1];
  return {
    r: parseInt(value.slice(0, 2), 16),
    g: parseInt(value.slice(2, 4), 16),
    b: parseInt(value.slice(4, 6), 16),
  };
}

function channelToHex(c: number): string {
  return Math.round(Math.min(255, Math.max(0, c)))
    .toString(16)
    .padStart(2, "0");
}

export function rgbToHex(c: Rgb): string {
  return `#${channelToHex(c.r)}${channelToHex(c.g)}${channelToHex(c.b)}`.toUpperCase();
}

/** Linear interpolation from `a` toward `b` by `t` (0..1). */
export function mixRgb(a: Rgb, b: Rgb, t: number): Rgb {
  return {
    r: a.r + (b.r - a.r) * t,
    g: a.g + (b.g - a.g) * t,
    b: a.b + (b.b - a.b) * t,
  };
}

/** Source-over compositing of `top` at `alpha` onto an opaque `bottom`. */
export function composite(top: Rgb, alpha: number, bottom: Rgb): Rgb {
  return mixRgb(bottom, top, alpha);
}

function linearize(channel: number): number {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.x relative luminance. */
export function relativeLuminance(c: Rgb): number {
  return 0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b);
}

/** WCAG 2.x contrast ratio (1..21). */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** HSL hue in degrees (0..360) and saturation (0..1). */
export function hueSaturation(c: Rgb): { hue: number; saturation: number } {
  const r = c.r / 255;
  const g = c.g / 255;
  const b = c.b / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  if (delta === 0) return { hue: 0, saturation: 0 };
  const lightness = (max + min) / 2;
  const saturation = delta / (1 - Math.abs(2 * lightness - 1));
  let hue: number;
  if (max === r) hue = ((g - b) / delta) % 6;
  else if (max === g) hue = (b - r) / delta + 2;
  else hue = (r - g) / delta + 4;
  hue *= 60;
  return { hue: hue < 0 ? hue + 360 : hue, saturation };
}

/**
 * "Blue" in the Flux Glass sense: a saturated hue between cyan-green and
 * blue-violet (165°–270°). Status colors must never fall in this band —
 * blue is reserved for Pegoles' presence and energy.
 */
export function isBlue(c: Rgb): boolean {
  const { hue, saturation } = hueSaturation(c);
  return saturation > 0.15 && hue >= 165 && hue <= 270;
}

function round(value: number, digits: number): number {
  const f = 10 ** digits;
  return Math.round(value * f) / f;
}

/** CSS serialization: `#RRGGBB` when opaque, `rgb(r g b / a)` otherwise. */
export function toCssColor(c: Rgb, alpha = 1): string {
  if (alpha >= 1) return rgbToHex(c);
  return `rgb(${Math.round(c.r)} ${Math.round(c.g)} ${Math.round(c.b)} / ${round(alpha, 3)})`;
}

/** Space-separated channels for `rgb(var(--x-rgb) / a)` composition in CSS. */
export function toCssChannels(c: Rgb): string {
  return `${Math.round(c.r)} ${Math.round(c.g)} ${Math.round(c.b)}`;
}
