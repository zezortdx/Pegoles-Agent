/**
 * Where AgentCursor actions come from.
 *
 * NO FAKE AUTONOMOUS CURSOR IN PRODUCTION. The production source relays
 * REAL agent-action events only. Phase 4 has no agent loop and therefore
 * no such events, so `productionAgentCursorSource` never emits, the hook
 * never activates, and `AgentCursorOverlay` renders nothing. Simulated
 * fixtures exist only in the design lab (apps/desktop/src/dev).
 */
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import type { EffectsTier } from "../tokens/effects.js";
import { AgentCursorLayer, type AgentCursorHandle } from "./AgentCursorLayer.js";

/** One action the agent performed inside its computer (overlay px). */
export type AgentCursorAction =
  | { readonly kind: "move"; readonly x: number; readonly y: number }
  | { readonly kind: "click" }
  | { readonly kind: "drag"; readonly x: number; readonly y: number }
  | { readonly kind: "typing"; readonly active: boolean }
  | { readonly kind: "wait" }
  | { readonly kind: "show" }
  | { readonly kind: "hide" };

export interface AgentCursorSource {
  /** Diagnostic name ("agent-actions" in production). */
  readonly id: string;
  subscribe(listener: (action: AgentCursorAction) => void): () => void;
}

/**
 * The production source. Phase 5 wires real agent-action events here
 * (from Pegoles Core, after they pass its policy); until then there is
 * nothing to relay, by design.
 */
export const productionAgentCursorSource: AgentCursorSource = {
  id: "agent-actions",
  subscribe: () => () => undefined,
};

/** Route one action to the cursor handle. */
export function applyAgentCursorAction(handle: AgentCursorHandle, action: AgentCursorAction): void {
  switch (action.kind) {
    case "move":
      void handle.moveTo(action.x, action.y);
      return;
    case "click":
      handle.click();
      return;
    case "drag":
      void handle.dragTo(action.x, action.y);
      return;
    case "typing":
      handle.typing(action.active);
      return;
    case "wait":
      handle.wait();
      return;
    case "show":
      handle.show();
      return;
    case "hide":
      handle.hide();
      return;
  }
}

export interface AgentCursorSourceBinding {
  /** Becomes true on the first real action (React state changes once, not per action). */
  readonly active: boolean;
  /** Ref callback for the AgentCursorLayer; flushes actions that arrived before mount. */
  readonly attach: (handle: AgentCursorHandle | null) => void;
}

/**
 * Subscribe to an action source. Actions are forwarded imperatively to the
 * attached cursor (no React render per action); the only React update is
 * the one-time `active` flip that mounts the layer.
 */
export function useAgentCursorSource(source: AgentCursorSource = productionAgentCursorSource): AgentCursorSourceBinding {
  const [active, setActive] = useState(false);
  const handleRef = useRef<AgentCursorHandle | null>(null);
  const queue = useRef<AgentCursorAction[]>([]);

  useEffect(() => {
    const unsubscribe = source.subscribe((action) => {
      const handle = handleRef.current;
      if (handle) {
        applyAgentCursorAction(handle, action);
      } else {
        queue.current = [...queue.current, action].slice(-32);
        setActive(true);
      }
    });
    return unsubscribe;
  }, [source]);

  const attach = useCallback((handle: AgentCursorHandle | null) => {
    handleRef.current = handle;
    if (!handle) return;
    const pending = queue.current;
    queue.current = [];
    for (const action of pending) applyAgentCursorAction(handle, action);
  }, []);

  return { active, attach };
}

export interface AgentCursorOverlayProps {
  readonly source?: AgentCursorSource;
  readonly effectsTier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly className?: string;
  readonly style?: CSSProperties;
}

/**
 * The production mount point: renders the AgentCursor layer only once the
 * source has delivered a real agent action. With the Phase 4 production
 * source this is always `null`.
 */
export function AgentCursorOverlay({ source, effectsTier, reducedMotion, className, style }: AgentCursorOverlayProps) {
  const { active, attach } = useAgentCursorSource(source);
  if (!active) return null;
  return (
    <AgentCursorLayer
      ref={attach}
      effectsTier={effectsTier}
      reducedMotion={reducedMotion}
      className={className}
      style={style}
    />
  );
}
