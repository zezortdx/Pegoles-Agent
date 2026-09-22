import { GlassButton } from "@pegoles/ui";
import type { ComputerState } from "../lib/tauri";

interface Props {
  created: boolean;
  state: ComputerState | null;
  busy: boolean;
  error: string | null;
  onCreate: () => void;
  onStart: () => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}

function Btn({
  label,
  onClick,
  disabled,
  primary = false,
}: {
  label: string;
  onClick: () => void;
  disabled: boolean;
  primary?: boolean;
}) {
  return (
    <GlassButton onClick={onClick} disabled={disabled} variant={primary ? "primary" : "secondary"} size="sm">
      {label}
    </GlassButton>
  );
}

export function ComputerControls(p: Props) {
  if (!p.created) {
    return (
      <div className="computer-actions">
        <Btn label={p.busy ? "Creating…" : "Create Computer"} onClick={p.onCreate} disabled={p.busy} primary />
      </div>
    );
  }
  return (
    <div className="computer-actions">
      <Btn label="Start" onClick={p.onStart} disabled={p.busy || p.state !== "stopped"} primary={p.state === "stopped"} />
      <Btn label="Pause" onClick={p.onPause} disabled={p.busy || p.state !== "running"} />
      <Btn label="Resume" onClick={p.onResume} disabled={p.busy || p.state !== "paused"} />
      <Btn label="Stop" onClick={p.onStop} disabled={p.busy || (p.state !== "running" && p.state !== "paused")} />
    </div>
  );
}
