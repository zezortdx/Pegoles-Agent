import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { DESIGN_LAB_MARKER, DesignLab } from "./DesignLab";

describe("DesignLab", () => {
  afterEach(() => {
    cleanup();
    window.location.hash = "";
  });

  it("renders every section of the lab and carries the dev marker", () => {
    const { container } = render(<DesignLab />);
    expect(container.querySelector(`[data-lab-marker="${DESIGN_LAB_MARKER}"]`)).not.toBeNull();
    for (const name of [
      "Color",
      "Typography",
      "Materials",
      "Buttons",
      "Status",
      "Command → Task",
      "Activity",
      "Pegoles Computer",
      "Motion",
      "Effects tiers",
    ]) {
      expect(screen.getByRole("heading", { level: 2, name })).toBeTruthy();
    }
  });

  it("switching the tier updates the root attribute (no simulated state leaks)", () => {
    render(<DesignLab />);
    const group = screen.getByRole("radiogroup", { name: "Effects tier" });
    fireEvent.click(within(group).getByRole("radio", { name: "Minimal" }));
    expect(document.documentElement.getAttribute("data-effects-tier")).toBe("minimal");
    fireEvent.click(within(group).getByRole("radio", { name: "Full" }));
    expect(document.documentElement.getAttribute("data-effects-tier")).toBe("full");
  });

  it("reads lab parameters from the hash route", () => {
    window.location.hash = "#/dev/design?tier=minimal&motion=reduce&state=agent_active";
    const { container } = render(<DesignLab />);
    expect(document.documentElement.getAttribute("data-effects-tier")).toBe("minimal");
    expect(document.documentElement.getAttribute("data-reduced-motion")).toBe("true");
    const viewport = container.querySelector(".lab-viewport .pg-viewport");
    expect(viewport?.getAttribute("data-state")).toBe("agent_active");
  });
});
