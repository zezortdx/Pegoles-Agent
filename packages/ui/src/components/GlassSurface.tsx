import {
  createElement,
  forwardRef,
  useCallback,
  useContext,
  useEffect,
  useRef,
  type HTMLAttributes,
  type Ref,
} from "react";
import type { ElevationLevel } from "../tokens/elevation.js";
import type { GlassMaterial } from "../tokens/effects.js";
import { registerBlurSurface } from "../runtime/blurAudit.js";
import { GlassNestingContext, useFluxGlass } from "../runtime/context.js";
import { IS_DEV } from "../runtime/env.js";
import { observeOffscreen } from "../runtime/offscreen.js";
import { cx } from "./cx.js";
import "./glass.css";

export type GlassElement = "div" | "section" | "aside" | "header" | "footer" | "nav" | "form" | "article";
export type GlassRadius = "md" | "lg" | "xl" | "xxl" | "pill";

export interface GlassState {
  /** True when rendered inside another glass surface (no second blur). */
  readonly nested: boolean;
  /** True when this surface applies backdrop-filter at the current tier. */
  readonly blurs: boolean;
  /** Attach to the surface element (feeds the dev blur audit). */
  readonly ref: (node: HTMLElement | null) => void;
  /** Data attributes that select the material CSS. */
  readonly attributes: Readonly<Record<string, string | undefined>>;
}

/**
 * Shared glass logic for GlassSurface and motion-driven surfaces
 * (CommandBar/TaskController): nesting detection, tier awareness and the
 * dev-only blur audit registration.
 */
export function useGlassSurface(material: GlassMaterial, auditLabel: string): GlassState {
  const nested = useContext(GlassNestingContext);
  const { tier, params } = useFluxGlass();
  const blurPx = params.glass[material].blurPx;
  const blurs = !nested && blurPx > 0;
  const nodeRef = useRef<HTMLElement | null>(null);

  const ref = useCallback((node: HTMLElement | null) => {
    nodeRef.current = node;
  }, []);

  useEffect(() => {
    const node = nodeRef.current;
    if (!IS_DEV || !blurs || !node) return undefined;
    const registration = registerBlurSurface({ label: auditLabel, material, blurPx });
    const unobserve = observeOffscreen(node, (offscreen) => registration.setOnscreen(!offscreen));
    return () => {
      unobserve();
      registration.unregister();
    };
  }, [blurs, auditLabel, material, blurPx, tier]);

  return {
    nested,
    blurs,
    ref,
    attributes: {
      "data-material": material,
      "data-nested": nested ? "true" : undefined,
    },
  };
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null): void {
  if (typeof ref === "function") ref(value);
  else if (ref) (ref as { current: T | null }).current = value;
}

export interface GlassSurfaceProps extends HTMLAttributes<HTMLElement> {
  readonly as?: GlassElement;
  /** regular (default) · clear · electric — see DESIGN_SYSTEM.md "Materials". */
  readonly material?: GlassMaterial;
  /** Shadow depth; floating layers use 2–3. Default 2. */
  readonly elevation?: ElevationLevel;
  /** Corner radius token. Default `xl` (the surface role). */
  readonly radius?: GlassRadius;
  /** Name shown in the dev blur audit. */
  readonly auditLabel?: string;
}

/**
 * The CSS glass backend. Tier-aware (blur/scrim from the tier variables),
 * readable by construction (tier scrims are contrast-tested), and never
 * blur-on-blur: a GlassSurface inside another renders as a flat tinted
 * layer without backdrop-filter.
 */
export const GlassSurface = forwardRef<HTMLElement, GlassSurfaceProps>(function GlassSurface(
  { as = "div", material = "regular", elevation = 2, radius = "xl", auditLabel, className, children, ...rest },
  forwardedRef,
) {
  const glass = useGlassSurface(material, auditLabel ?? `GlassSurface(${material})`);
  const glassRef = glass.ref;
  const setRef = useCallback(
    (node: HTMLElement | null) => {
      glassRef(node);
      assignRef(forwardedRef, node);
    },
    [glassRef, forwardedRef],
  );

  return createElement(
    as,
    {
      ...rest,
      ref: setRef,
      className: cx("pg-glass", className),
      ...glass.attributes,
      "data-elevation": String(elevation),
      "data-radius": radius,
    },
    <GlassNestingContext.Provider value={true}>{children}</GlassNestingContext.Provider>,
  );
});
