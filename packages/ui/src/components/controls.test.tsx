import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ActivityItem, ActivityList } from "./ActivityItem.js";
import { GlassButton } from "./GlassButton.js";
import { ProgressLine } from "./ProgressLine.js";
import { STATUS_SHAPES, STATUS_TONES, StatusIndicator } from "./StatusIndicator.js";

describe("GlassButton", () => {
  it("is a real, focusable button that defaults to type=button", () => {
    const onClick = vi.fn();
    render(<GlassButton onClick={onClick}>Pause</GlassButton>);
    const button = screen.getByRole("button", { name: "Pause" });
    expect(button.getAttribute("type")).toBe("button");
    button.focus();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("icon-only buttons keep an accessible name", () => {
    render(
      <GlassButton iconOnly icon={<svg />}>
        Stop computer
      </GlassButton>,
    );
    expect(screen.getByRole("button", { name: "Stop computer" }).getAttribute("data-icon-only")).toBe("true");
  });

  it("disabled and loading states block presses and are exposed", () => {
    const onClick = vi.fn();
    const { rerender } = render(
      <GlassButton disabled onClick={onClick}>
        Start
      </GlassButton>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(onClick).not.toHaveBeenCalled();
    rerender(
      <GlassButton isLoading onClick={onClick}>
        Start
      </GlassButton>,
    );
    const button = screen.getByRole("button", { name: "Start" });
    expect(button.getAttribute("aria-busy")).toBe("true");
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("StatusIndicator", () => {
  it("gives every tone a unique shape", () => {
    const shapes = STATUS_TONES.map((t) => STATUS_SHAPES[t]);
    expect(new Set(shapes).size).toBe(STATUS_TONES.length);
  });

  it("always renders the label text alongside the shape (color is never the only cue)", () => {
    for (const tone of STATUS_TONES) {
      const { container, unmount } = render(<StatusIndicator tone={tone} label={`State ${tone}`} />);
      const root = container.firstElementChild;
      expect(root?.getAttribute("data-shape")).toBe(STATUS_SHAPES[tone]);
      expect(root?.textContent).toContain(`State ${tone}`);
      expect(root?.querySelector("svg")?.closest("[aria-hidden='true']")).not.toBeNull();
      unmount();
    }
  });

  it("hidden labels stay in the accessibility tree; live indicators are status regions", () => {
    render(<StatusIndicator tone="success" label="Ready" hideLabel live />);
    const region = screen.getByRole("status");
    expect(region.textContent).toContain("Ready");
    expect(region.getAttribute("aria-live")).toBe("polite");
  });

  it("the presence halo is an ambient (gated) animation", () => {
    const { container } = render(<StatusIndicator tone="active" label="Working" pulse />);
    expect(container.querySelector(".pg-status__halo")?.classList.contains("pg-ambient")).toBe(true);
  });
});

describe("ProgressLine", () => {
  it("is indeterminate (no value, no percentage) without a real measure", () => {
    const { container } = render(<ProgressLine value={null} label="Starting" showValue />);
    const bar = screen.getByRole("progressbar", { name: "Starting" });
    expect(bar.hasAttribute("aria-valuenow")).toBe(false);
    expect(container.textContent).not.toContain("%");
  });

  it("shows the real percentage only when a value exists", () => {
    render(<ProgressLine value={0.426} label="Downloading" showValue />);
    const bar = screen.getByRole("progressbar", { name: "Downloading" });
    expect(bar.getAttribute("aria-valuenow")).toBe("43");
    expect(screen.getByText("43%")).toBeTruthy();
  });
});

describe("ActivityItem", () => {
  it("renders a timeline row with a machine-readable time and kind", () => {
    render(
      <ActivityList label="Computer activity">
        <ActivityItem kind="guest" title="Debian ready" detail="Guest runtime answered in 4.2 s" at="2026-09-21T14:02:00Z" timeLabel="14:02" tone="success" />
        <ActivityItem kind="error" title="Display failed" at="2026-09-21T14:03:00Z" timeLabel="14:03" />
      </ActivityList>,
    );
    const list = screen.getByRole("list", { name: "Computer activity" });
    expect(list.tagName).toBe("OL");
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]?.getAttribute("data-tone")).toBe("success");
    expect(items[1]?.getAttribute("data-tone")).toBe("danger");
    const time = items[0]?.querySelector("time");
    expect(time?.getAttribute("dateTime")).toBe("2026-09-21T14:02:00.000Z");
    expect(time?.textContent).toBe("14:02");
  });

  it("coalesces repeats into one row instead of heartbeat spam", () => {
    render(
      <ActivityList label="Activity">
        <ActivityItem kind="guest" title="Guest runtime reconnected" at="2026-09-21T14:05:00Z" repeatCount={3} />
      </ActivityList>,
    );
    expect(screen.getByRole("listitem").textContent).toContain("×3");
    expect(screen.getByRole("listitem").textContent).toContain("3 times");
  });
});
