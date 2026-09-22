/**
 * Pegoles Flux Glass — raw signal hues for status semantics.
 *
 * Deliberately outside the blue band (see `isBlue` in ./color.ts): blue
 * means Pegoles' presence, so success/warning/danger/waiting use warm or
 * green hues. Like the palette, these are referenced ONLY by semantic
 * tokens, and never used as the only cue (StatusIndicator pairs every
 * status with a distinct shape and a text label).
 */
export const signal = {
  /** Mint green, 154° — success, completed. */
  green: "#4FD69C",
  /** Amber, 38° — warning, attention soon. */
  amber: "#F5B545",
  /** Coral red, 4° — danger, failure. */
  red: "#FF6F66",
  /** Orchid, 290° — waiting on you (approval, input). */
  orchid: "#DB8CEB",
} as const;

export type SignalToken = keyof typeof signal;
