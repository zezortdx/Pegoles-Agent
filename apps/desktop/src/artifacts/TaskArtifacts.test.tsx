import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api, type TaskHistory } from "../lib/tauri";
import { TaskArtifacts } from "./TaskArtifacts";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const task = { id:"t1",title:"Report",status:"completed" as const,created_at:"2026-10-24T12:00:00Z",updated_at:"2026-10-24T12:00:00Z" };
const history = (data: NonNullable<TaskHistory["artifact"]>["data"]): TaskHistory => ({ task, audit:[], recovered:false, artifact:{task_id:task.id,data} });
describe("task result artifacts", () => {
  it("loads a genuine bounded summary by task id and renders hostile text inert", async () => {
    const hostile='<img src="https://evil.test/" onerror="alert(1)"><script>bad()</script> file:///etc/passwd';
    vi.spyOn(api,"getTaskHistory").mockResolvedValue(history({kind:"text",text:hostile}));
    const {container}=render(<TaskArtifacts taskId="t1" revision="one" enabled />);
    expect(await screen.findByText(hostile)).toBeTruthy(); expect(screen.getByText("Agent summary · text result")).toBeTruthy();
    expect(container.querySelector("img,script,a,iframe")).toBeNull(); expect(api.getTaskHistory).toHaveBeenCalledWith("t1");
  });
  it("table cells remain text nodes and task navigation never flashes another task result",async()=>{
    vi.spyOn(api,"getTaskHistory").mockResolvedValue(history({kind:"table",columns:["<svg onload=bad()>"],rows:[["[click](file:///etc/passwd)"]]}));
    const view=render(<TaskArtifacts taskId="t1" revision="one" enabled />);
    expect(await screen.findByRole("columnheader",{name:"<svg onload=bad()>"})).toBeTruthy();
    expect(screen.getByRole("cell",{name:"[click](file:///etc/passwd)"})).toBeTruthy(); expect(view.container.querySelector("svg,a")).toBeNull();
    vi.mocked(api.getTaskHistory).mockReturnValue(new Promise(()=>undefined)); view.rerender(<TaskArtifacts taskId="t2" revision="one" enabled />);
    expect(screen.queryByRole("table")).toBeNull(); expect(screen.getByText("Loading saved results…")).toBeTruthy();
  });
  it("shows an honest empty state, recovered notice and bounded saved status audit",async()=>{
    vi.spyOn(api,"getTaskHistory").mockResolvedValue({...history({kind:"text",text:"unused"}),artifact:null,recovered:true,audit:[{status:"running",at:task.updated_at}]});
    render(<TaskArtifacts taskId="t1" revision="one" enabled />);
    expect(await screen.findByText("No saved result for this task.")).toBeTruthy(); expect(screen.getByText("Closed before completion. This task won’t resume automatically.")).toBeTruthy(); expect(screen.getByText("Saved task history")).toBeTruthy();
  });
  it("rejects wrong-task data and reports load failure without claiming no artifacts exist",async()=>{
    vi.spyOn(api,"getTaskHistory").mockResolvedValue({...history({kind:"text",text:"another task result"}),task:{...task,id:"other"}});
    const view=render(<TaskArtifacts taskId="t1" revision="one" enabled />);
    expect(await screen.findByRole("alert")).toHaveProperty("textContent","Could not load saved results."); expect(screen.queryByText("another task result")).toBeNull();
    vi.mocked(api.getTaskHistory).mockRejectedValue(new Error("disk")); view.rerender(<TaskArtifacts taskId="t1" revision="two" enabled />);
    await waitFor(()=>expect(api.getTaskHistory).toHaveBeenCalledTimes(2)); expect(await screen.findByRole("alert")).toBeTruthy();
  });
});
