import { createContext, useCallback, useContext, useSyncExternalStore } from "react";
import {
  DEFAULT_EFFECTS_TIER,
  effectsTiers,
  resolveTransitionStyle,
  type EffectsTier,
  type EffectsTierParams,
  type TransitionStyle,
} from "../tokens/effects.js";
import type { AmbientController, AmbientSnapshot } from "./ambient.js";

export interface FluxGlassContextValue {
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  readonly params: EffectsTierParams;
  /** `morph` (spatial) or `crossfade` (reduced motion / Minimal). */
  readonly transitionStyle: TransitionStyle;
  /** The root's ambient gate; null outside FluxGlassRoot. */
  readonly ambient: AmbientController | null;
}

export function makeFluxGlassValue(
  tier: EffectsTier,
  reducedMotion: boolean,
  ambient: AmbientController | null,
): FluxGlassContextValue {
  return {
    tier,
    reducedMotion,
    params: effectsTiers[tier],
    transitionStyle: resolveTransitionStyle(tier, reducedMotion),
    ambient,
  };
}

/**
 * Low-frequency context: changes only when the tier or the reduced-motion
 * preference changes. Never put per-frame or per-event state here.
 */
export const FluxGlassContext = createContext<FluxGlassContextValue>(
  makeFluxGlassValue(DEFAULT_EFFECTS_TIER, false, null),
);

export function useFluxGlass(): FluxGlassContextValue {
  return useContext(FluxGlassContext);
}

/** True inside a GlassSurface: nested glass must not blur again. */
export const GlassNestingContext = createContext(false);

/**
 * Live ambient gate state for diagnostics/settings UI. Changes only on
 * focus/visibility/idle transitions (rare) — never per frame.
 */
export function useAmbientState(): AmbientSnapshot | null {
  const { ambient } = useFluxGlass();
  const subscribe = useCallback(
    (onChange: () => void) => (ambient ? ambient.subscribe(onChange) : () => undefined),
    [ambient],
  );
  const read = useCallback(() => (ambient ? ambient.snapshot() : null), [ambient]);
  return useSyncExternalStore(subscribe, read, read);
}
