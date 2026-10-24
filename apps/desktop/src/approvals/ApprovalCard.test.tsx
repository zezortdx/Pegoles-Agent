import { cleanup, fireEvent, render, screen, act } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { activateLocale } from "../i18n";
import { ApprovalCard } from "./ApprovalCard";
const approval = { id: "opaque-id", task_id: "task", computer_id: "computer", summary: '<img src=x onerror="alert(1)">', expires_at: new Date(Date.now() + 120000).toISOString() };
afterEach(() => { cleanup(); vi.useRealTimers(); activateLocale("en"); });
it("renders hostile summaries inert and waits for native confirmation before showing a result", async () => {
  let finish!: () => void;
  const resolve = vi.fn(() => new Promise<void>((r) => { finish = r; }));
  const { container } = render(<ApprovalCard approval={approval} resolve={resolve} />);
  expect(screen.getByText(approval.summary)).not.toBeNull();
  expect(container.querySelector("img")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Approve once" }));
  expect(resolve).toHaveBeenCalledWith("opaque-id", "approve_once");
  expect(screen.getByText("Waiting for native confirmation…")).not.toBeNull();
  expect((screen.getByRole("button", { name: "Approve once" }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { finish(); });
  expect(screen.getByText("Decision confirmed by Core.")).not.toBeNull();
});
it("expires locally, disables approval and never invents native success on refusal", async () => {
  const resolve = vi.fn().mockRejectedValue(new Error("refused"));
  render(<ApprovalCard approval={approval} resolve={resolve} />);
  fireEvent.click(screen.getByRole("button", { name: "Approve once" }));
  expect((await screen.findByRole("alert")).textContent).toContain("The decision could not be confirmed.");
  expect(screen.queryByText("Decision confirmed by Core.")).toBeNull();
  cleanup();
  render(<ApprovalCard approval={{ ...approval, expires_at: new Date(0).toISOString() }} resolve={resolve} />);
  expect(screen.getByText("Expired · action denied")).not.toBeNull();
  expect((screen.getByRole("button", { name: "Approve once" }) as HTMLButtonElement).disabled).toBe(true);
});

it("localizes safe Core summaries and controls without translating hostile text", () => {
  activateLocale("pt-BR");
  render(<ApprovalCard approval={{ ...approval, summary: "VM input: type" }} resolve={async () => undefined} />);
  expect(screen.getByText("Digitar texto na VM")).not.toBeNull();
  expect(screen.getByRole("button", { name: "Aprovar uma vez" })).not.toBeNull();
  expect(screen.getByRole("button", { name: "Negar" })).not.toBeNull();
});
