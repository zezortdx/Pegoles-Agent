import { act, cleanup, render, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PegolesMark } from "./PegolesMark.js";
import { SUCCESS_HOLD_MS } from "./presenceMachine.js";
import { EMPTY_PRESENCE_SNAPSHOT, type PresenceSnapshot } from "./presenceSource.js";
import { usePresence } from "./usePresence.js";

afterEach(cleanup);

function mark(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>(".pgm");
  if (!el) throw new Error("no mark");
  return el;
}

describe("PegolesMark", () => {
  it("is an accessible image whose name includes the presence state", () => {
    const { getByRole, rerender } = render(<PegolesMark state="thinking" effectsTier="full" reducedMotion={false} />);
    expect(getByRole("img").getAttribute("aria-label")).toBe("Pegoles — Thinking");
    rerender(<PegolesMark state="waitingForUser" effectsTier="full" reducedMotion={false} />);
    expect(getByRole("img").getAttribute("aria-label")).toBe("Pegoles — Waiting for you");
  });

  it("can be decorative (hidden from assistive tech)", () => {
    const { container, queryByRole } = render(<PegolesMark decorative />);
    expect(queryByRole("img")).toBeNull();
    expect(mark(container).getAttribute("aria-hidden")).toBe("true");
  });

  it("renders the traveling light only in Full without reduced motion", () => {
    const full = render(<PegolesMark state="thinking" effectsTier="full" reducedMotion={false} size={96} />);
    expect(full.container.querySelector(".pgm-orbit")).not.toBeNull();
    expect(full.container.querySelector(".pgm-orbit")?.classList.contains("pg-work-anim")).toBe(true);
    full.unmount();
    const reduced = render(<PegolesMark state="thinking" effectsTier="reduced" reducedMotion={false} size={96} />);
    expect(reduced.container.querySelector(".pgm-orbit")).toBeNull();
    expect(reduced.container.querySelector(".pgm-arc")).not.toBeNull();
    reduced.unmount();
    const rm = render(<PegolesMark state="thinking" effectsTier="full" reducedMotion size={96} />);
    expect(rm.container.querySelector(".pgm-orbit")).toBeNull();
    expect(rm.container.querySelector("[style*='animation-name']")).toBeNull();
  });

  it("gates ambient breathing through .pg-ambient and never sets play-state inline", () => {
    const { container } = render(<PegolesMark state="idle" effectsTier="full" reducedMotion={false} />);
    const breathing = container.querySelector<HTMLElement>(".pg-ambient");
    expect(breathing?.style.animationName).toBe("pgm-breathe");
    for (const el of container.querySelectorAll<HTMLElement>("[style]")) {
      expect(el.style.animationPlayState).toBe("");
    }
  });

  it("shows a semantic badge on error instead of recoloring the mark", () => {
    const { container } = render(<PegolesMark state="error" effectsTier="full" reducedMotion={false} />);
    expect(container.querySelector(".pgm-badge")).not.toBeNull();
    expect(container.innerHTML).not.toMatch(/#ff0000|red/i);
  });

  it("shifts the eyes at most 3 px and never under reduced motion", () => {
    const { container, rerender } = render(
      <PegolesMark size={160} lookAt={{ x: 100, y: 0 }} effectsTier="full" reducedMotion={false} />,
    );
    const eyes = () => container.querySelector<SVGElement>(".pgm-eyes");
    expect(eyes()?.style.transform).toBe("translate3d(3px, 0px, 0)");
    rerender(<PegolesMark size={160} lookAt={{ x: 100, y: 0 }} effectsTier="full" reducedMotion />);
    expect(eyes()?.style.transform).toBe("");
  });

  it("uses unique SVG ids per instance", () => {
    const { container } = render(
      <>
        <PegolesMark />
        <PegolesMark />
      </>,
    );
    const ids = [...container.querySelectorAll("[id]")].map((n) => n.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("usePresence (production wiring)", () => {
  it("follows real snapshots and returns to idle after success", () => {
    vi.useFakeTimers();
    try {
      let snapshot: PresenceSnapshot = { ...EMPTY_PRESENCE_SNAPSHOT, task: "running" };
      const { result, rerender } = renderHook(() => usePresence(snapshot));
      expect(result.current).toBe("thinking");
      snapshot = { ...snapshot, viewport: "agent_active" };
      rerender();
      expect(result.current).toBe("acting");
      snapshot = { ...snapshot, task: "completed" };
      rerender();
      expect(result.current).toBe("success");
      act(() => {
        vi.advanceTimersByTime(SUCCESS_HOLD_MS);
      });
      expect(result.current).toBe("idle");
    } finally {
      vi.useRealTimers();
    }
  });
});
