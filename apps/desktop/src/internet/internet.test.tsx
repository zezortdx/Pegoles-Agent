import { useState } from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Composer } from "../composer/Composer";
import type { EgressDecision, InternetStatus } from "../lib/tauri";
import { InternetChip, InternetPanel } from "./InternetControl";
import { InternetIndicator } from "./InternetIndicator";
import { INTERNET_OFF, MAX_SITES, decisionLine, draftProblem, parseSite, toAccess, type InternetDraft } from "./model";

afterEach(cleanup);

function Harness({ initial = INTERNET_OFF, onDraft }: { initial?: InternetDraft; onDraft?: (draft: InternetDraft) => void }) {
  const [draft, setDraft] = useState(initial);
  return <InternetPanel draft={draft} onChange={(next) => { setDraft(next); onDraft?.(next); }} onClose={() => undefined} />;
}

const addSite = (text: string) => {
  const input = screen.getByLabelText("Add a site");
  fireEvent.change(input, { target: { value: text } });
  fireEvent.keyDown(input, { key: "Enter" });
};

describe("internet choice", () => {
  it("is off by default and sends no sites", () => {
    expect(INTERNET_OFF.mode).toBe("off");
    expect(toAccess(INTERNET_OFF)).toEqual({ mode: "off", domains: [] });
    render(<Harness />);
    expect(screen.getByRole("radio", { name: "Off" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByLabelText("Add a site")).toBeNull();
  });

  it("turns typed sites into clean chips and sends only names", () => {
    const seen = vi.fn();
    render(<Harness onDraft={seen} />);
    fireEvent.click(screen.getByRole("radio", { name: "Only these sites" }));
    addSite("https://Docs.Example.com/path?q=1");
    addSite("example.org");
    addSite("example.org");
    const list = screen.getByRole("list", { name: "Allowed sites" });
    expect(within(list).getAllByText(/example/u).map((node) => node.textContent)).toEqual(["docs.example.com", "example.org"]);
    const [last] = seen.mock.lastCall ?? [];
    expect(toAccess(last as InternetDraft)).toEqual({ mode: "allowlist", domains: ["docs.example.com", "example.org"] });
    fireEvent.click(screen.getByRole("button", { name: "Remove example.org" }));
    expect(screen.queryByText("example.org")).toBeNull();
  });

  it("explains what it cannot take instead of adding it", () => {
    render(<Harness initial={{ mode: "allowlist", sites: [] }} />);
    for (const [typed, hint] of [["*.example.com", /wildcards/iu], ["10.0.0.1", /IP address/u], ["localhost", /site name/u]] as const) {
      addSite(typed);
      expect(screen.getByRole("alert").textContent).toMatch(hint);
    }
    expect(screen.queryByRole("button", { name: /^Remove/u })).toBeNull();
  });

  it("holds the limit of 32 sites", () => {
    render(<Harness initial={{ mode: "allowlist", sites: [] }} />);
    const many = Array.from({ length: MAX_SITES + 3 }, (_, i) => `site${i}.example.com`).join(" ");
    fireEvent.paste(screen.getByLabelText("Add a site"), { clipboardData: { getData: () => many } });
    expect(screen.getAllByRole("button", { name: /^Remove/u })).toHaveLength(MAX_SITES);
    expect(screen.getByRole("alert").textContent).toContain(`${MAX_SITES}`);
  });

  it("warns plainly about open web", () => {
    render(<Harness initial={{ mode: "open_web", sites: [] }} />);
    expect(screen.getByRole("note").textContent).toMatch(/almost any public site/u);
    expect(screen.getByRole("note").textContent).toMatch(/secrets/u);
  });

  it("names the chip after the choice and blocks an empty site list", () => {
    const toggle = vi.fn();
    const { rerender } = render(<InternetChip draft={INTERNET_OFF} open={false} onToggle={toggle} />);
    expect(screen.getByRole("button").getAttribute("aria-label")).toContain("Off");
    fireEvent.click(screen.getByRole("button"));
    expect(toggle).toHaveBeenCalledOnce();
    rerender(<InternetChip draft={{ mode: "allowlist", sites: ["a.example", "b.example"] }} open onToggle={toggle} />);
    expect(screen.getByRole("button").getAttribute("aria-label")).toContain("2 sites");
    expect(draftProblem({ mode: "allowlist", sites: [] })).toMatch(/at least one site/u);
    expect(draftProblem({ mode: "open_web", sites: [] })).toBeNull();
  });

  it("does not let the composer hand over an incomplete choice", () => {
    const submit = vi.fn();
    render(<Composer value="Look it up" onChange={() => undefined} onSubmit={submit} placeholder="Task" blocker={draftProblem({ mode: "allowlist", sites: [] })} />);
    fireEvent.submit(screen.getByRole("form"));
    expect(submit).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toMatch(/at least one site/u);
  });
});

describe("site parsing", () => {
  it("accepts names and trims addresses down to them", () => {
    expect(parseSite(" Example.COM. ")).toEqual({ site: "example.com" });
    expect(parseSite("http://a.example.com:8080/x")).toEqual({ problem: expect.any(String) });
    expect(parseSite("https://a.example.com/x#y")).toEqual({ site: "a.example.com" });
    expect(parseSite("")).toHaveProperty("problem");
    expect(parseSite("user@example.com")).toHaveProperty("problem");
  });
});

const decision = (host: string, allowed: boolean, reason: string, bytes = 0): EgressDecision =>
  ({ task_id: "t1", host, allowed, reason, bytes, dropped_before: 0, at: new Date(0).toISOString() });

const status = (overrides: Partial<InternetStatus> = {}): InternetStatus => ({
  active: true, task_id: "t1", mode: "allowlist", domains: ["example.com", "docs.example.org"], allowed: 1, blocked: 2, dropped: 0,
  recent: [decision("example.com", true, "allowed", 2048), decision("bad.example.net", false, "threat_malware"), decision("files.example.com", false, "inspect_download_not_safe")],
  ...overrides,
});

describe("internet indicator", () => {
  it("shows nothing while the task is offline", () => {
    const { container } = render(<InternetIndicator internet={status({ active: false })} />);
    expect(container.firstChild).toBeNull();
  });

  it("shows the mode, the sites and each decision in plain words, newest first", () => {
    render(<InternetIndicator internet={status()} />);
    const region = screen.getByRole("region", { name: "Internet access" });
    expect(region.textContent).toContain("Internet is on for this task");
    expect(region.textContent).toContain("Only these sites: example.com, docs.example.org");
    expect(region.textContent).toContain("1 allowed · 2 blocked");
    const rows = within(screen.getByRole("list", { name: /Recent requests/u })).getAllByRole("listitem").map((row) => row.textContent);
    expect(rows[0]).toBe("Blockedfiles.example.comdownload type not allowed");
    expect(rows[1]).toBe("Blockedbad.example.netknown malware site");
    expect(rows[2]).toBe("Allowedexample.com2.0 KB");
  });

  it("says open web in words, and collapses to its header", () => {
    render(<InternetIndicator internet={status({ mode: "open_web", domains: [] })} />);
    expect(screen.getByRole("region").textContent).toContain("Open web");
    fireEvent.click(screen.getByRole("button", { name: /Internet is on/u }));
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("renders hosts from the computer as text, never markup", () => {
    const hostile = "<img src=x onerror=alert(1)>.example.com";
    const { container } = render(<InternetIndicator internet={status({ recent: [decision(hostile, false, "transport_invalid_host")] })} />);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText(hostile)).toBeTruthy();
  });

  it("words every reason Core can give", () => {
    expect(decisionLine(decision("a.example", false, "made_up_code")).text).toBe("not allowed");
    expect(decisionLine(decision("a.example", false, "upstream_connect")).label).toBe("Failed");
    expect(decisionLine(decision("", true, "allowed")).host).toBe("unknown site");
  });
});
