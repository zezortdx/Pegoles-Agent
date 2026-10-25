import { useCallback, useEffect, useRef, useState } from "react";
import { api, type StatusPayload } from "../lib/tauri";
import { errorText } from "../state/errors";
import type { IntelligenceState } from "../state/useIntelligence";
import type { LocalView } from "../state/localModel";
import {
  computerReady, imageReady, modelInstalled, planFor, setupProgress, type SetupPlan, type SetupProgress,
} from "./setupProgress";

export interface SetupRun {
  readonly progress: SetupProgress;
  /** "Set up Pegoles": starts (or resumes) every missing part, in order. */
  readonly start: () => void;
  /** Stop downloading; what is on disk is kept and resumed. */
  readonly pause: () => void;
  /** After a failure: try every part that isn't done again. */
  readonly retry: () => void;
}

interface Attempts { model: boolean; image: boolean; computer: boolean }
const NONE: Attempts = { model: false, image: false, computer: false };

/**
 * Drives "Set up Pegoles" from Core's state alone: the AI model, then the
 * computer image, then the computer's first start. Each step is started
 * once per attempt and then only watched, so a window reload (or Core
 * finishing a job on its own) never starts anything twice.
 */
export function useSetupRun(args: {
  readonly status: StatusPayload | null;
  readonly local: LocalView;
  readonly intelligence: IntelligenceState;
  readonly refresh: () => Promise<void>;
}): SetupRun {
  const { status, local, intelligence, refresh } = args;
  const [plan, setPlan] = useState<SetupPlan | null>(null);
  const [running, setRunning] = useState(false);
  const [paused, setPaused] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const attempts = useRef<Attempts>({ ...NONE });
  const busy = useRef(false);

  const effectivePlan = plan ?? planFor(local, status);
  const progress = setupProgress({ plan: effectivePlan, local, status, running, paused, commandError });

  const step = useCallback((command: () => Promise<unknown>) => {
    busy.current = true;
    void command()
      .catch((error: unknown) => setCommandError(errorText(error)))
      .finally(() => {
        busy.current = false;
        void refresh();
      });
  }, [refresh]);

  useEffect(() => {
    if (!running || paused || busy.current || commandError) return;
    const modelDone = !effectivePlan.model || modelInstalled(local);
    const imageDone = !effectivePlan.image || imageReady(status);
    if (!modelDone) {
      if (local.stage === "preparing" || attempts.current.model || !local.model) return;
      attempts.current.model = true;
      const id = local.model.id;
      step(() => (local.stage === "damaged" ? intelligence.repair(id) : intelligence.install(id)));
      return;
    }
    if (!imageDone) {
      if (status?.image_setup?.installing || attempts.current.image) return;
      attempts.current.image = true;
      step(() => api.installComputerImage());
      return;
    }
    if (!computerReady(status) && !attempts.current.computer) {
      attempts.current.computer = true;
      step(async () => {
        if (!status?.computer_created) await api.createComputer();
        // A computer left broken by an earlier attempt starts over cleanly.
        const broken = status?.computer_state === "error" || status?.guest_state === "error" || status?.guest_state === "incompatible";
        if (broken && status?.computer_state !== "stopped") await api.stopComputer();
        if (broken || status?.computer_state !== "running") await api.startComputer();
      });
    }
  }, [running, paused, commandError, effectivePlan, local, status, intelligence, step]);

  useEffect(() => {
    if (progress.phase === "done" && running) setRunning(false);
  }, [progress.phase, running]);

  const start = useCallback(() => {
    setPlan((current) => current ?? planFor(local, status));
    attempts.current = { ...NONE };
    setCommandError(null);
    setPaused(false);
    setRunning(true);
  }, [local, status]);

  const pause = useCallback(() => {
    setPaused(true);
    setRunning(false);
    if (local.stage === "preparing") void intelligence.cancelInstall();
    if (status?.image_setup?.installing) void api.cancelComputerImageInstall().finally(() => void refresh());
  }, [local.stage, intelligence, status?.image_setup?.installing, refresh]);

  const retry = useCallback(() => {
    attempts.current = { ...NONE };
    setCommandError(null);
    setPaused(false);
    setRunning(true);
  }, []);

  return { progress, start, pause, retry };
}
