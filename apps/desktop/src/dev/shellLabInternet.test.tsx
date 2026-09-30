import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import App from "../App";
import { installShellLab } from "./shellLab";

afterEach(cleanup);

/** The dev scenarios `#/dev/shell/online` and `#/dev/shell/online-open` (visual QA of the online state). */
describe("shell lab: online scenarios", () => {
  it("shows the allowlist session with its live decisions", async () => {
    installShellLab("#/dev/shell/online");
    render(<App />);
    const region = await screen.findByRole("region", { name: "Internet access" });
    await waitFor(() => expect(region.textContent).toContain("Only these sites: rust-lang.org, doc.rust-lang.org, crates.io"));
    expect(region.textContent).toContain("not one of the allowed sites");
  });

  it("shows the open-web session with its warnings", async () => {
    installShellLab("#/dev/shell/online-open");
    render(<App />);
    const region = await screen.findByRole("region", { name: "Internet access" });
    await waitFor(() => expect(region.textContent).toContain("Open web"));
    expect(region.textContent).toContain("known malware site");
    expect(region.textContent).toContain("download type not allowed");
  });
});
