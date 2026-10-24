import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ApprovalSetting } from "./ApprovalSetting";
import { api } from "../lib/tauri";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
it("labels restart behavior and keeps Core's value after native refusal", async () => {
  vi.spyOn(api, "setInputApproval").mockRejectedValue(new Error("native refusal"));
  const refresh = vi.fn(async () => undefined);
  render(<ApprovalSetting enabled={true} refresh={refresh} />);
  expect(screen.getByText(/resets to off when Pegoles restarts/)).not.toBeNull();
  fireEvent.click(screen.getByRole("checkbox"));
  await waitFor(() => expect(api.setInputApproval).toHaveBeenCalledWith(false));
  await screen.findByRole("alert");
  expect((screen.getByRole("checkbox") as HTMLInputElement).checked).toBe(true);
  expect(refresh).not.toHaveBeenCalled();
});
