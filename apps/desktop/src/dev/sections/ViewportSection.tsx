import { useEffect, useRef, useState } from "react";
import {
  ComputerViewport,
  GlassButton,
  PauseIcon,
  PlayIcon,
  PowerIcon,
  StopIcon,
  VIEWPORT_STATE_SPECS,
  VIEWPORT_STATES,
  type ViewportState,
} from "@pegoles/ui";
import { ERROR_FIXTURE, stagesFor } from "../fixtures";
import { LabSection, Segmented, SimulatedTag, Switch } from "../lab/controls";

const BOOT_SEQUENCE: readonly ViewportState[] = [
  "preparing",
  "starting",
  "guest_connecting",
  "display_starting",
  "ready",
];
const BOOT_STEP_MS = 1500;

/** Stand-in for the guest framebuffer (Weston + foot). Lab only. */
function SimulatedDisplay({ compact = false }: { readonly compact?: boolean }) {
  return (
    <div className="lab-display" data-compact={compact ? "true" : undefined}>
      <div className="lab-display__terminal">
        <div className="lab-display__titlebar">foot — pegoles@computer</div>
        <pre className="lab-display__screen">
          {`pegoles@computer:~$ uname -sr
Linux 6.12.43+deb13-arm64
pegoles@computer:~$ df -h /
Filesystem  Size  Used Avail Use% Mounted on
/dev/vda1    16G  3.1G   12G  21% /
pegoles@computer:~$ `}
          <span className="lab-display__caret" />
        </pre>
      </div>
      {!compact && <span className="lab-display__tag">Simulated display · design lab only</span>}
    </div>
  );
}

export function ViewportSection({ initialState = "ready" }: { readonly initialState?: ViewportState }) {
  const [state, setState] = useState<ViewportState>(initialState);
  const [native, setNative] = useState(false);
  const [variant, setVariant] = useState<"full" | "compact">("full");
  const [playing, setPlaying] = useState(false);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    if (!playing) return undefined;
    let step = 0;
    setState(BOOT_SEQUENCE[0] ?? "preparing");
    timer.current = window.setInterval(() => {
      step += 1;
      const next = BOOT_SEQUENCE[step];
      if (next) setState(next);
      if (step >= BOOT_SEQUENCE.length - 1) setPlaying(false);
    }, BOOT_STEP_MS);
    return () => {
      if (timer.current !== null) window.clearInterval(timer.current);
    };
  }, [playing]);

  const pick = (next: ViewportState) => {
    setPlaying(false);
    setState(next);
  };

  const running = state !== "off" && state !== "error";
  const headerActions = running ? (
    <>
      <GlassButton
        variant="quiet"
        size="sm"
        iconOnly
        icon={state === "paused" ? <PlayIcon size={12} /> : <PauseIcon size={12} />}
        onClick={() => pick(state === "paused" ? "ready" : "paused")}
        disabled={VIEWPORT_STATE_SPECS[state].booting}
      >
        {state === "paused" ? "Resume" : "Pause"}
      </GlassButton>
      <GlassButton variant="quiet" size="sm" iconOnly icon={<StopIcon size={12} />} onClick={() => pick("off")}>
        Stop
      </GlassButton>
    </>
  ) : null;

  return (
    <LabSection
      id="lab-computer"
      index="08"
      title="Pegoles Computer"
      lead="Chrome around a framebuffer slot that keeps the guest's aspect ratio. With a native surface the slot is a hole — nothing in the DOM may paint over it. The edge tells you who is there: faint blue when ready, electric when Pegoles works, white when you do."
    >
      <div className="lab-viewport-controls">
        <Segmented
          label="Viewport state"
          value={state}
          onChange={pick}
          size="sm"
          options={VIEWPORT_STATES.map((s) => ({ value: s, label: s.replace("_", " ") }))}
        />
        <div className="lab-viewport-controls__row">
          <Segmented
            label="Variant"
            value={variant}
            onChange={setVariant}
            size="sm"
            options={[
              { value: "full", label: "Full" },
              { value: "compact", label: "Compact" },
            ]}
          />
          <Switch label="Native surface" hint="Slot becomes a hole" checked={native} onChange={setNative} />
          <GlassButton size="sm" icon={<PlayIcon size={12} />} onClick={() => setPlaying(true)} disabled={playing}>
            Play boot sequence
          </GlassButton>
          <SimulatedTag>Simulated states</SimulatedTag>
        </div>
      </div>

      <div className="lab-viewport" data-variant={variant}>
        <ComputerViewport
          state={state}
          variant={variant}
          nativeSurface={native}
          headingLevel={3}
          subtitle="Debian 13 · Weston"
          statusDetail={state === "ready" ? "5.6 s" : undefined}
          bootStages={stagesFor(state)}
          errorMessage={ERROR_FIXTURE}
          errorActions={
            <GlassButton size="sm" onClick={() => setPlaying(true)}>
              Restart
            </GlassButton>
          }
          offActions={
            <GlassButton variant="primary" icon={<PowerIcon size={14} />} onClick={() => setPlaying(true)}>
              Start Pegoles Computer
            </GlassButton>
          }
          headerActions={headerActions}
          onTakeControl={() => pick("user_controlled")}
          onReturnControl={() => pick("ready")}
          placeholder={<SimulatedDisplay compact={variant === "compact"} />}
        />
      </div>

      <h3 className="lab-subhead">Every state · compact</h3>
      <div className="lab-viewport-grid">
        {VIEWPORT_STATES.map((s) => (
          <ComputerViewport
            key={s}
            state={s}
            variant="compact"
            headingLevel={4}
            title={s.replace("_", " ")}
            bootStages={stagesFor(s)}
            errorMessage={ERROR_FIXTURE}
            placeholder={<SimulatedDisplay compact />}
            onTakeControl={() => pick("user_controlled")}
            onReturnControl={() => pick("ready")}
          />
        ))}
      </div>
    </LabSection>
  );
}
