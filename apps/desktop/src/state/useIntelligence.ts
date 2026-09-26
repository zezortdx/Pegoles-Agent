import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api, MODEL_INSTALL_EVENT, type InstallErrorKind, type InstallPhase, type Intelligence, type ModelInstallStatus, type Provider,
} from "../lib/tauri";
import { errorText } from "./errors";
import { installActive, sentence } from "./localModel";

/** What the Intelligence settings are doing: reading, choosing who plans, setting up, cancelling or removing a local model. */
export type IntelligenceOp = "load" | "provider" | "install" | "cancel" | "remove";

export interface IntelligenceError {
  readonly op: IntelligenceOp;
  /** Core's own words. */
  readonly text: string;
}

export interface IntelligenceState {
  readonly intelligence: Intelligence | null;
  readonly pending: IntelligenceOp | null;
  readonly error: IntelligenceError | null;
  /** Read Core again, quietly (never blocked by an operation in flight). */
  readonly refresh: () => void;
  readonly setProvider: (provider: Provider, localModel?: string) => Promise<boolean>;
  /** Set up a local model (default: the chosen one). A partial download resumes. */
  readonly install: (model?: string) => Promise<boolean>;
  readonly cancelInstall: () => Promise<boolean>;
  readonly remove: (model: string) => Promise<boolean>;
  /** Remove files that failed the store's check, then set the model up again. */
  readonly repair: (model: string) => Promise<boolean>;
}

const FAILED: Record<IntelligenceOp, string> = {
  load: "Couldn’t read the Intelligence settings.",
  provider: "Couldn’t change the model.",
  install: "Couldn’t set up Pegoles Local.",
  cancel: "Couldn’t cancel the setup.",
  remove: "Couldn’t remove the model.",
};

/** A failed operation as one sentence: ours, then Core's own words. */
export function intelligenceProblem(error: IntelligenceError | null, ops?: readonly IntelligenceOp[]): string | null {
  if (!error || (ops && !ops.includes(error.op))) return null;
  return `${FAILED[error.op]} ${sentence(error.text)}`;
}

const PHASES = new Set<InstallPhase>(["idle", "downloading", "verifying", "finalizing", "ready", "failed", "cancelled"]);
const KINDS = new Set<InstallErrorKind>(["disk_space", "network", "corrupted", "other"]);
const SETTLED = new Set<InstallPhase>(["ready", "failed", "cancelled"]);
/** While a setup runs, Core is read again if no progress event arrived for this long (a missed event never strands it). */
const WATCHDOG_MS = 4000;
const WATCH_EVERY_MS = 1000;

const count = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value) && value >= 0;

/** The event payload is checked before it is believed. */
export function isInstallStatus(value: unknown): value is ModelInstallStatus {
  if (!value || typeof value !== "object") return false;
  const status = value as Record<string, unknown>;
  return typeof status.model === "string" && PHASES.has(status.phase as InstallPhase) &&
    count(status.done_bytes) && count(status.total_bytes) &&
    (status.error === null || typeof status.error === "string") &&
    (status.error_kind === null || KINDS.has(status.error_kind as InstallErrorKind));
}

function withInstall(intelligence: Intelligence, install: ModelInstallStatus): Intelligence {
  return { ...intelligence, local: { ...intelligence.local, install } };
}

/**
 * Who plans, as Core reports it, kept live while a local model is set up:
 * progress arrives on `pegoles://model-install` and Core is read again
 * whenever a setup settles (ready, failed or cancelled). `onChanged` lets
 * Core's status (whether a task can start) catch up after a change.
 */
export function useIntelligence(enabled: boolean, onChanged?: () => void): IntelligenceState {
  const [intelligence, setIntelligence] = useState<Intelligence | null>(null);
  const [pending, setPending] = useState<IntelligenceOp | null>(null);
  const [error, setError] = useState<IntelligenceError | null>(null);
  const alive = useRef(false);
  const busy = useRef(false);
  const on = useRef(enabled);
  const changed = useRef(onChanged);
  /** Orders requests and events: an event newer than a request knows better about the setup in flight. */
  const clock = useRef(0);
  const latest = useRef<{ status: ModelInstallStatus; at: number } | null>(null);
  const heard = useRef(0);
  /** Live events couldn't be subscribed to. */
  const deaf = useRef(false);

  useEffect(() => { changed.current = onChanged; }, [onChanged]);
  useEffect(() => { on.current = enabled; }, [enabled]);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);

  const apply = useCallback((next: Intelligence, issued: number) => {
    if (!alive.current) return;
    const event = latest.current;
    setIntelligence(event && event.at > issued ? withInstall(next, event.status) : next);
  }, []);

  /** `settled`: a setup ended, so what Core can run changed and its status should catch up. */
  const reload = useCallback(async (settled: boolean) => {
    if (!on.current) return;
    const issued = ++clock.current;
    try {
      apply(await api.getIntelligence(), issued);
      if (settled) changed.current?.();
    } catch (raw) {
      if (alive.current) setError({ op: "load", text: errorText(raw).trim() || "No details were reported." });
    }
  }, [apply]);

  const perform = useCallback(async (op: IntelligenceOp, command: () => Promise<Intelligence>): Promise<boolean> => {
    if (busy.current) return false;
    busy.current = true;
    setPending(op);
    setError(null);
    const issued = ++clock.current;
    try {
      apply(await command(), issued);
      if (op !== "load") changed.current?.();
      return true;
    } catch (raw) {
      if (alive.current) setError({ op, text: errorText(raw).trim() || "No details were reported." });
      return false;
    } finally {
      busy.current = false;
      if (alive.current) setPending(null);
    }
  }, [apply]);

  useEffect(() => {
    if (enabled) void perform("load", api.getIntelligence);
  }, [enabled, perform]);

  // Live setup progress. A settled setup changes what Core can run, so it is read again.
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let stop: (() => void) | null = null;
    listen<unknown>(MODEL_INSTALL_EVENT, ({ payload }) => {
      if (disposed || !isInstallStatus(payload)) return;
      latest.current = { status: payload, at: ++clock.current };
      heard.current = Date.now();
      setIntelligence((previous) => previous && withInstall(previous, payload));
      if (SETTLED.has(payload.phase)) void reload(true);
    })
      .then((unlisten) => { deaf.current = false; if (disposed) unlisten(); else stop = unlisten; })
      // Without events, the watchdog below reads Core every second while a setup runs.
      .catch(() => { deaf.current = true; });
    return () => { disposed = true; stop?.(); };
  }, [enabled, reload]);

  const preparing = installActive(intelligence?.local.install);
  useEffect(() => {
    if (!enabled || !preparing) return;
    heard.current = Date.now();
    const timer = window.setInterval(() => {
      const quiet = deaf.current ? WATCH_EVERY_MS : WATCHDOG_MS;
      if (!document.hidden && Date.now() - heard.current >= quiet) {
        heard.current = Date.now();
        void reload(true);
      }
    }, WATCH_EVERY_MS);
    return () => window.clearInterval(timer);
  }, [enabled, preparing, reload]);

  const refresh = useCallback(() => { void reload(false); }, [reload]);
  const setProvider = useCallback((provider: Provider, localModel?: string) =>
    perform("provider", () => api.setProvider(provider, localModel)), [perform]);
  const install = useCallback((model?: string) => perform("install", () => api.installLocalModel(model)), [perform]);
  const cancelInstall = useCallback(() => perform("cancel", api.cancelLocalModelInstall), [perform]);
  const remove = useCallback((model: string) => perform("remove", () => api.removeLocalModel(model)), [perform]);
  const repair = useCallback((model: string) => perform("install", async () => {
    await api.removeLocalModel(model);
    return api.installLocalModel(model);
  }), [perform]);

  return { intelligence, pending, error, refresh, setProvider, install, cancelInstall, remove, repair };
}
