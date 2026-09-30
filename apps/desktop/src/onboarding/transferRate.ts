import { useEffect, useRef, useState } from "react";

/**
 * Download speed and time left from byte counts Core reports, smoothed so
 * the numbers read calmly. Nothing is shown until the estimate is
 * reliable (a few seconds of steady samples); a stall hides it again.
 */
export interface RateSample {
  readonly bytes: number;
  readonly at: number;
}

export interface RateState {
  readonly last: RateSample | null;
  readonly first: RateSample | null;
  /** Exponentially smoothed bytes per second. */
  readonly rate: number;
}

export const EMPTY_RATE: RateState = { last: null, first: null, rate: 0 };
/** Seconds of samples before speed and time left are shown. */
export const RELIABLE_AFTER_MS = 3000;
const SMOOTHING = 0.25;
const STALL_MS = 8000;

export function addSample(state: RateState, sample: RateSample): RateState {
  const { last } = state;
  if (!last || sample.bytes < last.bytes) return { last: sample, first: sample, rate: 0 };
  const dt = (sample.at - last.at) / 1000;
  if (dt <= 0) return state;
  const instant = (sample.bytes - last.bytes) / dt;
  const rate = state.rate === 0 ? instant : state.rate + SMOOTHING * (instant - state.rate);
  return { last: sample, first: state.first ?? sample, rate };
}

export interface RateEstimate {
  readonly bytesPerSecond: number;
  readonly secondsLeft: number | null;
}

export function estimate(state: RateState, total: number, now: number): RateEstimate | null {
  const { first, last, rate } = state;
  if (!first || !last || rate <= 0) return null;
  if (last.at - first.at < RELIABLE_AFTER_MS || now - last.at > STALL_MS) return null;
  const remaining = Math.max(0, total - last.bytes);
  return { bytesPerSecond: rate, secondsLeft: rate > 0 ? Math.ceil(remaining / rate) : null };
}

/** "8.4 MB/s". */
export function formatRate(bytesPerSecond: number): string {
  const mb = bytesPerSecond / 1e6;
  return mb >= 10 ? `${Math.round(mb)} MB/s` : mb >= 0.1 ? `${mb.toFixed(1)} MB/s` : `${Math.max(1, Math.round(bytesPerSecond / 1e3))} KB/s`;
}

/** "About 3 minutes left", "Less than a minute left". */
export function formatTimeLeft(seconds: number): string {
  if (seconds < 60) return "Less than a minute left";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `About ${minutes} minute${minutes === 1 ? "" : "s"} left`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest >= 10 ? `About ${hours} h ${rest} min left` : `About ${hours} hour${hours === 1 ? "" : "s"} left`;
}

/** Live estimate for a transfer; resets whenever `active` turns off. */
export function useTransferRate(doneBytes: number, totalBytes: number, active: boolean): RateEstimate | null {
  const state = useRef<RateState>(EMPTY_RATE);
  const [result, setResult] = useState<RateEstimate | null>(null);
  useEffect(() => {
    if (!active) {
      state.current = EMPTY_RATE;
      setResult(null);
      return;
    }
    const now = Date.now();
    state.current = addSample(state.current, { bytes: doneBytes, at: now });
    setResult(estimate(state.current, totalBytes, now));
  }, [doneBytes, totalBytes, active]);
  // Re-check once a second so a stalled download stops promising a time.
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setResult(estimate(state.current, totalBytes, Date.now())), 1000);
    return () => window.clearInterval(timer);
  }, [active, totalBytes]);
  return result;
}
