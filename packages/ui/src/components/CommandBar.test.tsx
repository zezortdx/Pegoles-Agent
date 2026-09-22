import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { FluxGlassRoot } from "../runtime/FluxGlassRoot.js";
import { CommandBar } from "./CommandBar.js";
import { CommandMorph, TaskController } from "./TaskController.js";

describe("CommandBar", () => {
  it("is a labelled form with a labelled field and the Pegoles placeholder", () => {
    render(<CommandBar onSubmit={() => undefined} />);
    expect(screen.getByRole("form", { name: "Ask Pegoles" })).toBeTruthy();
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" });
    expect(input.getAttribute("placeholder")).toBe("Ask Pegoles…");
  });

  it("submits trimmed text on Enter and clears when uncontrolled", () => {
    const onSubmit = vi.fn();
    render(<CommandBar onSubmit={onSubmit} />);
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  Open a terminal and check disk space  " } });
    fireEvent.submit(input.form as HTMLFormElement);
    expect(onSubmit).toHaveBeenCalledWith("Open a terminal and check disk space");
    expect(input.value).toBe("");
  });

  it("cannot submit empty or whitespace-only text", () => {
    const onSubmit = vi.fn();
    render(<CommandBar onSubmit={onSubmit} />);
    const send = screen.getByRole("button", { name: "Send to Pegoles" }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.submit(input.form as HTMLFormElement);
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "Summarize the boot log" } });
    expect(send.disabled).toBe(false);
  });

  it("Escape clears first, then leaves the field", () => {
    render(<CommandBar onSubmit={() => undefined} />);
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" }) as HTMLInputElement;
    input.focus();
    fireEvent.change(input, { target: { value: "draft" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(input.value).toBe("");
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape" });
    expect(document.activeElement).not.toBe(input);
  });

  it("supports controlled usage", () => {
    function Controlled({ onSubmit }: { onSubmit: (t: string) => void }) {
      const [value, setValue] = useState("Check memory");
      return <CommandBar value={value} onValueChange={setValue} onSubmit={onSubmit} />;
    }
    const onSubmit = vi.fn();
    render(<Controlled onSubmit={onSubmit} />);
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" }) as HTMLInputElement;
    expect(input.value).toBe("Check memory");
    fireEvent.submit(input.form as HTMLFormElement);
    expect(onSubmit).toHaveBeenCalledWith("Check memory");
    expect(input.value).toBe("Check memory");
  });

  it("busy state blocks re-submit and is announced", () => {
    const onSubmit = vi.fn();
    render(<CommandBar onSubmit={onSubmit} defaultValue="Run the backup" isBusy />);
    const form = screen.getByRole("form", { name: "Ask Pegoles" });
    expect(form.getAttribute("aria-busy")).toBe("true");
    fireEvent.submit(form);
    expect(onSubmit).not.toHaveBeenCalled();
  });
});

describe("TaskController", () => {
  it("renders the task, a live status and real detail", () => {
    render(
      <TaskController
        title="Open a terminal and check disk space"
        status={{ tone: "active", label: "Working" }}
        detail="Opening a terminal"
        elapsed="0:42"
        working
      />,
    );
    const region = screen.getByRole("region", { name: "Current task" });
    expect(region.textContent).toContain("Open a terminal and check disk space");
    expect(screen.getByRole("status").textContent).toContain("Working");
    const bar = screen.getByRole("progressbar");
    expect(bar.hasAttribute("aria-valuenow")).toBe(false);
    expect(region.textContent).not.toContain("%");
  });
});

describe("CommandMorph", () => {
  function Harness({ reducedMotion, tier }: { reducedMotion: boolean; tier: "full" | "minimal" }) {
    const [task, setTask] = useState<string | null>(null);
    return (
      <FluxGlassRoot tier={tier} reducedMotion={reducedMotion}>
        <CommandMorph
          mode={task ? "task" : "command"}
          command={<CommandBar onSubmit={setTask} />}
          task={<TaskController title={task ?? ""} status={{ tone: "active", label: "Working" }} />}
        />
      </FluxGlassRoot>
    );
  }

  it.each([
    ["full", false, "morph"],
    ["full", true, "crossfade"],
    ["minimal", false, "crossfade"],
  ] as const)("tier %s, reduced motion %s → %s, and the task replaces the bar", async (tier, rm, style) => {
    const { container } = render(<Harness tier={tier} reducedMotion={rm} />);
    expect(container.querySelector(".pg-command-morph")?.getAttribute("data-transition")).toBe(style);
    const input = screen.getByRole("textbox", { name: "Ask Pegoles" }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Check disk space" } });
    await act(async () => {
      fireEvent.submit(input.form as HTMLFormElement);
    });
    expect(screen.getByRole("region", { name: "Current task" }).textContent).toContain("Check disk space");
  });
});
