import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApprovalCard } from "./ApprovalCard.js";
import { CurrentAction } from "./CurrentAction.js";

describe("ApprovalCard", () => {
  it("emerges as glass with honest copy and no invented diff", () => {
    render(<ApprovalCard title="Apply changes" />);
    expect(screen.getByLabelText("Pegoles needs permission")).toBeTruthy();
    expect(screen.getByText("Apply changes")).toBeTruthy();
    expect(screen.getByText(/approval payload/)).toBeTruthy();
  });
});

describe("CurrentAction", () => {
  it("announces the live label politely without a spinner", () => {
    render(<CurrentAction label="Pegoles is typing" working />);
    expect(screen.getByRole("status").textContent).toContain("Pegoles is typing");
  });
});
