import { useFluxGlass } from "../runtime/context.js";
import { usePrefersReducedMotion } from "../runtime/reducedMotion.js";
import type { EffectsTier } from "../tokens/effects.js";

export interface ResolvedEffects {
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
}

/**
 * Effects tier + reduced motion for a presence/cursor component: explicit
 * props win (design lab previews), else the Flux Glass root context. Outside
 * a FluxGlassRoot the OS reduced-motion preference is still honored.
 */
export function useResolvedEffects(tier?: EffectsTier, reducedMotion?: boolean): ResolvedEffects {
  const ctx = useFluxGlass();
  const systemReducedMotion = usePrefersReducedMotion();
  const insideRoot = ctx.ambient !== null;
  return {
    tier: tier ?? ctx.tier,
    reducedMotion: reducedMotion ?? (insideRoot ? ctx.reducedMotion : systemReducedMotion),
  };
}
