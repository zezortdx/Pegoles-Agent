/**
 * The WebGL2 presence renderer. Lazily imported by PegolesPresence after
 * first paint and never re-exported, so it stays out of the entry chunk.
 *
 * One canvas and one context for the whole app: the most recently mounted
 * presence claims it and the canvas is moved into that presence's box
 * (moving a canvas keeps its context). Pose lives in springs owned here, so
 * handing the canvas from Home to a task is the same object continuing.
 */
import type { AmbientController, AmbientSnapshot } from "@pegoles/ui";
import { ambientPaused, frameDelay, frameTooSoon, QUALITY_LADDER, QualityGovernor, SVG_RUNG } from "../governor";
import { WORK_MODES, type PresenceMode } from "../modes";
import { disableGlForSession, glCaveatCheck, MAX_CONTEXT_LOSSES } from "../probe";
import { isSettled, stepCritical, type Spring } from "../spring";
import { presenceTargets, type PresenceTargets } from "../targets";
import { markSdf, SDF_SIZE } from "./sdf";
import { FOV_DEG, FRAGMENT_SHADER, PULSE_LIFE, PULSE_SLOTS, VERTEX_SHADER } from "./shaders";
import type { GlClaimInput, GlHandle, PointerPose, PresenceGlModule, Vec2 } from "./types";

/** Canvas box relative to the mark box: room for the thought field. */
const FIELD_EXTENT = 2;
const DEG = Math.PI / 180;
const LIGHT = normalize([-0.45, 0.62, 0.64]);
/** Back light: the translucent shell glows at its thin edges against it. */
const BACK = normalize([0.35, 0.45, -0.82]);
/** Camera distance so the z = 0 plane fills the canvas at FOV_DEG. */
const CAM_DIST = FIELD_EXTENT / Math.tan((FOV_DEG / 2) * DEG);
/** The camera looks down very slightly so the bevel and sides read as 3D. */
const REST_PITCH = -3;
/** Idle sway: amplitudes (deg) on incommensurate periods (s). */
const SWAY_YAW = 2.5;
const SWAY_PITCH = 1.5;
const SWAY_YAW_S = 11;
const SWAY_PITCH_S = 17;
/** Float: mark units (~0.4% of the mark) over its own slow period. */
const FLOAT = 0.008;
const FLOAT_S = 9;
/** DPR ceiling while the computer panel shares the GPU. */
const SHARED_DPR = 1.5;
/** Spring responses (s): eyes are quick, light is calm, the body is weighty. */
const EYE = 0.2;
const GLOW = 0.42;
const BODY = 0.6;
/** One blink, close and open, in seconds. */
const BLINK_S = 0.17;

type Channel =
  | "breathAmp" | "shellSep" | "field" | "filament" | "flow" | "eyeScaleX" | "eyeScaleY" | "lid" | "eyeGlow"
  | "pitch" | "yaw" | "gazeX" | "gazeY" | "rim" | "warn" | "dim" | "halo" | "impulse"
  | "squash" | "squint" | "kick" | "bloom" | "aware" | "hover" | "sway" | "inner" | "scan";
const RESPONSE: Record<Channel, number> = {
  breathAmp: BODY, shellSep: BODY, field: GLOW, filament: GLOW, flow: GLOW, eyeScaleX: EYE, eyeScaleY: EYE, lid: EYE, eyeGlow: GLOW,
  pitch: BODY, yaw: BODY, gazeX: 0.26, gazeY: 0.26, rim: GLOW, warn: GLOW, dim: GLOW, halo: GLOW, impulse: 0.35,
  squash: 0.45, squint: 0.55, kick: 0.44, bloom: 0.6, aware: GLOW, hover: GLOW, sway: 1.4, inner: 0.7, scan: 0.9,
};
/** Channels that only ever rest at 0 and are driven by one-shot humps. */
const HUMPS: ReadonlySet<Channel> = new Set(["impulse", "squash", "squint", "kick", "bloom"]);
const CHANNELS = Object.keys(RESPONSE) as Channel[];

interface Claim {
  readonly input: GlClaimInput;
  size: number;
  field: boolean;
  mode: PresenceMode;
  reducedMotion: boolean;
  look: Vec2 | null;
  pointer: PointerPose | null;
  glance: Vec2 | null;
  aware: boolean;
  hover: boolean;
  gpuShare: boolean;
  released: boolean;
  initialized: boolean;
}

interface Pulse { age: number; strength: number }

function normalize(v: [number, number, number]): [number, number, number] {
  const l = Math.hypot(v[0], v[1], v[2]);
  return [v[0] / l, v[1] / l, v[2] / l];
}

class PresenceRenderer {
  readonly canvas = document.createElement("canvas");
  private gl: WebGL2RenderingContext | null = null;
  private program: WebGLProgram | null = null;
  private uniforms = new Map<string, WebGLUniformLocation | null>();
  private readonly claims: Claim[] = [];
  private active: Claim | null = null;
  private shown = false;
  private losses = 0;
  private dead = false;

  private readonly springs = Object.fromEntries(CHANNELS.map((c) => [c, { position: 0, velocity: 0 }])) as Record<Channel, Spring>;
  private targets: PresenceTargets = presenceTargets("idle");
  private readonly pulses: Pulse[] = Array.from({ length: PULSE_SLOTS }, () => ({ age: -1, strength: 0 }));
  private time = 0;
  private flowPhase = 0;
  private innerPhase = 0;
  private sweepPhase = 0;
  private blinkT = -1;
  private gestureUntil = 0;

  private raf = 0;
  private timer = 0;
  private last = 0;
  private lastDraw = 0;
  private continuous = false;
  private readonly governor = new QualityGovernor();
  private offscreen = false;
  private observer: IntersectionObserver | null = null;
  private ambient: AmbientController | null = null;
  private ambientSnapshot: AmbientSnapshot | null = null;
  private unsubscribeAmbient: (() => void) | null = null;

  constructor() {
    this.canvas.setAttribute("aria-hidden", "true");
    this.canvas.addEventListener("webglcontextlost", this.onLost, false);
    this.canvas.addEventListener("webglcontextrestored", this.onRestored, false);
    document.addEventListener("visibilitychange", this.wake);
    this.init();
    this.snapTo(presenceTargets("idle"));
  }

  claim(input: GlClaimInput): GlHandle | null {
    if (this.dead || !this.gl) return null;
    const claim: Claim = {
      input, size: input.size, field: input.field, mode: "idle", reducedMotion: false,
      look: null, pointer: null, glance: null, aware: false, hover: false, gpuShare: input.gpuShare ?? false, released: false, initialized: false,
    };
    this.claims.push(claim);
    this.activate(claim);
    return {
      setMode: (mode, reducedMotion) => this.update(claim, () => this.setMode(claim, mode, reducedMotion)),
      setLook: (look) => this.update(claim, () => { claim.look = look; }),
      setPointer: (pointer) => this.update(claim, () => { claim.pointer = pointer; }),
      setAware: (aware) => this.update(claim, () => { claim.aware = aware; }),
      setHover: (hover) => this.update(claim, () => { claim.hover = hover; }),
      blink: () => this.update(claim, () => { if (this.blinkT < 0) this.blinkT = 0; }),
      glance: (offset) => this.update(claim, () => { claim.glance = offset; }),
      nudge: () => this.update(claim, () => this.nudge(claim)),
      boop: () => this.update(claim, () => this.boop(claim)),
      pulse: () => this.update(claim, () => this.pulse(1)),
      setSize: (size, field) => this.update(claim, () => { claim.size = size; claim.field = field; }),
      setGpuShare: (share) => this.update(claim, () => { claim.gpuShare = share; }),
      release: () => this.release(claim),
    };
  }

  // ── Claims ──────────────────────────────────────────────────────

  private update(claim: Claim, apply: () => void): void {
    if (claim.released) return;
    apply();
    if (claim === this.active) this.wake();
  }

  private activate(claim: Claim): void {
    if (this.active && this.active !== claim) this.active.input.onActive(false);
    this.active = claim;
    this.shown = false;
    this.targets = presenceTargets(claim.mode, { reducedMotion: claim.reducedMotion });
    claim.input.container.appendChild(this.canvas);
    this.observe(claim.input.container);
    this.bindAmbient(claim.input.ambient);
    this.wake();
  }

  private release(claim: Claim): void {
    if (claim.released) return;
    claim.released = true;
    const index = this.claims.indexOf(claim);
    if (index >= 0) this.claims.splice(index, 1);
    if (this.active !== claim) return;
    this.active = null;
    this.stop();
    this.canvas.remove();
    this.observer?.disconnect();
    const next = this.claims[this.claims.length - 1];
    if (next) this.activate(next);
  }

  private setMode(claim: Claim, mode: PresenceMode, reducedMotion: boolean): void {
    // The first mode a presence mounts with is where it already is, not an event.
    const entering = claim.initialized && mode !== claim.mode;
    claim.initialized = true;
    claim.mode = mode;
    claim.reducedMotion = reducedMotion;
    if (claim !== this.active) return;
    this.targets = presenceTargets(mode, { reducedMotion });
    if (!entering) return;
    // Done always ends in light: a brief soft bloom of the rim.
    if (mode === "done") this.hump("bloom", 1);
    if (reducedMotion) return;
    // One-shot gestures answer something that just happened.
    if (mode === "acknowledging") {
      this.springs.pitch.velocity += 42;
      this.springs.impulse.velocity += 0.6;
      this.pulse(0.9);
      this.gestureUntil = this.time + 0.9;
    } else if (mode === "done") {
      this.springs.impulse.velocity += 0.75;
      this.pulse(0.7);
      this.gestureUntil = this.time + 1.1;
    } else if (mode === "error") {
      this.springs.yaw.velocity -= 36;
      this.gestureUntil = this.time + 0.6;
    }
  }

  /** Kick a resting channel into one smooth hump (critically damped: no overshoot). */
  private hump(channel: Channel, peak: number): void {
    const omega = (2 * Math.PI) / RESPONSE[channel];
    this.springs[channel].velocity += omega * Math.E * peak;
  }

  private nudge(claim: Claim): void {
    if (claim !== this.active) return;
    this.hump("kick", 1);
    if (!claim.reducedMotion) {
      this.springs.pitch.velocity += (claim.look?.y ?? 1) * 30;
      this.springs.yaw.velocity += (claim.look?.x ?? 0) * 30;
    }
    this.pulse(0.35);
  }

  private boop(claim: Claim): void {
    if (claim !== this.active) return;
    if (claim.reducedMotion) { this.hump("bloom", 0.6); return; }
    this.hump("squash", 0.8);
    this.hump("squint", 0.6);
    this.pulse(0.8);
  }

  private pulse(strength: number): void {
    let slot = this.pulses[0];
    for (const p of this.pulses) {
      if (p.age < 0 || p.age > PULSE_LIFE) { slot = p; break; }
      if (p.age > slot.age) slot = p;
    }
    slot.age = 0;
    slot.strength = strength;
  }

  // ── Scheduling ──────────────────────────────────────────────────

  private readonly wake = (): void => {
    if (!this.active || this.dead) return;
    if (this.timer) { window.clearTimeout(this.timer); this.timer = 0; }
    if (!this.raf) this.raf = window.requestAnimationFrame(this.frame);
  };

  private stop(): void {
    if (this.raf) window.cancelAnimationFrame(this.raf);
    if (this.timer) window.clearTimeout(this.timer);
    this.raf = 0;
    this.timer = 0;
    this.continuous = false;
  }

  private paused(): boolean {
    const mode = this.active?.mode ?? "idle";
    return this.offscreen || ambientPaused(this.ambientSnapshot, WORK_MODES.has(mode));
  }

  private schedule(unsettled: boolean): void {
    const claim = this.active;
    if (!claim) return;
    const snapshot = this.ambientSnapshot;
    const delay = frameDelay({
      visible: document.visibilityState !== "hidden",
      paused: this.paused(),
      unsettled,
      pulsesActive: this.pulses.some((p) => p.age >= 0 && p.age <= PULSE_LIFE),
      fpsCap: this.targets.fpsCap,
      reducedMotion: claim.reducedMotion,
      focused: snapshot ? snapshot.focused : true,
    });
    this.continuous = delay === 0;
    if (delay === null) return;
    if (delay === 0) this.raf = window.requestAnimationFrame(this.frame);
    else this.timer = window.setTimeout(() => { this.timer = 0; this.raf = window.requestAnimationFrame(this.frame); }, delay);
  }

  private readonly frame = (now: number): void => {
    this.raf = 0;
    const claim = this.active;
    if (!claim || !this.gl) return;
    // Never faster than 60 fps, whatever the display refresh (WKWebView on 120 Hz panels).
    if (frameTooSoon(now, this.lastDraw)) { this.raf = window.requestAnimationFrame(this.frame); return; }
    this.lastDraw = now;
    try {
      const elapsed = this.last ? now - this.last : 16.7;
      if (this.continuous && claim.input.quality === "auto" && this.governor.sample(elapsed, now)) {
        if (this.governor.rung >= SVG_RUNG) { this.fail(); return; }
      }
      this.last = now;
      const paused = this.paused();
      if (paused) this.snapTo(this.targets);
      const unsettled = paused ? false : this.step(Math.min(elapsed / 1000, 0.1), claim);
      this.draw(claim);
      if (!this.shown) {
        this.shown = true;
        claim.input.onActive(true);
      }
      this.schedule(unsettled);
    } catch {
      this.fail();
    }
  };

  // ── Animation ───────────────────────────────────────────────────

  private snapTo(targets: PresenceTargets): void {
    for (const channel of CHANNELS) {
      const spring = this.springs[channel];
      spring.position = this.targetOf(channel, targets, this.active);
      spring.velocity = 0;
    }
  }

  private targetOf(channel: Channel, t: PresenceTargets, claim: Claim | null): number {
    if (HUMPS.has(channel)) return 0;
    const look = claim?.look;
    const pointer = claim?.pointer;
    const glance = claim?.glance;
    const gaze = t.gaze;
    switch (channel) {
      case "pitch": return t.pitch + ((look?.y ?? 0) * 5 + (pointer?.pitch ?? 0)) * gaze;
      case "yaw": return t.yaw + ((look?.x ?? 0) * 6 + (pointer?.yaw ?? 0)) * gaze;
      // Eye offsets: fractions of the mark size → mark units (the artboard spans 2), y up.
      case "gazeX": return 2 * ((look?.x ?? 0) * 0.03 + (pointer?.eyeX ?? 0) + (glance?.x ?? 0)) * gaze;
      case "gazeY": return -2 * ((look?.y ?? 0) * 0.025 + (pointer?.eyeY ?? 0) + (glance?.y ?? 0)) * gaze;
      case "aware": return claim?.aware ? 1 : 0;
      case "hover": return claim?.hover ? 1 : 0;
      default: return t[channel as keyof PresenceTargets] as number;
    }
  }

  /** Advance springs and phases; returns true while anything still moves. */
  private step(dt: number, claim: Claim): boolean {
    this.time += dt;
    let unsettled = this.time < this.gestureUntil;
    for (const channel of CHANNELS) {
      const spring = this.springs[channel];
      const target = this.targetOf(channel, this.targets, claim);
      stepCritical(spring, target, RESPONSE[channel], dt);
      if (!isSettled(spring, target, channel === "pitch" || channel === "yaw" ? 0.02 : 1e-3)) unsettled = true;
    }
    const flow = this.springs.flow.position;
    this.flowPhase = (this.flowPhase + (dt * flow) / 4.2) % 1;
    this.innerPhase = (this.innerPhase + dt * flow) % 1000;
    this.sweepPhase = (this.sweepPhase + dt * flow * 1.7) % (Math.PI * 2);
    for (const p of this.pulses) if (p.age >= 0) p.age = p.age > PULSE_LIFE ? -1 : p.age + dt;
    if (this.blinkT >= 0) {
      this.blinkT += dt;
      unsettled = true;
      if (this.blinkT > BLINK_S || claim.reducedMotion) this.blinkT = -1;
    }
    return unsettled;
  }

  // ── GL ──────────────────────────────────────────────────────────

  private init(): void {
    const gl = this.canvas.getContext("webgl2", {
      alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false,
      powerPreference: "low-power", failIfMajorPerformanceCaveat: glCaveatCheck(),
    });
    if (!gl) throw new Error("WebGL2 unavailable");
    const program = link(gl);
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG16F, SDF_SIZE, SDF_SIZE, 0, gl.RG, gl.FLOAT, markSdf());
    gl.useProgram(program);
    gl.bindVertexArray(gl.createVertexArray());
    gl.clearColor(0, 0, 0, 0);
    this.gl = gl;
    this.program = program;
    this.uniforms.clear();
    gl.uniform1i(this.loc("uSdf"), 0);
    gl.uniform3fv(this.loc("uLight"), LIGHT);
  }

  private loc(name: string): WebGLUniformLocation | null {
    if (!this.uniforms.has(name) && this.gl && this.program) this.uniforms.set(name, this.gl.getUniformLocation(this.program, name));
    return this.uniforms.get(name) ?? null;
  }

  private resize(claim: Claim): void {
    const rung = QUALITY_LADDER[Math.min(this.governor.rung, QUALITY_LADDER.length - 1)];
    const cap = claim.gpuShare || claim.mode === "using-computer" ? SHARED_DPR : 2;
    const dpr = Math.min(window.devicePixelRatio || 1, rung.dpr, cap);
    const css = claim.size * FIELD_EXTENT;
    const px = Math.max(2, Math.round(css * dpr));
    if (this.canvas.width !== px || this.canvas.height !== px) {
      this.canvas.width = px;
      this.canvas.height = px;
    }
  }

  private draw(claim: Claim): void {
    const gl = this.gl;
    if (!gl) return;
    this.resize(claim);
    const s = this.springs;
    const w = this.canvas.width;
    gl.viewport(0, 0, w, w);
    const breath = s.breathAmp.position * (0.5 - 0.5 * Math.cos((2 * Math.PI * this.time) / Math.max(this.targets.breathPeriod, 0.5)));
    const blink = this.blinkT >= 0 ? 0.92 * Math.sin(Math.PI * Math.min(this.blinkT / BLINK_S, 1)) : 0;
    const widen = 1 + 0.04 * s.kick.position + 0.03 * s.aware.position;
    const squash = Math.max(0, s.squash.position);
    const rung = QUALITY_LADDER[Math.min(this.governor.rung, QUALITY_LADDER.length - 1)];
    gl.uniform2f(this.loc("uRes"), w, w);
    gl.uniform1f(this.loc("uExtent"), FIELD_EXTENT);
    const sway = Math.max(0, s.sway.position);
    const swayYaw = sway * SWAY_YAW * Math.sin((2 * Math.PI * this.time) / SWAY_YAW_S);
    const swayPitch = sway * SWAY_PITCH * Math.sin((2 * Math.PI * this.time) / SWAY_PITCH_S + 1.3);
    const pitch = (REST_PITCH + s.pitch.position + swayPitch) * DEG;
    const yaw = (s.yaw.position + swayYaw) * DEG;
    gl.uniform1f(this.loc("uCamDist"), CAM_DIST);
    gl.uniformMatrix3fv(this.loc("uInvRot"), false, inverseRotation(pitch, yaw));
    gl.uniform3fv(this.loc("uLight"), rotateIntoObject(LIGHT, pitch, yaw));
    gl.uniform3fv(this.loc("uBack"), rotateIntoObject(BACK, pitch, yaw));
    // The studio drifts a little with the sway, so the highlight travels across the ring.
    gl.uniform2f(this.loc("uEnvDrift"), swayYaw * DEG * 1.6, swayPitch * DEG * 1.6);
    gl.uniform1f(this.loc("uLift"), sway * FLOAT * Math.sin((2 * Math.PI * this.time) / FLOAT_S));
    gl.uniform1f(this.loc("uScale"), 1 + breath + s.impulse.position);
    const ph = this.innerPhase;
    gl.uniform1f(this.loc("uInner"), clamp01(s.inner.position));
    gl.uniform2f(this.loc("uInnerPos"), 0.34 * Math.sin(ph * 0.9), 0.15 * Math.sin(ph * 1.37 + 1.1) - 0.03);
    gl.uniform1f(this.loc("uScan"), clamp01(s.scan.position));
    gl.uniform1f(this.loc("uScanX"), -1.15 + 2.3 * ((ph * 0.3) % 1));
    gl.uniform1f(this.loc("uShellSep"), clamp01(s.shellSep.position));
    gl.uniform1f(this.loc("uField"), claim.field ? clamp01(s.field.position) : 0);
    gl.uniform1f(this.loc("uFilament"), clamp01(s.filament.position));
    gl.uniform1f(this.loc("uFlowPhase"), this.flowPhase);
    gl.uniform1f(this.loc("uSweepPhase"), this.sweepPhase);
    gl.uniform4f(this.loc("uEye"), s.eyeScaleX.position * widen, s.eyeScaleY.position * widen, s.gazeX.position, s.gazeY.position);
    gl.uniform1f(this.loc("uLid"), Math.min(0.96, Math.max(s.lid.position, blink)));
    gl.uniform1f(this.loc("uSquint"), Math.min(0.9, Math.max(0, s.squint.position)));
    gl.uniform2f(this.loc("uSquash"), 1 + 0.05 * squash, 1 - 0.08 * squash);
    gl.uniform1f(this.loc("uBloom"), clamp01(s.bloom.position));
    gl.uniform1f(this.loc("uEyeGlow"), Math.max(0, s.eyeGlow.position));
    gl.uniform1f(this.loc("uRim"), s.rim.position);
    gl.uniform1f(this.loc("uWarn"), clamp01(s.warn.position));
    gl.uniform1f(this.loc("uDim"), clamp01(s.dim.position));
    gl.uniform1f(this.loc("uHalo"), Math.min(1.4, Math.max(0, s.halo.position + 0.1 * s.aware.position + 0.14 * s.hover.position)));
    gl.uniform1f(this.loc("uDetail"), rung.detail);
    const pulses = new Float32Array(PULSE_SLOTS * 2);
    this.pulses.forEach((p, i) => { pulses[i * 2] = p.age; pulses[i * 2 + 1] = p.strength; });
    gl.uniform2fv(this.loc("uPulses[0]"), pulses);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  // ── Environment ─────────────────────────────────────────────────

  private observe(element: HTMLElement): void {
    this.observer?.disconnect();
    this.offscreen = false;
    if (typeof IntersectionObserver === "undefined") return;
    this.observer = new IntersectionObserver((records) => {
      const record = records[records.length - 1];
      if (!record) return;
      this.offscreen = !record.isIntersecting;
      this.wake();
    }, { rootMargin: "64px" });
    this.observer.observe(element);
  }

  private bindAmbient(ambient: AmbientController | null): void {
    if (ambient === this.ambient) return;
    this.unsubscribeAmbient?.();
    this.ambient = ambient;
    this.ambientSnapshot = ambient?.snapshot() ?? null;
    this.unsubscribeAmbient = ambient?.subscribe((snapshot) => {
      this.ambientSnapshot = snapshot;
      this.wake();
    }) ?? null;
  }

  private readonly onLost = (event: Event): void => {
    event.preventDefault();
    this.stop();
    this.losses += 1;
    this.gl = null;
    this.active?.input.onActive(false);
    this.shown = false;
    if (this.losses >= MAX_CONTEXT_LOSSES) this.fail();
  };

  private readonly onRestored = (): void => {
    if (this.dead) return;
    try {
      this.init();
      this.wake();
    } catch {
      this.fail();
    }
  };

  /** WebGL is done for this session: every presence falls back to SVG. */
  private fail(): void {
    if (this.dead) return;
    this.dead = true;
    this.stop();
    disableGlForSession();
    this.canvas.remove();
    this.observer?.disconnect();
    this.unsubscribeAmbient?.();
    document.removeEventListener("visibilitychange", this.wake);
    const claims = [...this.claims];
    this.claims.length = 0;
    this.active = null;
    for (const claim of claims) claim.input.onFail();
  }
}

function clamp01(v: number): number {
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

/** Column-major world → object rotation for object pose R = Ry(yaw) · Rx(pitch). */
function inverseRotation(pitch: number, yaw: number): Float32Array {
  const cp = Math.cos(pitch), sp = Math.sin(pitch), cy = Math.cos(yaw), sy = Math.sin(yaw);
  // R (row-major) = [[cy, sy*sp, sy*cp], [0, cp, -sp], [-sy, cy*sp, cy*cp]]; its transpose is the inverse.
  // Column-major storage of Rᵀ is R's rows in order.
  return new Float32Array([cy, sy * sp, sy * cp, 0, cp, -sp, -sy, cy * sp, cy * cp]);
}

function rotateIntoObject(v: readonly number[], pitch: number, yaw: number): Float32Array {
  const m = inverseRotation(pitch, yaw);
  // m is column-major Rᵀ: result = Rᵀ · v.
  return new Float32Array([
    m[0] * v[0] + m[3] * v[1] + m[6] * v[2],
    m[1] * v[0] + m[4] * v[1] + m[7] * v[2],
    m[2] * v[0] + m[5] * v[1] + m[8] * v[2],
  ]);
}

function link(gl: WebGL2RenderingContext): WebGLProgram {
  const compile = (type: number, source: string) => {
    const shader = gl.createShader(type);
    if (!shader) throw new Error("shader allocation failed");
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS) && !gl.isContextLost()) {
      throw new Error(gl.getShaderInfoLog(shader) ?? "shader compile failed");
    }
    return shader;
  };
  const program = gl.createProgram();
  if (!program) throw new Error("program allocation failed");
  gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX_SHADER));
  gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT_SHADER));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS) && !gl.isContextLost()) {
    throw new Error(gl.getProgramInfoLog(program) ?? "program link failed");
  }
  return program;
}

let renderer: PresenceRenderer | null = null;

export const claimPresenceGl: PresenceGlModule["claimPresenceGl"] = (input) => {
  if (!renderer) {
    try {
      renderer = new PresenceRenderer();
    } catch {
      disableGlForSession();
      return null;
    }
  }
  return renderer.claim(input);
};
