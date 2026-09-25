import { describe, expect, it } from "vitest";
import { capabilityFor, eventsForTask, fileArtifacts } from "./execution";
import type { ActionRequestWire, AgentEvent } from "./tauri";

const at = "2026-09-24T10:00:00Z";
const request: ActionRequestWire = { action_id: "a1", task_id: "t1", computer_id: "vm1", action: { type: "click" }, requested_at: at };
const start: AgentEvent = { type: "action_started", request, action_id: "a1", at };
const done: AgentEvent = { type: "action_completed", request, result: { action_id: "a1", success: true, outcome: "executed", message: "", duration_ms: 20 } };

describe("event utilities", () => {
  it("correlates nested task actions and their frames without leaking another task", () => {
    const foreign = { ...start, request: { ...request, task_id: "t2", action_id: "a2" }, action_id: "a2" };
    const frame = { type: "frame_observed", action_id: "a1", at };
    expect(eventsForTask([start, foreign, frame, { type: "computer_created", computer_id: "vm1", at }], "t1")).toEqual([start, frame]);
  });
  it("only emits file artifacts for successful executed writes", () => {
    const write = { ...done, request: { ...request, action: { type: "write_file", path: "/workspace/report.md", content: "private contents" } } };
    expect(fileArtifacts([write])).toEqual([{ id: "a1", path: "/workspace/report.md", message: "" }]);
    expect(fileArtifacts([{ ...write, result: { success: false, outcome: "needs_approval" } }])).toEqual([]);
    expect(fileArtifacts([write, write])).toHaveLength(1);
  });
  it("maps verbs to capabilities and ignores malformed envelopes", () => {
    expect(capabilityFor("click")).toBe("Computer");
    expect(capabilityFor("write_file")).toBe("Files");
    expect(capabilityFor("open_url")).toBe("Web");
    expect(capabilityFor("teleport")).toBeUndefined();
    expect(eventsForTask([{ type: "future_event", request: null }], "t1")).toEqual([]);
  });
});
