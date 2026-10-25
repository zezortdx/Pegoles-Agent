import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SnapshotControl, guestPoint } from "./SnapshotControl";
import { api } from "../lib/tauri";
vi.mock("../lib/tauri", () => ({ api: { humanInput: vi.fn().mockResolvedValue(undefined), returnControl: vi.fn().mockResolvedValue(undefined) } }));
afterEach(cleanup);
const meta = { frame_id: "frame", computer_id: "computer", width_px: 200, height_px: 100 };
describe("snapshot human control", () => {
  it("maps the contained image and rejects letterbox and nonfinite points", () => {
    const box = { left: 10, top: 20, width: 200, height: 200 };
    expect(guestPoint(box, 200, 100, 110, 120)).toEqual({ x: .5, y: .5 });
    expect(guestPoint(box, 200, 100, 10, 20)).toBeNull();
    expect(guestPoint(box, 200, 100, NaN, 120)).toBeNull();
    expect(guestPoint({ ...box, width: 0 }, 200, 100, 10, 20)).toBeNull();
  });
  it("never dispatches without confirmed ownership and releases on focus loss", async () => {
    vi.mocked(api.humanInput).mockClear(); vi.mocked(api.returnControl).mockClear();
    const { rerender } = render(<SnapshotControl src="data:image/png;base64,AA==" meta={meta} generation={4} enabled={false} />);
    const surface = screen.getByRole("application");
    fireEvent.keyDown(surface, { key: "a" }); expect(api.humanInput).not.toHaveBeenCalled();
    rerender(<SnapshotControl src="data:image/png;base64,AA==" meta={meta} generation={4} enabled />);
    fireEvent.keyDown(surface, { key: "a" });
    await waitFor(() => expect(api.humanInput).toHaveBeenCalledWith({computer_id:"computer",frame_id:"frame",width:200,height:100,generation:4},{type:"text",text:"a"}));
    fireEvent.blur(surface); await waitFor(() => expect(api.returnControl).toHaveBeenCalled());
  });
  it("allows the explicit paste shortcut to produce a browser paste event", () => {
    vi.mocked(api.humanInput).mockClear();
    render(<SnapshotControl src="data:image/png;base64,AA==" meta={meta} generation={4} enabled />);
    const surface = screen.getByRole("application");
    for (const modifier of [{ ctrlKey: true }, { metaKey: true }]) {
      expect(fireEvent.keyDown(surface, { key: "v", ...modifier })).toBe(true);
    }
    expect(api.humanInput).not.toHaveBeenCalled();
    fireEvent.paste(surface, { clipboardData: { getData: () => "manual paste" } });
    expect(api.humanInput).toHaveBeenCalledWith(expect.anything(), { type: "text", text: "manual paste" });
  });
  it("drops the bounded queue on focus loss while one event is in flight", async () => {
    let finish: (() => void) | undefined;
    vi.mocked(api.humanInput).mockClear(); vi.mocked(api.returnControl).mockClear();
    vi.mocked(api.humanInput).mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    render(<SnapshotControl src="data:image/png;base64,AA==" meta={meta} generation={4} enabled />);
    const surface = screen.getByRole("application");
    fireEvent.keyDown(surface, {key:"a"}); fireEvent.keyDown(surface, {key:"b"});
    fireEvent.blur(surface); finish?.();
    await waitFor(() => expect(api.returnControl).toHaveBeenCalled());
    expect(api.humanInput).toHaveBeenCalledTimes(1);
  });
  it("refuses oversized paste and keeps hostile guest text inert", () => {
    vi.mocked(api.humanInput).mockClear();
    render(<SnapshotControl src="data:image/png;base64,AA==" meta={meta} generation={4} enabled />);
    const surface=screen.getByRole("application");
    fireEvent.paste(surface,{clipboardData:{getData:()=>"x".repeat(257)}});
    expect(api.humanInput).not.toHaveBeenCalled();
    fireEvent.paste(surface,{clipboardData:{getData:()=>"<script>alert(1)</script>"}});
    expect(surface.querySelector("script")).toBeNull();
  });
});
