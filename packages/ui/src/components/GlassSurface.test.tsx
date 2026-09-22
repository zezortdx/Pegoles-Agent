import { render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { getBlurAudit, resetBlurAuditForTests } from "../runtime/blurAudit.js";
import { EffectsScope, FluxGlassRoot } from "../runtime/FluxGlassRoot.js";
import { GlassSurface } from "./GlassSurface.js";

describe("GlassSurface", () => {
  afterEach(() => {
    resetBlurAuditForTests();
  });

  it("renders the requested element with material/elevation attributes", () => {
    const { getByTestId } = render(
      <FluxGlassRoot tier="full">
        <GlassSurface as="section" material="electric" elevation={3} data-testid="glass" aria-label="Task">
          content
        </GlassSurface>
      </FluxGlassRoot>,
    );
    const el = getByTestId("glass");
    expect(el.tagName).toBe("SECTION");
    expect(el.getAttribute("data-material")).toBe("electric");
    expect(el.getAttribute("data-elevation")).toBe("3");
    expect(el.hasAttribute("data-nested")).toBe(false);
    expect(el.className).toContain("pg-glass");
  });

  it("suppresses backdrop blur for nested glass (no blur-on-blur)", () => {
    const { getByTestId } = render(
      <FluxGlassRoot tier="full">
        <GlassSurface data-testid="outer" auditLabel="outer">
          <GlassSurface data-testid="inner" auditLabel="inner">
            <GlassSurface data-testid="innermost" material="clear" auditLabel="innermost" />
          </GlassSurface>
        </GlassSurface>
      </FluxGlassRoot>,
    );
    expect(getByTestId("outer").hasAttribute("data-nested")).toBe(false);
    expect(getByTestId("inner").getAttribute("data-nested")).toBe("true");
    expect(getByTestId("innermost").getAttribute("data-nested")).toBe("true");
    // Only the outer surface applies backdrop-filter and is counted.
    expect(getBlurAudit().mounted).toBe(1);
    expect(getBlurAudit().surfaces.map((s) => s.label)).toEqual(["outer"]);
  });

  it("counts backdrop surfaces per tier and none on Minimal", () => {
    const { rerender } = render(
      <FluxGlassRoot tier="full">
        <GlassSurface auditLabel="a" />
        <GlassSurface auditLabel="b" material="clear" />
      </FluxGlassRoot>,
    );
    expect(getBlurAudit().mounted).toBe(2);
    expect(getBlurAudit().surfaces.find((s) => s.label === "b")?.blurPx).toBe(18);

    rerender(
      <FluxGlassRoot tier="minimal">
        <GlassSurface auditLabel="a" />
        <GlassSurface auditLabel="b" material="clear" />
      </FluxGlassRoot>,
    );
    expect(getBlurAudit().mounted).toBe(0);
  });

  it("follows a scoped tier (EffectsScope) for audit purposes", () => {
    render(
      <FluxGlassRoot tier="full">
        <EffectsScope tier="minimal" data-testid="scope">
          <GlassSurface auditLabel="scoped" />
        </EffectsScope>
        <GlassSurface auditLabel="root" />
      </FluxGlassRoot>,
    );
    expect(getBlurAudit().surfaces.map((s) => s.label)).toEqual(["root"]);
  });

  it("unregisters on unmount", () => {
    const { unmount } = render(
      <FluxGlassRoot tier="reduced">
        <GlassSurface auditLabel="gone" />
      </FluxGlassRoot>,
    );
    expect(getBlurAudit().mounted).toBe(1);
    unmount();
    expect(getBlurAudit().mounted).toBe(0);
  });
});
