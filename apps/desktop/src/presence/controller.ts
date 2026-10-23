/**
 * Real presence modes select bloub poses; this layer owns animation time only.
 * It never derives progress, task outcomes or speech from model text.
 */
import { BotEngine, type BotFrame } from "./engine/engine";
import { EXPRESSION_BY_ID, type ExpressionId } from "./engine/expressions";
import { PITCH, PITCH_MAX, YAW_MAX } from "./engine/gaze";
import { clamp, createRng } from "./engine/math";
import { BODY_RADIUS } from "./engine/coordinates";
import { POSES, STATE_BY_ID, type StateId } from "./engine/states";
import type { PresenceMode } from "./modes";

interface Script {
  readonly state: StateId;
  readonly expression?: ExpressionId;
  readonly still?: boolean;
}

/** `planning` is reserved by the existing adapter; the avatar never emits it. */
const SCRIPTS: Readonly<Record<PresenceMode, Script>> = {
  offline: { state: "idle", expression: "sad", still: true },
  idle: { state: "idle" },
  attentive: { state: "wide" },
  acknowledging: { state: "idle" },
  thinking: { state: "thinking" },
  planning: { state: "thinking" },
  working: { state: "orbit" },
  "using-computer": { state: "orbit" },
  waiting: { state: "notify" },
  "needs-user": { state: "notify" },
  blocked: { state: "exclaim", still: true },
  done: { state: "idle", expression: "happy" },
  error: { state: "exclaim", still: true },
};

export const MIN_ORBIT_SIZE = 56;

/** Below 56 px, an orbit without readable rings resembles a play button. */
export function engineStateForMode(mode: PresenceMode, size: number): StateId {
  const state = SCRIPTS[mode].state;
  return state === "orbit" && size < MIN_ORBIT_SIZE ? "thinking" : state;
}

function expressionForMode(mode: PresenceMode) {
  return EXPRESSION_BY_ID.get(SCRIPTS[mode].expression ?? "attentive") ?? null;
}

function sampleStaticPose(mode: PresenceMode, state: StateId): BotFrame {
  return new BotEngine(BODY_RADIUS, state, null, expressionForMode(mode)).sample(POSES[state]);
}

/** Safe synchronous first paint while a new Core signal replaces an old sampler. */
export function staticFrameForMode(mode: PresenceMode, size: number): BotFrame {
  return sampleStaticPose(mode, engineStateForMode(mode, size));
}

export type AvatarReaction = "acknowledge" | "nudge";
const REACTIONS = {
  acknowledge: { state: "wink", duration: 1.6 },
  nudge: { state: "wide", duration: 0.24 },
} as const;

let nextSeed = 1;

export class AvatarController {
  private readonly engine: BotEngine;
  private mode: PresenceMode;
  private baseState: StateId;
  private time: number;
  private blockStart: number;
  private reaction: { state: StateId; duration: number; start: number } | null = null;
  private looking = false;

  constructor(mode: PresenceMode, size: number, seed = nextSeed++) {
    this.mode = mode;
    this.baseState = engineStateForMode(mode, size);
    // Different phases keep simultaneous avatars from blinking in unison.
    this.time = createRng(seed * 7919 + 13)() * 600;
    this.blockStart = this.time;
    this.engine = new BotEngine(BODY_RADIUS, this.baseState, null, this.expression());
    this.engine.reset(this.baseState, this.time);
  }

  get engineState(): StateId { return this.engine.state; }
  get still(): boolean { return SCRIPTS[this.mode].still === true; }

  private expression() {
    return expressionForMode(this.mode);
  }

  setMode(mode: PresenceMode, size: number): void {
    const state = engineStateForMode(mode, size);
    if (mode === this.mode && state === this.baseState) return;
    this.mode = mode;
    this.baseState = state;
    this.blockStart = this.time;
    this.reaction = null;
    this.engine.setExpression(this.expression(), this.time);
    // A new Core signal takes priority over an old orbit or one-shot gesture.
    this.engine.reset(state, this.time);
    this.engine.setLook(null, this.time);
    this.looking = false;
  }

  /** Clear transient gestures when motion is disabled. */
  settle(): void {
    this.reaction = null;
    this.blockStart = this.time;
    this.engine.reset(this.baseState, this.time);
    this.engine.setLook(null, this.time);
    this.looking = false;
  }

  react(kind: AvatarReaction): void {
    // User gestures may decorate a resting face, never obscure work or a brake.
    if (!["idle", "attentive", "acknowledging", "done"].includes(this.mode)) return;
    const reaction = REACTIONS[kind];
    this.reaction = { ...reaction, start: this.time };
    this.engine.setState(reaction.state, this.time);
  }

  setLook(target: { readonly x: number; readonly y: number } | null): void {
    if (!target || !STATE_BY_ID.get(this.engine.state)?.baseFace) {
      if (this.looking) this.engine.setLook(null, this.time);
      this.looking = false;
      return;
    }
    if (!Number.isFinite(target.x + target.y)) return;
    this.engine.setLook({
      yaw: clamp(target.x, -1, 1) * YAW_MAX * 1.5,
      pitch: PITCH - clamp(target.y, -1, 1) * PITCH_MAX,
      mix: 1, spin: 0, wander: 0,
    }, this.time);
    this.looking = true;
  }

  sample(dt: number): BotFrame {
    this.time += Number.isFinite(dt) ? Math.max(0, dt) : 0;
    if (this.still) return this.staticFrame();
    if (this.reaction) {
      if (this.time - this.reaction.start >= this.reaction.duration) {
        this.reaction = null;
        this.blockStart = this.time;
        this.engine.setState(this.baseState, this.time);
      }
    } else if (this.baseState === "orbit") {
      const duration = this.engine.state === "orbit" ? 3.4 : 0.9;
      if (this.time - this.blockStart >= duration) {
        this.blockStart = this.time;
        this.engine.setState(this.engine.state === "orbit" ? "idle" : "orbit", this.time);
      }
    }
    return this.engine.sample(this.time);
  }

  /** New sampler: no pending morph, reaction, pointer or advancing clock. */
  staticFrame(): BotFrame {
    return sampleStaticPose(this.mode, this.baseState);
  }
}
