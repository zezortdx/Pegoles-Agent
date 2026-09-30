import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useFluxGlass } from "./context.js";
import { FluxGlassRoot } from "./FluxGlassRoot.js";
import { TOKEN_STYLE_ID } from "./styleSheet.js";

function Probe() {
  const { tier, reducedMotion, transitionStyle, params } = useFluxGlass();
  return (
    <output data-testid="probe">
      {tier}|{String(reducedMotion)}|{transitionStyle}|{params.maxBackdropSurfaces}
    </output>
  );
}

describe("FluxGlassRoot", () => {
  afterEach(() => {
    document.getElementById(TOKEN_STYLE_ID)?.remove();
  });

  it("injects the token sheet once and writes the gate attributes on the target", () => {
    const target = document.createElement("div");
    document.body.append(target);
    const { rerender } = render(
      <FluxGlassRoot tier="full" reducedMotion={false} target={target}>
        <Probe />
      </FluxGlassRoot>,
    );
    expect(document.querySelectorAll(`#${TOKEN_STYLE_ID}`)).toHaveLength(1);
    expect(document.getElementById(TOKEN_STYLE_ID)?.textContent).toContain("--pg-bg-primary: #02040A;");
    expect(target.getAttribute("data-effects-tier")).toBe("full");
    expect(target.getAttribute("data-reduced-motion")).toBe("false");
    expect(target.getAttribute("data-ambient")).toMatch(/running|paused/);
    expect(screen.getByTestId("probe").textContent).toBe("full|false|morph|6");

    rerender(
      <FluxGlassRoot tier="minimal" reducedMotion target={target}>
        <Probe />
      </FluxGlassRoot>,
    );
    expect(document.querySelectorAll(`#${TOKEN_STYLE_ID}`)).toHaveLength(1);
    expect(target.getAttribute("data-effects-tier")).toBe("minimal");
    expect(target.getAttribute("data-reduced-motion")).toBe("true");
    expect(target.getAttribute("data-ambient")).toBe("paused");
    expect(screen.getByTestId("probe").textContent).toBe("minimal|true|crossfade|0");
    target.remove();
  });

  it("reduced motion keeps the tier (orthogonal axes)", () => {
    const target = document.createElement("div");
    render(
      <FluxGlassRoot tier="full" reducedMotion target={target}>
        <Probe />
      </FluxGlassRoot>,
    );
    expect(target.getAttribute("data-effects-tier")).toBe("full");
    expect(screen.getByTestId("probe").textContent).toBe("full|true|crossfade|6");
  });

  it("stops the gate on unmount (ambient paused)", () => {
    const target = document.createElement("div");
    const { unmount } = render(<FluxGlassRoot tier="full" target={target} />);
    act(() => {
      unmount();
    });
    expect(target.getAttribute("data-ambient")).toBe("paused");
    expect(target.getAttribute("data-ambient-reason")).toBe("stopped");
  });
});
