import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { activateLocale } from "../i18n";
import type { AgentEvent, TeamStatus, TeamStepId } from "../lib/tauri";
import { teamRunOf } from "../state/teamRun";
import { TeamPanel } from "./TeamPanel";

afterEach(() => { cleanup(); activateLocale("en"); });

function must<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error("expected a value");
  return value;
}
const at = "2026-10-25T10:00:00Z";
const ev = (step: TeamStepId, status: TeamStatus, summary = ""): AgentEvent => ({ type: "team_progress", task_id: "t", step, status, summary, at });
const finished = [
  ev("captain_plan", "done", "1. Open the terminal"), ev("research", "done", "Terminal is idle"), ev("execute", "done", "Typed it"),
  ev("critique", "done", "PASS: the prompt shows it"), ev("synthesis", "done", "All done."),
];

describe("TeamPanel", () => {
  it("shows four roles, each with a distinct silhouette, a written name and a written state", () => {
    const { container } = render(<TeamPanel run={must(teamRunOf([ev("captain_plan", "done"), ev("research", "working")]))} />);
    const strip = screen.getByRole("list", { name: "Team" });
    const roles = within(strip).getAllByRole("listitem");
    expect(roles.map((role) => role.getAttribute("data-role"))).toEqual(["captain", "researcher", "executor", "critic"]);
    expect(roles.map((role) => role.textContent)).toEqual(["CaptainDone", "ResearcherWorking", "ExecutorWaiting", "CriticWaiting"]);
    const shapes = [...container.querySelectorAll("svg.presence-avatar")].map((svg) => svg.getAttribute("data-shape"));
    expect(new Set(shapes).size).toBe(4);
    expect(new Set([...container.querySelectorAll("svg.presence-avatar")].map((svg) => svg.getAttribute("data-color")))).toEqual(new Set(["pearl", "silver", "graphite"]));
    // The avatars are decorative: the text above is the accessible name.
    expect(container.querySelectorAll("svg.presence-avatar[aria-hidden=true]").length).toBe(4);
    expect(screen.getByRole("status").textContent).toBe("Researcher: look at the screen: Working");
  });

  it("makes the final result prominent and keeps the panel collapsed until asked", () => {
    render(<TeamPanel run={must(teamRunOf(finished))} result="All done." />);
    const result = screen.getByRole("region", { name: "Team result" });
    expect(result.textContent).toContain("All done.");
    expect(screen.getByRole("status").textContent).toBe("The team finished.");
    const toggle = screen.getByRole("button", { name: "How the team thought" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    const region = must(document.getElementById(must(toggle.getAttribute("aria-controls"))));
    expect(region.hidden).toBe(true);

    toggle.focus();
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(region.hidden).toBe(false);
    expect(within(region).getByText("Short status notes from each role. Hidden reasoning is never shown.")).toBeTruthy();
    const steps = within(region).getAllByRole("listitem");
    expect(steps.map((step) => step.querySelector(".team-step__title")?.textContent)).toEqual([
      "Captain: plan", "Researcher: look at the screen", "Executor: do the work", "Critic: check the result", "Captain: final answer",
    ]);
    expect(steps[3].textContent).toContain("PASS: the prompt shows it");
    expect(document.activeElement).toBe(toggle);
    fireEvent.click(toggle);
    expect(region.hidden).toBe(true);
  });

  it("says plainly when the team stopped, with the steps that never ran", () => {
    render(<TeamPanel run={must(teamRunOf([ev("captain_plan", "done"), ev("research", "failed"), ev("execute", "skipped"), ev("critique", "skipped"), ev("synthesis", "skipped")]))} />);
    expect(screen.getByRole("status").textContent).toBe("The team stopped before finishing.");
    fireEvent.click(screen.getByRole("button", { name: "How the team thought" }));
    expect(screen.getAllByText("Skipped").length).toBeGreaterThanOrEqual(3);
    expect(screen.getAllByText("Failed").length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByRole("region", { name: "Team result" })).toBeNull();
  });

  it("renders role and result text as plain text, never as markup", () => {
    const hostile = "<img src=x onerror=alert(1)><b>bold</b>";
    const { container } = render(<TeamPanel run={must(teamRunOf([ev("captain_plan", "done", hostile)]))} result={hostile} />);
    fireEvent.click(screen.getByRole("button", { name: "How the team thought" }));
    expect(container.querySelector("img, b, script")).toBeNull();
    expect(screen.getAllByText(hostile).length).toBe(2);
  });

  it("follows the saved language for product copy but leaves role text as written", () => {
    activateLocale("pt-BR");
    render(<TeamPanel run={must(teamRunOf([ev("captain_plan", "done", "Open the terminal")]))} result="Texto do modelo" />);
    expect(screen.getByRole("list", { name: "Equipe" })).toBeTruthy();
    expect(screen.getByText("Capitão")).toBeTruthy();
    expect(screen.getByText("Pesquisador")).toBeTruthy();
    expect(screen.getByRole("region", { name: "Resultado da equipe" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Como a equipe pensou" }));
    expect(screen.getByText("Open the terminal")).toBeTruthy();
  });
});
