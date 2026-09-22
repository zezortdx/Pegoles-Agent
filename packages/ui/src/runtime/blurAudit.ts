/**
 * Dev-only blur audit: counts mounted glass surfaces that actually apply
 * `backdrop-filter`, and how many of them are on screen right now.
 *
 * Backdrop blur is the most expensive thing Flux Glass does (every frame
 * the content behind it changes, the GPU re-samples and blurs it). The
 * tier's `maxBackdropSurfaces` is the budget; the design lab renders a
 * readout that flags overruns — no console noise. `GlassSurface`
 * registers only when `IS_DEV` is true, so production builds pay nothing.
 */
import { useSyncExternalStore } from "react";
import type { GlassMaterial } from "../tokens/effects.js";

export interface BlurSurfaceRecord {
  readonly id: number;
  readonly label: string;
  readonly material: GlassMaterial;
  readonly blurPx: number;
  readonly onscreen: boolean;
}

export interface BlurAuditSnapshot {
  readonly surfaces: readonly BlurSurfaceRecord[];
  /** Mounted backdrop-filter surfaces. */
  readonly mounted: number;
  /** Mounted AND intersecting the viewport. */
  readonly visible: number;
}

type Listener = () => void;

let nextId = 1;
const records = new Map<number, BlurSurfaceRecord>();
const listeners = new Set<Listener>();
let snapshot: BlurAuditSnapshot = { surfaces: [], mounted: 0, visible: 0 };

function publish(): void {
  const surfaces = [...records.values()];
  snapshot = {
    surfaces,
    mounted: surfaces.length,
    visible: surfaces.filter((s) => s.onscreen).length,
  };
  for (const listener of listeners) listener();
}

export interface BlurRegistration {
  setOnscreen(onscreen: boolean): void;
  unregister(): void;
}

export function registerBlurSurface(info: {
  label: string;
  material: GlassMaterial;
  blurPx: number;
}): BlurRegistration {
  const id = nextId++;
  records.set(id, { id, ...info, onscreen: true });
  publish();
  return {
    setOnscreen(onscreen: boolean) {
      const current = records.get(id);
      if (!current || current.onscreen === onscreen) return;
      records.set(id, { ...current, onscreen });
      publish();
    },
    unregister() {
      if (records.delete(id)) publish();
    },
  };
}

export function getBlurAudit(): BlurAuditSnapshot {
  return snapshot;
}

export function subscribeBlurAudit(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Live audit snapshot for dev tooling (design lab readout). */
export function useBlurAudit(): BlurAuditSnapshot {
  return useSyncExternalStore(subscribeBlurAudit, getBlurAudit, getBlurAudit);
}

/** Test hook. */
export function resetBlurAuditForTests(): void {
  records.clear();
  publish();
}
