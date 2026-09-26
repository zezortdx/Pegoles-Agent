import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isEffectsTier, type EffectsTier } from "@pegoles/ui";
import { humanizeError, type ErrorScope, type HumanError } from "./errors";
import { api, type AgentEvent, type AgentTask, type HostCapabilities, type StatusPayload } from "../lib/tauri";

const POLL_MS = 2500;
const CORE_EVENTS = ["pegoles://event", "pegoles://display-activity"] as const;

export type Busy = Readonly<Record<ErrorScope, boolean>>;
export type Errors = Readonly<Record<ErrorScope, HumanError | null>>;

const IDLE: Busy = { task: false, run: false, computer: false, general: false };
const CLEAR: Errors = { task: null, run: null, computer: null, general: null };

export interface Core {
  /** Running inside the desktop app (not a browser preview). */
  readonly native: boolean;
  readonly connected: boolean;
  readonly status: StatusPayload | null;
  readonly events: AgentEvent[];
  readonly tasks: AgentTask[];
  readonly host: HostCapabilities | null;
  /** Effects tier Core recommends for this machine. */
  readonly recommendedTier: EffectsTier;
  readonly busy: Busy;
  readonly errors: Errors;
  readonly refresh: () => Promise<void>;
  /** Run a Core command; failures become a human error in `scope`. */
  readonly run: <T>(scope: ErrorScope, command: () => Promise<T>) => Promise<T | undefined>;
  readonly dismiss: (scope: ErrorScope) => void;
  /** Report a failure that did not come from `run` (e.g. native geometry). */
  readonly report: (scope: ErrorScope, error: unknown) => void;
}

/**
 * Core snapshots mirrored into React. Refresh is single-flight with one
 * trailing re-run, so an event that lands mid-refresh is never lost. Polls
 * only while the window is visible; host capabilities are read once.
 */
export function useCore(): Core {
  const native = isTauri();
  const [status, setStatus] = useState<StatusPayload | null>(null);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [tasks, setTasks] = useState<AgentTask[]>([]);
  const [host, setHost] = useState<HostCapabilities | null>(null);
  const [connected, setConnected] = useState(false);
  const [recommendedTier, setRecommendedTier] = useState<EffectsTier>("reduced");
  const [busy, setBusy] = useState<Busy>(IDLE);
  const [errors, setErrors] = useState<Errors>(CLEAR);
  const busyRef = useRef<Record<ErrorScope, boolean>>({ ...IDLE });
  const mounted = useRef(false);
  const flight = useRef<Promise<void> | null>(null);
  const again = useRef(false);
  const hostRead = useRef(false);

  const report = useCallback((scope: ErrorScope, raw: unknown) => {
    if (!mounted.current) return;
    const error = humanizeError(raw, scope);
    setErrors((previous) => ({ ...previous, [error.scope]: error }));
  }, []);

  const refresh = useCallback((): Promise<void> => {
    if (!native) return Promise.resolve();
    if (flight.current) { again.current = true; return flight.current; }
    const pending = (async () => {
      try {
        const [nextStatus, nextEvents, nextTasks] = await Promise.all([api.getStatus(), api.listEvents(), api.listTasks()]);
        if (!mounted.current) return;
        setStatus(nextStatus); setEvents(nextEvents); setTasks(nextTasks); setConnected(true);
        if (!hostRead.current) {
          hostRead.current = true;
          void api.getHostCapabilities().then((value) => { if (mounted.current) setHost(value); })
            .catch(() => { hostRead.current = false; });
        }
      } catch {
        if (mounted.current) setConnected(false);
      }
    })().finally(() => {
      flight.current = null;
      if (again.current && mounted.current) { again.current = false; void refresh(); }
    });
    flight.current = pending;
    return pending;
  }, [native]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    let disposed = false;
    const stops: (() => void)[] = [];
    if (native) {
      void api.suggestedEffects()
        .then((result) => { if (!disposed && isEffectsTier(result.tier)) setRecommendedTier(result.tier); })
        .catch(() => undefined);
      for (const name of CORE_EVENTS) {
        void listen(name, () => void refresh())
          .then((stop) => { if (disposed) stop(); else stops.push(stop); })
          .catch((error: unknown) => { if (!disposed) report("general", `Live updates unavailable: ${String(error)}`); });
      }
    }
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, POLL_MS);
    return () => {
      disposed = true;
      mounted.current = false;
      stops.forEach((stop) => stop());
      window.clearInterval(timer);
    };
  }, [native, refresh, report]);

  const run = useCallback(async <T,>(scope: ErrorScope, command: () => Promise<T>): Promise<T | undefined> => {
    if (!native || busyRef.current[scope]) return undefined;
    busyRef.current[scope] = true;
    setBusy((previous) => ({ ...previous, [scope]: true }));
    setErrors((previous) => (previous[scope] ? { ...previous, [scope]: null } : previous));
    try {
      const result = await command();
      // A refresh already in flight started before the command finished:
      // wait for it, then read Core again so callers see the command's effect.
      if (flight.current) await flight.current;
      await refresh();
      return result;
    } catch (error) {
      report(scope, error);
      return undefined;
    } finally {
      busyRef.current[scope] = false;
      if (mounted.current) setBusy((previous) => ({ ...previous, [scope]: false }));
    }
  }, [native, refresh, report]);

  const dismiss = useCallback((scope: ErrorScope) => {
    setErrors((previous) => (previous[scope] ? { ...previous, [scope]: null } : previous));
  }, []);

  return { native, connected, status, events, tasks, host, recommendedTier, busy, errors, refresh, run, dismiss, report };
}
