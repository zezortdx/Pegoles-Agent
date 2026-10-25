/**
 * Where AgentCursor actions come from, and the production mount point.
 *
 * NO FAKE AUTONOMOUS CURSOR IN PRODUCTION. A source relays REAL agent
 * actions only (the desktop app maps Core's `action_started` events, i.e.
 * actions that passed policy and are executing, see
 * apps/desktop/src/lib/agentCursorFeed.ts). Without actions the layer stays
 * hidden. Simulated fixtures exist only in the design lab.
 */
import { useEffect, useRef, type CSSProperties } from "react";
import type { EffectsTier } from "../tokens/effects.js";
import { AgentCursorLayer, type AgentCursorHandle } from "./AgentCursorLayer.js";
import type { FrameFit, Point, Size } from "./mapping.js";

/** One thing the agent did (or is doing) on its computer. Points are normalized guest coordinates (0..1). */
export type AgentCursorAction =
  | { readonly kind: "move"; readonly at: Point }
  | { readonly kind: "click"; readonly at: Point; readonly count: 1 | 2 }
  | { readonly kind: "press"; readonly at: Point }
  | { readonly kind: "release"; readonly at: Point }
  | { readonly kind: "drag"; readonly from: Point; readonly to: Point; readonly durationMs: number }
  | { readonly kind: "scroll"; readonly at: Point; readonly dx: number; readonly dy: number }
  | { readonly kind: "typing"; readonly active: boolean }
  | { readonly kind: "observe" }
  | { readonly kind: "think" }
  | { readonly kind: "done" }
  /** Stop / reset / cancelled / failed / the person took control / the computer went away. */
  | { readonly kind: "stop" };

export interface AgentCursorSource {
  /** Diagnostic name. */
  readonly id: string;
  subscribe(listener: (action: AgentCursorAction) => void): () => void;
}

/** A source with nothing to relay: the cursor never appears. */
export const silentAgentCursorSource: AgentCursorSource = {
  id: "silent",
  subscribe: () => () => undefined,
};

/** Route one action to the cursor. */
export function applyAgentCursorAction(handle: AgentCursorHandle, action: AgentCursorAction): void {
  switch (action.kind) {
    case "move":
      handle.moveTo(action.at);
      return;
    case "click":
      handle.click(action.at, action.count);
      return;
    case "press":
      handle.press(action.at);
      return;
    case "release":
      handle.release(action.at);
      return;
    case "drag":
      handle.drag(action.from, action.to, action.durationMs);
      return;
    case "scroll":
      handle.scroll(action.at, action.dx, action.dy);
      return;
    case "typing":
      handle.typing(action.active);
      return;
    case "observe":
      handle.observe();
      return;
    case "think":
      handle.think();
      return;
    case "done":
      handle.done();
      return;
    case "stop":
      handle.stop();
      return;
  }
}

export interface AgentCursorOverlayProps {
  readonly source?: AgentCursorSource;
  /** Guest framebuffer size (px) of the picture under the overlay. */
  readonly frameSize?: Size | null;
  /** The preview's CSS `object-fit` for the picture under the overlay. */
  readonly fit?: FrameFit;
  readonly effectsTier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly className?: string;
  readonly style?: CSSProperties;
}

/**
 * The production mount point: an AgentCursor layer over the preview, fed
 * imperatively by `source` (no React render per action). Actions from
 * before it mounted are never replayed: the cursor starts at the next real
 * action. Unmounting (preview gone, computer replaced) disposes everything.
 */
export function AgentCursorOverlay({ source = silentAgentCursorSource, frameSize, fit, effectsTier, reducedMotion, className, style }: AgentCursorOverlayProps) {
  const handle = useRef<AgentCursorHandle | null>(null);
  useEffect(() => {
    const unsubscribe = source.subscribe((action) => {
      if (handle.current) applyAgentCursorAction(handle.current, action);
    });
    return () => {
      unsubscribe();
      handle.current?.stop();
    };
  }, [source]);
  return (
    <AgentCursorLayer
      ref={handle}
      frameSize={frameSize}
      fit={fit}
      effectsTier={effectsTier}
      reducedMotion={reducedMotion}
      className={className}
      style={style}
    />
  );
}
