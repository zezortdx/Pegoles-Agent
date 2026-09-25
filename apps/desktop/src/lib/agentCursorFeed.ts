/**
 * Real action-event cursor feed (Phase 5).
 *
 * Bridges Tauri `pegoles://event` action lifecycle events to the
 * production `AgentCursorSource` consumed by `AgentCursorOverlay`:
 * - `action_started` with a pointer action → show + move/click/drag,
 * - typing actions → typing indicator on, cleared on terminal events,
 * - user takeover → hide immediately.
 *
 * Coordinates are REAL: normalized agent positions from the action,
 * mapped through the live framebuffer slot rect into overlay pixels.
 * Nothing is invented: without action events the overlay stays null.
 */
import { listen } from "@tauri-apps/api/event";
import type { AgentCursorAction, AgentCursorSource } from "@pegoles/ui";
import type { ActionRequestWire, AgentEvent } from "./tauri";

type Listener = (action: AgentCursorAction) => void;

interface GuestSize {
  w: number;
  h: number;
}

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** Overlay-px point for a normalized agent position, via the live slot. */
export function overlayPoint(nx: number, ny: number): { x: number; y: number } | null {
  if (typeof document === "undefined") return null;
  const wrap = document.querySelector(".computer__screen");
  const slot = document.querySelector("[data-framebuffer-slot]");
  if (!(wrap instanceof HTMLElement) || !(slot instanceof HTMLElement)) return null;
  const wrapRect = wrap.getBoundingClientRect();
  const slotRect = slot.getBoundingClientRect();
  if (slotRect.width < 2 || slotRect.height < 2) return null;
  const cx = Math.min(1, Math.max(0, nx));
  const cy = Math.min(1, Math.max(0, ny));
  return {
    x: Math.round(slotRect.left - wrapRect.left + cx * slotRect.width),
    y: Math.round(slotRect.top - wrapRect.top + cy * slotRect.height),
  };
}

class ActionCursorFeed implements AgentCursorSource {
  readonly id = "tauri-action-events";
  private listeners = new Set<Listener>();
  private typing = new Set<string>();
  private display: GuestSize | null = null;
  private lastFrame: GuestSize | null = null;
  private live = false;
  private disposed = false;

  setDisplaySize(size: GuestSize | null): void {
    this.display = size;
  }

  private guestSize(): GuestSize | null {
    return this.lastFrame ?? this.display;
  }

  private ensureLive(): void {
    if (this.live) return;
    this.live = true;
    void listen<AgentEvent>("pegoles://event", (e) => {
      if (!this.disposed) this.onEvent(e.payload);
    }).catch(() => {
      this.live = false;
    });
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    this.ensureLive();
    return () => {
      this.listeners.delete(listener);
    };
  }

  dispose(): void {
    this.disposed = true;
    this.listeners.clear();
  }

  private emit(action: AgentCursorAction): void {
    for (const l of this.listeners) {
      try {
        l(action);
      } catch {
        // A throwing listener must not break the feed.
      }
    }
  }

  private at(action: ActionRequestWire["action"]): { x: number; y: number } | null {
    if (!this.guestSize()) return null;
    const x = num(action.x) ?? num(action.from_x);
    const y = num(action.y) ?? num(action.from_y);
    if (x === null || y === null) return null;
    return overlayPoint(x, y);
  }

  private onEvent(e: AgentEvent): void {
    if (e.type === "frame_observed") {
      const f = (e as { frame: { width_px: number; height_px: number } }).frame;
      if (f.width_px > 0 && f.height_px > 0) {
        this.lastFrame = { w: f.width_px, h: f.height_px };
      }
      return;
    }
    if (e.type === "control_ownership_changed") {
      const to = (e as { to: string }).to;
      if (to === "user") {
        this.typing.clear();
        this.emit({ kind: "hide" });
      }
      return;
    }
    if (e.type === "action_started") {
      const s = e as { action_id: string; request: ActionRequestWire };
      const action = s.request.action;
      this.emit({ kind: "show" });
      const verb = action.type;
      if (
        verb === "move_pointer" ||
        verb === "click" ||
        verb === "double_click" ||
        verb === "mouse_down" ||
        verb === "scroll"
      ) {
        const p = this.at(action);
        if (p) this.emit({ kind: "move", x: p.x, y: p.y });
      }
      if (verb === "click" || verb === "mouse_down") {
        this.emit({ kind: "click" });
      }
      if (verb === "double_click") {
        this.emit({ kind: "click" });
        window.setTimeout(() => {
          if (!this.disposed) this.emit({ kind: "click" });
        }, 150);
      }
      if (verb === "drag") {
        const fx = num(action.from_x);
        const fy = num(action.from_y);
        const tx = num(action.to_x);
        const ty = num(action.to_y);
        if (this.guestSize() && fx !== null && fy !== null && tx !== null && ty !== null) {
          const from = overlayPoint(fx, fy);
          const to = overlayPoint(tx, ty);
          if (from && to) {
            this.emit({ kind: "move", x: from.x, y: from.y });
            this.emit({ kind: "drag", x: to.x, y: to.y });
          }
        }
      }
      if (verb === "type_text" || verb === "type") {
        this.typing.add(s.action_id);
        this.emit({ kind: "typing", active: true });
      }
      if (verb === "wait") {
        this.emit({ kind: "wait" });
      }
      return;
    }
    if (e.type === "action_completed" || e.type === "action_failed" || e.type === "action_denied") {
      const id =
        (e as { action_id?: string }).action_id ??
        (e as { request?: ActionRequestWire }).request?.action_id;
      if (id && this.typing.delete(id)) {
        this.emit({ kind: "typing", active: false });
      }
    }
  }
}

/** App-wide singleton (single computer in Phase 1). */
export const actionCursorSource = new ActionCursorFeed();
