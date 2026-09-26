import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { api, type AgentEvent, type AgentTask, type ModelSettings, type StatusPayload } from "../lib/tauri";
import { intelligenceOf, NO_KEY } from "../state/intelligenceFixture";

/**
 * Everything the model, the guest or an error message says reaches the page
 * as untrusted text (CLAUDE.md: the frontend never renders model/guest text
 * as HTML). These tests feed hostile strings through every such channel of
 * the real App and check they render as inert text: no element, attribute,
 * style or link is ever made from them.
 */
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

const P = {
  script: `<script id="xss-script">window.__pwned = "script"</script>`,
  img: `<img id="xss-img" src=x onerror="window.__pwned='img'">`,
  link: `<a id="xss-link" href="javascript:window.__pwned='link'">open</a>`,
  jsUrl: `javascript:window.__pwned='url'`,
  svg: `<svg id="xss-svg" onload="window.__pwned='svg'"><a xlink:href="javascript:alert(1)"><circle r="40"/></a></svg>`,
  css: `<style id="xss-style">*{display:none!important}</style><div id="xss-css" style="position:fixed;inset:0;background:url(https://evil.example/x)">`,
  bidi: "Report ‮txt.exe‬ ⁦isolated⁩ ‏ done",
  frame: `<iframe id="xss-frame" srcdoc="<script>parent.__pwned='frame'</script>"></iframe>`,
  entity: "&lt;b id=&quot;xss-entity&quot;&gt;bold&lt;/b&gt; &#x3C;img id=xss-entity2 src=x onerror=alert(1)&#x3E;",
};

const TITLE = `Clean up ${P.script}${P.img}`;
const at = (s: number) => `2026-09-22T10:00:${String(s).padStart(2, "0")}Z`;
const task: AgentTask = { id: "task-x", title: TITLE, status: "failed", created_at: at(0), updated_at: at(30) };
const request = (id: string, action: { type: string; [k: string]: unknown }, s: number) => ({
  action_id: id, task_id: task.id, computer_id: "vm", action, requested_at: at(s),
});
const write = request("a-write", { type: "write_file", path: `/workspace/${P.img}${P.bidi}.txt`, content: P.frame }, 3);
const typed = request("a-type", { type: "type_text", text: P.img, sensitive: false }, 5);
const visit = request("a-url", { type: "open_url", url: P.jsUrl }, 7);
const denied = request("a-denied", { type: "shell", command: P.svg }, 9);

const EVENTS: AgentEvent[] = [
  { type: "task_created", task_id: task.id, title: TITLE, at: at(0) },
  { type: "task_status_changed", task_id: task.id, from: "pending", to: "running", at: at(1) },
  { type: "agent_message", task_id: task.id, kind: "progress", text: `${P.img}${P.link}`, at: at(2) },
  { type: "action_started", action_id: write.action_id, request: write, at: at(3) },
  { type: "action_completed", request: write, result: { action_id: write.action_id, outcome: "executed", success: true, message: P.svg, duration_ms: 12 } },
  { type: "action_started", action_id: typed.action_id, request: typed, at: at(5) },
  { type: "action_failed", action_id: typed.action_id, request: typed, error: P.script, at: at(6) },
  { type: "action_started", action_id: visit.action_id, request: visit, at: at(7) },
  { type: "action_completed", request: visit, result: { action_id: visit.action_id, outcome: "executed", success: true, message: P.link, duration_ms: 3 } },
  { type: "action_denied", request: denied, reason: `${P.css}${P.entity}` },
  { type: "approval_requested", task_id: task.id, reason: P.img, at: at(10) },
  { type: "guest_runtime_error", computer_id: "vm", message: P.svg, at: at(11) },
  { type: "guest_runtime_disconnected", computer_id: "vm", reason: P.frame, at: at(12) },
  { type: "agent_message", task_id: task.id, kind: "summary", text: `${P.svg}${P.bidi}`, at: at(13) },
  { type: "agent_message", task_id: task.id, kind: "error", text: `${P.css}\n${P.entity}`, at: at(14) },
  { type: "task_status_changed", task_id: task.id, from: "running", to: "failed", at: at(30) },
];

const STATUS: StatusPayload = {
  core: "running", model: "configured", provider: "local", backend: "real", computer_created: true,
  computer_state: "error", computer_id: "vm", image_status: "ready", spec_os: "Debian 13",
  spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "error",
  guest_ready_ms: null, viewport_state: "error", viewport_issue: P.script, display_available: false,
  display_attached: false, display_config: null, display_error: P.img, display_setup_error: P.css, control_owner: "none",
  input_available: false, agent_busy: false, active_task: null,
};

const BOOT_LOG = [P.script, P.img, P.svg, P.bidi, "\u001b[31mred\u001b[0m", P.entity];

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("min-width"), media: query, addEventListener: () => undefined, removeEventListener: () => undefined,
  }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(api, "getStatus").mockResolvedValue(STATUS);
  vi.spyOn(api, "listEvents").mockResolvedValue(EVENTS);
  vi.spyOn(api, "listTasks").mockResolvedValue([task]);
  vi.spyOn(api, "getModelSettings").mockResolvedValue(NO_KEY satisfies ModelSettings);
  vi.spyOn(api, "getIntelligence").mockResolvedValue(intelligenceOf());
  vi.spyOn(api, "getHostCapabilities").mockResolvedValue({ platform: "macos", architecture: "arm64", backend: "real", backend_available: true, backend_detail: P.img, guest_transport: "virtio_socket", guest_transport_available: true, required_setup: [P.script], supported: true });
  vi.spyOn(api, "suggestedEffects").mockResolvedValue({ tier: "reduced" });
  vi.spyOn(api, "accessibilityDisplay").mockResolvedValue({ reduce_transparency: false, increase_contrast: false });
  vi.spyOn(api, "captureScreen").mockRejectedValue(P.img);
  vi.spyOn(api, "readBootLog").mockResolvedValue({ available: true, total_lines: BOOT_LOG.length, tail: BOOT_LOG });
  vi.spyOn(api, "runTask").mockRejectedValue(P.img);
  vi.spyOn(api, "startComputer").mockRejectedValue(`computer error: ${P.script}`);
});
afterEach(() => {
  cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  delete (window as { __pwned?: unknown }).__pwned;
});

const sidebar = () => screen.getByRole("complementary", { name: "Sidebar" });
const panel = () => screen.getByRole("complementary", { name: "Computer" });

/** Attributes that load, link or style something: never made from untrusted text. */
const ACTIVE_ATTRIBUTES = new Set(["href", "src", "srcset", "action", "formaction", "data", "poster", "background", "style", "id", "class", "for", "name"]);

/**
 * Nothing on the page was made from a hostile string: only text. Plain-text
 * attributes (title, aria-label) may carry it; React escapes them.
 */
function expectInert() {
  const root = document.body;
  expect(root.querySelector("[id^='xss-'], [id^='xss']"), "an element was injected").toBeNull();
  expect(root.querySelectorAll("script, iframe, frame, object, embed, style, link, meta, base, form[action]")).toHaveLength(0);
  for (const element of root.querySelectorAll("*")) {
    for (const { name, value } of element.attributes) {
      const attribute = name.toLowerCase();
      expect(attribute.startsWith("on"), `${element.tagName} has ${name}`).toBe(false);
      expect(["srcdoc", "xlink:href"], `${element.tagName} has ${name}`).not.toContain(attribute);
      if (ACTIVE_ATTRIBUTES.has(attribute)) {
        expect(/javascript:|evil\.example|xss|__pwned|<|>/i.test(value), `${element.tagName}[${name}]=${value}`).toBe(false);
      }
    }
  }
  for (const link of root.querySelectorAll("a[href]")) expect(link.getAttribute("href")?.startsWith("#")).toBe(true);
  expect((window as { __pwned?: unknown }).__pwned).toBeUndefined();
}

const shows = (text: string) => expect(document.body.textContent ?? "").toContain(text);

describe("untrusted text renders as inert text", () => {
  it("in a task: its title, the model's narration, file paths and action failures", async () => {
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /xss-script/ }));
    expect(await screen.findByRole("heading", { level: 1, name: TITLE })).toBeTruthy();
    await waitFor(() => shows(`${P.img}${P.link}`));
    shows(TITLE);
    shows(`${P.img}${P.bidi}.txt`);
    shows(`${P.svg}${P.bidi}`);
    shows(P.css);
    expectInert();
  });

  it("in a refused start: Core's hostile error is summarized, never rendered", async () => {
    const pending: AgentTask = { ...task, status: "pending", updated_at: at(0) };
    vi.mocked(api.listTasks).mockResolvedValue([pending]);
    vi.mocked(api.runTask).mockRejectedValue(`Refused: ${P.link}`);
    vi.mocked(api.listEvents).mockResolvedValue([]);
    vi.mocked(api.getStatus).mockResolvedValue({ ...STATUS, computer_created: false, computer_state: null, viewport_state: "off", guest_state: "unavailable" });
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /xss-script/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    await waitFor(() => expect(api.runTask).toHaveBeenCalledExactlyOnceWith(pending.id));
    await waitFor(() => shows("Pegoles couldn’t start this task."));
    expectInert();
  });

  it("in the activity history: approvals, policy denials and guest errors", async () => {
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: "Activity" }));
    await waitFor(() => shows(`Asked for approval: ${P.img}`));
    expectInert();
  });

  it("in the computer's Details: display errors, command errors and the guest's boot log", async () => {
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /^Pegoles Computer/ }));
    const summary = within(panel()).getByText("Details", { selector: ".computer__details summary" });
    const details = summary.parentElement as HTMLDetailsElement;
    await act(async () => {
      details.open = true;
      fireEvent(details, new Event("toggle"));
    });
    await waitFor(() => shows(P.entity));
    for (const line of BOOT_LOG) shows(line);
    shows(P.img);
    shows(P.css);
    expectInert();

    // A failed retry puts Core's (hostile) error in Details too.
    fireEvent.click(within(panel()).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(api.startComputer).toHaveBeenCalledOnce());
    await waitFor(() => shows(`computer error: ${P.script}`));
    expectInert();
  });

  it("in Settings: errors from Core and the local runtime's problem", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ local: { runtime_ready: false, runtime_problem: P.svg } }));
    vi.spyOn(api, "setProvider").mockRejectedValue(`${P.img}${P.link}`);
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: "Settings" }));
    fireEvent.click(await screen.findByRole("radio", { name: "Anthropic" }));
    await waitFor(() => expect(screen.getAllByRole("alert").some((alert) => alert.textContent?.includes(`${P.img}${P.link}`))).toBe(true));
    await waitFor(() => shows(P.svg));
    expectInert();
  });

  it("the check itself catches markup that became elements", () => {
    const probe = document.createElement("div");
    probe.innerHTML = P.img;
    document.body.append(probe);
    try {
      expect(() => expectInert()).toThrow();
    } finally {
      probe.remove();
    }
  });
});
