import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DiagnosticReportButton } from "./DiagnosticReport";

afterEach(cleanup);

const report = { file_name: "Pegoles-report-20260928-130405.json", location: "~/Downloads", text: "{\"report\":{}}" };

describe("DiagnosticReportButton", () => {
  it("says what the report holds before anyone presses it", () => {
    render(<DiagnosticReportButton save={vi.fn()} />);
    expect(screen.getByRole("status").textContent).toMatch(/no screenshots, tasks, keys or personal files/);
  });

  it("saves the report and says where it went, then copies it", async () => {
    const save = vi.fn().mockResolvedValue(report);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<DiagnosticReportButton save={save} />);
    fireEvent.click(screen.getByRole("button", { name: "Save a report for help" }));
    expect(await screen.findByText(report.file_name)).toBeTruthy();
    expect(screen.getByRole("status").textContent).toMatch(/~\/Downloads.*Send that file/);
    fireEvent.click(screen.getByRole("button", { name: "Copy report" }));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeTruthy();
    expect(writeText).toHaveBeenCalledWith(report.text);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("explains a failure in plain words", async () => {
    const save = vi.fn().mockRejectedValue("could not save the report in ~/Downloads: permission denied");
    render(<DiagnosticReportButton save={save} />);
    fireEvent.click(screen.getByRole("button", { name: "Save a report for help" }));
    expect((await screen.findByRole("status")).textContent).toMatch(/couldn’t save the report: could not save/);
  });
});
