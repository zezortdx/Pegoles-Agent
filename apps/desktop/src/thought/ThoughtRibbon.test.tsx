import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { RibbonModel } from "../state/ribbon";
import { PresenceMark } from "../presence";
import { ThoughtRibbon } from "./ThoughtRibbon";

afterEach(cleanup);

const model: RibbonModel = {
  trunk: "working",
  activeId: "a3",
  strands: [
    { id: "a1", capability: "Files", label: "Read a file", phase: "done", at: "t" },
    { id: "a2", capability: "Web", label: "Opened a website", phase: "failed", at: "t" },
    { id: "a3", capability: "Computer", label: "Looking at the screen", phase: "running", at: "t" },
  ],
};

describe("ThoughtRibbon", () => {
  it("draws one strand per action with DOM labels and a status for assistive tech", () => {
    const { container, getByRole } = render(<ThoughtRibbon model={model} />);
    expect(container.querySelectorAll(".thought-ribbon__strand")).toHaveLength(3);
    expect(getByRole("list").textContent).toContain("Looking at the screen, in progress");
    expect(container.querySelector(".thought-ribbon")?.getAttribute("data-trunk")).toBe("working");
    expect(container.querySelector(".presence")?.getAttribute("data-mode")).toBe("using-computer");
  });

  it("makes only the active strand luminous and loops only while it runs", () => {
    const { container } = render(<ThoughtRibbon model={model} />);
    expect(container.querySelectorAll(".thought-ribbon__lit")).toHaveLength(1);
    expect(container.querySelector('[data-strand="a3"] .thought-ribbon__signal')?.classList.contains("pg-work-anim")).toBe(true);
    expect(container.querySelector('[data-strand="a2"] .thought-ribbon__cap')).toBeTruthy();
    const settled = render(<ThoughtRibbon model={{ ...model, trunk: "done", activeId: undefined, strands: model.strands.map((s) => ({ ...s, phase: "done" as const })) }} />);
    expect(settled.container.querySelector(".thought-ribbon__signal")).toBeNull();
    expect(settled.container.querySelector(".thought-ribbon__lit")).toBeNull();
  });

  it("has no traveling signal under reduced motion and no strands for a bare trunk", () => {
    expect(render(<ThoughtRibbon model={model} reducedMotion />).container.querySelector(".thought-ribbon__signal")).toBeNull();
    expect(render(<ThoughtRibbon model={{ trunk: "thinking", strands: [] }} />).container.querySelector(".thought-ribbon__strand")).toBeNull();
  });
});

describe("PresenceMark", () => {
  it("is an SVG-only, decorative mark with the mode's pose", () => {
    const { container } = render(<PresenceMark mode="blocked" size={18} />);
    const mark = container.querySelector<HTMLElement>(".presence");
    expect(mark?.getAttribute("aria-hidden")).toBe("true");
    expect(mark?.hasAttribute("data-small")).toBe(true);
    expect(mark?.style.getPropertyValue("--p-lid")).toBe("0.46");
    expect(container.querySelector(".presence__gl")).toBeNull();
  });
});
