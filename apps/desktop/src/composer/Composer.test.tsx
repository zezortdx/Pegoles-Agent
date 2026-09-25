import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Composer } from "./Composer";
afterEach(cleanup);
describe("task title contract", () => {
  it("normalizes pasted line breaks before submitting to Core", () => {
    const submit = vi.fn();
    render(<Composer value={"Research\ncompetitors"} onChange={() => undefined} onSubmit={submit} placeholder="Task" />);
    fireEvent.submit(screen.getByRole("form"));
    expect(submit).toHaveBeenCalledWith("Research competitors");
  });
  it("blocks overlong titles and counts Unicode scalars like Core", () => {
    const submit = vi.fn();
    const view = render(<Composer value={"x".repeat(501)} onChange={() => undefined} onSubmit={submit} placeholder="Task" />);
    fireEvent.submit(screen.getByRole("form"));
    expect(submit).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBe("true");
    view.rerender(<Composer value={"💡".repeat(500)} onChange={() => undefined} onSubmit={submit} placeholder="Task" />);
    fireEvent.submit(screen.getByRole("form"));
    expect(submit).toHaveBeenCalledOnce();
  });
  it("does not send Enter while an IME composition is active", () => {
    const submit = vi.fn();
    render(<Composer value="Research" onChange={() => undefined} onSubmit={submit} placeholder="Task" />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", isComposing: true });
    expect(submit).not.toHaveBeenCalled();
  });
});
