/** One rAF for every avatar; hidden time never advances the engine. */
type Listener = (dt: number) => void;
const MAX_FRAME_DELTA = 0.064;
const listeners = new Set<Listener>();
let cancel: (() => void) | null = null;
let last: number | null = null;

function schedule(callback: FrameRequestCallback): () => void {
  if (typeof requestAnimationFrame === "function") {
    const id = requestAnimationFrame(callback);
    return () => cancelAnimationFrame(id);
  }
  const id = setTimeout(() => callback(performance.now()), 16);
  return () => clearTimeout(id);
}

function tick(time: number): void {
  cancel = schedule(tick);
  const dt = last === null ? 0 : Math.min(Math.max((time - last) / 1000, 0), MAX_FRAME_DELTA);
  last = time;
  for (const listener of [...listeners]) listener(dt);
}

function sync(): void {
  if (listeners.size > 0 && !document.hidden) {
    cancel ??= schedule(tick);
  } else {
    cancel?.();
    cancel = null;
    last = null;
  }
}

export function subscribeAvatarClock(listener: Listener): () => void {
  if (listeners.size === 0) document.addEventListener("visibilitychange", sync);
  listeners.add(listener);
  sync();
  return () => {
    listeners.delete(listener);
    sync();
    if (listeners.size === 0) document.removeEventListener("visibilitychange", sync);
  };
}
