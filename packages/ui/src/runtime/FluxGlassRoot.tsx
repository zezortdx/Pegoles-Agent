import {
  createElement,
  useEffect,
  useInsertionEffect,
  useLayoutEffect,
  useMemo,
  useState,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import { LazyMotion, MotionConfig } from "motion/react";
import type { EffectsTier } from "../tokens/effects.js";
import { AmbientController, DEFAULT_IDLE_AFTER_MS } from "./ambient.js";
import { FluxGlassContext, makeFluxGlassValue, useFluxGlass } from "./context.js";
import { usePrefersReducedMotion } from "./reducedMotion.js";
import { ensureFluxGlassStyleSheet } from "./styleSheet.js";
import "../styles/base.css";

const loadMotionFeatures = () => import("./motionFeatures.js").then((mod) => mod.default);

export interface FluxGlassRootProps {
  /** Resolved tier — compute it with `selectEffectsTier`. */
  readonly tier: EffectsTier;
  /**
   * Reduced-motion override for previews (design lab). Leave undefined in
   * the app: the OS preference is followed live and never changed.
   */
  readonly reducedMotion?: boolean;
  /** Active work in progress (task running, computer booting): prevents idle. */
  readonly busy?: boolean;
  /** Host window hidden/minimized signal (e.g. from Tauri). */
  readonly windowHidden?: boolean;
  /** No-input interval before ambient life sleeps. Default 60 s. */
  readonly idleAfterMs?: number;
  /** Element that receives the gate attributes. Default: <html>. */
  readonly target?: HTMLElement;
  readonly children?: ReactNode;
}

/**
 * Mount once at the app root. Injects the token stylesheet, runs the
 * AmbientController (Idle GPU gate) on `target`, and provides the
 * low-frequency Flux Glass context (tier, reduced motion, tier params)
 * plus lazily loaded motion features for shared-layout morphs.
 */
export function FluxGlassRoot({
  tier,
  reducedMotion,
  busy = false,
  windowHidden = false,
  idleAfterMs = DEFAULT_IDLE_AFTER_MS,
  target,
  children,
}: FluxGlassRootProps) {
  const systemReducedMotion = usePrefersReducedMotion();
  const effectiveReducedMotion = reducedMotion ?? systemReducedMotion;
  const [controller, setController] = useState<AmbientController | null>(null);

  useInsertionEffect(() => {
    ensureFluxGlassStyleSheet();
  }, []);

  // Created before paint so the tier attributes exist on the first frame.
  // Tier/motion/busy/hidden are applied by the effects below, so they are
  // intentionally not dependencies here.
  useLayoutEffect(() => {
    const ambient = new AmbientController({
      root: target,
      tier,
      reducedMotion: effectiveReducedMotion,
      busy,
      externallyHidden: windowHidden,
      idleAfterMs,
    });
    ambient.start();
    setController(ambient);
    return () => {
      ambient.stop();
      setController(null);
    };
  }, [target, idleAfterMs]);

  useLayoutEffect(() => {
    controller?.setTier(tier);
  }, [controller, tier]);

  useLayoutEffect(() => {
    controller?.setReducedMotion(effectiveReducedMotion);
  }, [controller, effectiveReducedMotion]);

  useEffect(() => {
    controller?.setBusy(busy);
  }, [controller, busy]);

  useEffect(() => {
    controller?.setExternallyHidden(windowHidden);
  }, [controller, windowHidden]);

  const value = useMemo(
    () => makeFluxGlassValue(tier, effectiveReducedMotion, controller),
    [tier, effectiveReducedMotion, controller],
  );

  return (
    <FluxGlassContext.Provider value={value}>
      <MotionConfig reducedMotion={effectiveReducedMotion ? "always" : "never"}>
        <LazyMotion features={loadMotionFeatures}>{children}</LazyMotion>
      </MotionConfig>
    </FluxGlassContext.Provider>
  );
}

export interface EffectsScopeProps extends HTMLAttributes<HTMLElement> {
  readonly tier: EffectsTier;
  readonly as?: "div" | "section" | "article";
}

/**
 * Renders a subtree with a different tier (tier CSS variables are keyed
 * on `[data-effects-tier]`, so they cascade from here). Used by the
 * design lab to compare tiers side by side.
 */
export function EffectsScope({ tier, as = "div", children, ...rest }: EffectsScopeProps) {
  const parent = useFluxGlass();
  const value = useMemo(
    () => makeFluxGlassValue(tier, parent.reducedMotion, parent.ambient),
    [tier, parent.reducedMotion, parent.ambient],
  );
  return createElement(
    as,
    { ...rest, "data-effects-tier": tier },
    <FluxGlassContext.Provider value={value}>{children}</FluxGlassContext.Provider>,
  );
}
