import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  createPresenceMachine,
  type PresenceMachine,
  type PresenceState,
  type PresenceTimers,
} from "./presenceMachine.js";
import { derivePresence, presenceEventsForChange, type PresenceSnapshot } from "./presenceSource.js";

/**
 * Production hook: presence driven ONLY by real app state. Pass the facts
 * the app already has (Core connection, task status, ViewportState, input
 * focus); the machine handles transitions, including the timed
 * Success → Idle return. Re-renders only when the presence state changes.
 */
export function usePresence(snapshot: PresenceSnapshot, timers?: PresenceTimers): PresenceState {
  const [machine] = useState<PresenceMachine>(() => createPresenceMachine(derivePresence(snapshot), timers));
  const prev = useRef<PresenceSnapshot>(snapshot);
  const { online, task, viewport, inputFocused } = snapshot;

  useEffect(() => {
    const next: PresenceSnapshot = { online, task, viewport, inputFocused };
    for (const event of presenceEventsForChange(prev.current, next)) machine.send(event);
    prev.current = next;
  }, [machine, online, task, viewport, inputFocused]);

  useEffect(() => () => machine.dispose(), [machine]);

  return useSyncExternalStore(
    machine.subscribe,
    () => machine.state,
    () => machine.state,
  );
}
