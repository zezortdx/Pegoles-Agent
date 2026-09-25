import type { AmbientController } from "@pegoles/ui";
import type { PresenceMode } from "../modes";
import type { PresenceQuality } from "../quality";

export interface Vec2 {
  readonly x: number;
  readonly y: number;
}

/** Pointer response, already eased by distance. */
export interface PointerPose {
  /** Eye offset as a fraction of the mark size (y down). */
  readonly eyeX: number;
  readonly eyeY: number;
  /** Degrees: nod (+ = top toward the viewer) and turn (+ = toward the right). */
  readonly pitch: number;
  readonly yaw: number;
}

/** What a mounted presence hands the shared WebGL renderer. */
export interface GlClaimInput {
  /** Element the shared canvas is moved into (2× the mark box, centred). */
  readonly container: HTMLElement;
  /** Mark size in CSS px. */
  readonly size: number;
  readonly field: boolean;
  readonly quality: PresenceQuality;
  readonly ambient: AmbientController | null;
  /** Another GPU-heavy surface (the computer panel) is on screen: cap DPR at 1.5. */
  readonly gpuShare?: boolean;
  /** The canvas is showing this presence (true) or moved away / paused into SVG (false). */
  readonly onActive: (active: boolean) => void;
  /** WebGL is gone for the session (lost context, crash, sustained slow frames). */
  readonly onFail: () => void;
}

/** Imperative handle: every call is cheap and never re-renders React. */
export interface GlHandle {
  setMode(mode: PresenceMode, reducedMotion: boolean): void;
  setLook(look: Vec2 | null): void;
  setPointer(pointer: PointerPose | null): void;
  /** The pointer is close: eyes widen slightly, the halo rises. */
  setAware(aware: boolean): void;
  /** Hovering a pressable presence: the halo lifts. */
  setHover(hover: boolean): void;
  /** Idle life. */
  blink(): void;
  glance(offset: Vec2 | null): void;
  /** One keystroke: eyes widen for a moment, a lean toward `look`, a shimmer. */
  nudge(): void;
  /** Pressed: squash and stretch, a happy squint, a ripple. */
  boop(): void;
  pulse(): void;
  setSize(size: number, field: boolean): void;
  setGpuShare(share: boolean): void;
  release(): void;
}

export interface PresenceGlModule {
  claimPresenceGl(input: GlClaimInput): GlHandle | null;
}
