import { createContext, useContext, type ReactNode } from "react";
import type { EffectsTier } from "@pegoles/ui";

/** The person's choice in Settings. */
export type PresenceQuality = "auto" | "full" | "reduced";

const PresenceQualityContext = createContext<PresenceQuality>("auto");

export function PresenceQualityProvider({ quality, children }: { quality: PresenceQuality; children?: ReactNode }) {
  return <PresenceQualityContext.Provider value={quality}>{children}</PresenceQualityContext.Provider>;
}

export function usePresenceQuality(): PresenceQuality {
  return useContext(PresenceQualityContext);
}

/** Something else needs the GPU (its computer's screen is visible): every presence renders lighter. */
const PresenceGpuShareContext = createContext(false);

export function PresenceGpuShareProvider({ shared, children }: { shared: boolean; children?: ReactNode }) {
  return <PresenceGpuShareContext.Provider value={shared}>{children}</PresenceGpuShareContext.Provider>;
}

export function usePresenceGpuShare(): boolean {
  return useContext(PresenceGpuShareContext);
}

/** Smallest presence that gets the WebGL renderer; below this SVG is sharper anyway. */
export const GL_MIN_SIZE = 48;

export interface RendererInput {
  readonly quality: PresenceQuality;
  readonly tier: EffectsTier;
  readonly size: number;
  readonly webgl2: boolean;
  readonly disabled: boolean;
}

/**
 * Which renderer a presence should use. Pure, so the policy is tested:
 * Reduced (or a reduced/minimal effects tier under Auto) means SVG; Full
 * means WebGL wherever it is available; small marks are always SVG.
 */
export function wantsGl({ quality, tier, size, webgl2, disabled }: RendererInput): boolean {
  if (!webgl2 || disabled || size < GL_MIN_SIZE) return false;
  if (quality === "reduced" || tier === "minimal") return false;
  if (quality === "full") return true;
  return tier === "full";
}
