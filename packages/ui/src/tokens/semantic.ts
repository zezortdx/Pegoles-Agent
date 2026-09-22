/**
 * Pegoles Flux Glass — semantic color tokens.
 *
 * The ONLY consumer of the raw palette (./palette.ts) and the raw signal
 * hues (./signal.ts). Every semantic color is a reference to a raw color,
 * optionally mixed toward a second raw color and/or given an alpha, so
 * tests can prove that nothing in the system invents a color.
 *
 * Components consume these as CSS custom properties (see ./css.ts):
 * `semantic.text.secondary` → `var(--pg-text-secondary)`.
 */
import { hexToRgb, mixRgb, toCssChannels, toCssColor, type Rgb } from "./color.js";
import { palette, type PaletteToken } from "./palette.js";
import { signal, type SignalToken } from "./signal.js";

export type RawColorRef =
  | { readonly source: "palette"; readonly key: PaletteToken }
  | { readonly source: "signal"; readonly key: SignalToken };

export interface ColorToken {
  readonly base: RawColorRef;
  /** Optional second raw color the base is mixed toward. */
  readonly mix?: { readonly toward: RawColorRef; readonly amount: number };
  /** 0..1; 1 = opaque. */
  readonly alpha: number;
}

export interface ResolvedColor {
  readonly rgb: Rgb;
  readonly alpha: number;
  /** CSS value: `#RRGGBB` or `rgb(r g b / a)`. */
  readonly css: string;
}

function pal(key: PaletteToken, alpha = 1): ColorToken {
  return { base: { source: "palette", key }, alpha };
}

function sig(key: SignalToken, alpha = 1): ColorToken {
  return { base: { source: "signal", key }, alpha };
}

function blend(base: PaletteToken, toward: PaletteToken, amount: number, alpha = 1): ColorToken {
  return {
    base: { source: "palette", key: base },
    mix: { toward: { source: "palette", key: toward }, amount },
    alpha,
  };
}

export function rawHex(ref: RawColorRef): string {
  return ref.source === "palette" ? palette[ref.key] : signal[ref.key];
}

export function resolveColor(token: ColorToken): ResolvedColor {
  const base = hexToRgb(rawHex(token.base));
  const rgb = token.mix ? mixRgb(base, hexToRgb(rawHex(token.mix.toward)), token.mix.amount) : base;
  return { rgb, alpha: token.alpha, css: toCssColor(rgb, token.alpha) };
}

/** `r g b` channels of a token (alpha dropped) for `rgb(var(--x-rgb) / a)`. */
export function resolveChannels(token: ColorToken): string {
  return toCssChannels(resolveColor(token).rgb);
}

export const semantic = {
  background: {
    /** App canvas: the void Pegoles glows in. */
    primary: pal("void"),
    /** Structural regions (rails, panels) one step above the void. */
    secondary: pal("surface"),
  },
  surface: {
    default: pal("surface"),
    /** Cards and raised rows: surface lifted 4.5% toward text. */
    elevated: blend("surface", "text", 0.045),
    /** Pressed / selected rows inside elevated surfaces. */
    raised: blend("surface", "text", 0.08),
    /** Wells, the framebuffer slot backing (matches the guest background). */
    sunken: pal("void"),
  },
  text: {
    primary: pal("text"),
    secondary: pal("muted"),
    /**
     * Secondary text over translucent (clear) glass. Apple-style vibrancy:
     * brighter than flat secondary so it survives any backdrop.
     */
    vibrantSecondary: blend("muted", "text", 0.6),
    disabled: pal("muted", 0.55),
    onAccent: pal("text"),
    /** Links / active labels. Pegoles Blue itself is too dark for text (3.9:1). */
    accent: pal("electric"),
    inverse: pal("void"),
  },
  accent: {
    /** Fills: primary buttons, selection. White text on it is 4.9:1. */
    primary: pal("pegolesBlue"),
    /** Energy: active edges, live strokes. */
    active: pal("electric"),
    /** Selected backgrounds, soft emphasis. */
    soft: pal("pegolesBlue", 0.16),
    /** Glow core, focus light. */
    glow: pal("cyan"),
    deep: pal("blueDeep"),
    shadow: pal("blueShadow"),
    highlight: pal("ice"),
  },
  status: {
    success: sig("green"),
    warning: sig("amber"),
    danger: sig("red"),
    waiting: sig("orchid"),
  },
  statusSoft: {
    success: sig("green", 0.14),
    warning: sig("amber", 0.14),
    danger: sig("red", 0.14),
    waiting: sig("orchid", 0.14),
  },
  glass: {
    /** Tint bases. Opacity (the readability scrim) comes from the effects tier. */
    regular: pal("surface"),
    clear: pal("surface"),
    electric: blend("surface", "blueDeep", 0.35),
  },
  border: {
    subtle: pal("text", 0.06),
    default: pal("text", 0.1),
    strong: pal("text", 0.18),
  },
  stroke: {
    /** Bright top edge where "light" catches the material. */
    specular: pal("text", 0.16),
    electric: pal("electric", 0.5),
    /** Neutral/white interaction edge (user in control). */
    user: pal("text", 0.72),
  },
  focus: {
    ring: pal("cyan"),
    halo: pal("electric", 0.3),
  },
  shadow: {
    color: pal("void"),
  },
} as const satisfies Record<string, Record<string, ColorToken>>;

export type SemanticGroup = keyof typeof semantic;

export interface SemanticEntry {
  /** Dotted path, e.g. `text.secondary`. */
  readonly path: string;
  readonly token: ColorToken;
}

export function semanticEntries(): SemanticEntry[] {
  const out: SemanticEntry[] = [];
  for (const [group, tokens] of Object.entries(semantic)) {
    for (const [name, token] of Object.entries(tokens)) {
      out.push({ path: `${group}.${name}`, token });
    }
  }
  return out;
}
