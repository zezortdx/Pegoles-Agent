import { MARK_ARTBOARD, MARK_GEOMETRY } from "@pegoles/ui";
import { SDF_EXTENT } from "./sdf";

const HALF = MARK_ARTBOARD / 2;
const f = (n: number) => n.toFixed(5);
const eye = (index: number) => {
  const e = MARK_GEOMETRY.eyes[index];
  return `vec2(${f((e.cx - HALF) / HALF)}, ${f(-(e.cy - HALF) / HALF)})`;
};
const EYE_HALF_W = MARK_GEOMETRY.eyes[0].width / 2 / HALF;
const EYE_HALF_H = MARK_GEOMETRY.eyes[0].height / 2 / HALF;

export const PULSE_SLOTS = 6;
export const PULSE_LIFE = 0.76;
/** Vertical field of view of the presence camera, degrees. */
export const FOV_DEG = 18;

export const VERTEX_SHADER = `#version 300 es
const vec2 P[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
out vec2 vUv;
void main() {
  vec2 p = P[gl_VertexID];
  vUv = p * 0.5 + 0.5;
  gl_Position = vec4(p, 0.0, 1.0);
}`;

/**
 * One full-screen triangle. The body is an extruded, bevelled, pillowed
 * distance field of the mark with the face recessed into it, raymarched
 * through a narrow perspective camera only near the silhouette.
 *
 * Material: frosted, glazed white ceramic (Schlick F0 0.04 over a soft
 * translucent body that lets light through its thin edges) and a dark
 * glass face with the eyes sitting just behind the glass. Lighting is an
 * analytic studio: a sky/floor gradient and two soft rectangular light
 * boxes, sampled from the world-space reflection vector, so highlights
 * travel across the ring as the object moves. ACES-ish tone mapping and
 * interleaved-gradient dither keep the black face free of banding.
 */
export const FRAGMENT_SHADER = `#version 300 es
precision highp float;
precision highp sampler2D;
in vec2 vUv;
out vec4 outColor;

uniform sampler2D uSdf;
uniform vec2 uRes;
uniform float uExtent;
uniform float uCamDist;
uniform mat3 uInvRot;
uniform vec3 uLight;
uniform vec3 uBack;
uniform vec2 uEnvDrift;
uniform float uScale;
uniform float uLift;
uniform float uShellSep;
uniform float uField;
uniform float uFilament;
uniform float uFlowPhase;
uniform float uSweepPhase;
uniform vec4 uEye;
uniform float uLid;
uniform float uSquint;
uniform vec2 uSquash;
uniform float uBloom;
uniform float uEyeGlow;
uniform float uRim;
uniform float uWarn;
uniform float uDim;
uniform float uHalo;
uniform float uDetail;
uniform float uInner;
uniform vec2 uInnerPos;
uniform float uScan;
uniform float uScanX;
uniform vec2 uPulses[${PULSE_SLOTS}];

const float E = ${f(SDF_EXTENT)};
const float H = 0.16;
const float PILLOW = 0.03;
const float R = 0.14;
const float D = 0.075;
const float RL = 0.03;
const float EYE_DEPTH = 0.02;
const vec3 WARN = vec3(0.961, 0.710, 0.271);
const vec3 COLD = vec3(0.80, 0.88, 1.0);
const vec3 WHITE = vec3(0.961, 0.961, 0.953);
const vec3 AZURE = vec3(0.42, 0.66, 1.0);

vec2 sdf2(vec2 p) {
  return texture(uSdf, vec2(p.x + E, E - p.y) / (2.0 * E)).rg;
}

float extrude(float d, float z, float h, float r) {
  vec2 w = vec2(d + r, abs(z) - h + r);
  return min(max(w.x, w.y), 0.0) + length(max(w, 0.0)) - r;
}

float smax(float a, float b, float k) {
  float h = clamp(0.5 - 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) + k * h * (1.0 - h);
}

float map(vec3 p) {
  vec2 d = sdf2(p.xy);
  // Pillowed cap: the ring swells slightly between its two edges, so its
  // normals turn gently across the width and catch moving light.
  float pillow = PILLOW * smoothstep(0.0, 0.13, min(-d.x, d.y));
  float body = extrude(d.x, p.z, H + pillow, R);
  float recess = extrude(d.y, p.z - H, D, RL);
  return smax(body, -recess, 0.02);
}

vec3 normalAt(vec3 p) {
  const vec2 k = vec2(1.0, -1.0);
  const float e = 0.008;
  return normalize(
    k.xyy * map(p + k.xyy * e) +
    k.yyx * map(p + k.yyx * e) +
    k.yxy * map(p + k.yxy * e) +
    k.xxx * map(p + k.xxx * e));
}

float roundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float eyes(vec2 p) {
  vec2 gaze = uEye.zw;
  vec2 hs = vec2(${f(EYE_HALF_W)} * uEye.x, ${f(EYE_HALF_H)} * uEye.y);
  float r = min(hs.x, hs.y);
  vec2 a = p - ${eye(0)} - gaze;
  vec2 b = p - ${eye(1)} - gaze;
  // Upper lid: a gently arched edge sliding down, lower at the sides.
  float lid = hs.y - 2.0 * hs.y * uLid;
  float arch = 0.45 * hs.x * min(uLid * 4.0, 1.0);
  float la = a.y - lid + arch * (a.x * a.x) / (hs.x * hs.x);
  float lb = b.y - lid + arch * (b.x * b.x) / (hs.x * hs.x);
  // Happy squint: a lower lid rising, higher in the middle.
  float low = -hs.y + 2.0 * hs.y * uSquint;
  float up = 0.55 * hs.x * min(uSquint * 3.0, 1.0);
  float sa = low + up * (1.0 - (a.x * a.x) / (hs.x * hs.x)) - a.y;
  float sb = low + up * (1.0 - (b.x * b.x) / (hs.x * hs.x)) - b.y;
  float ea = max(max(roundBox(a, hs, r), la), uSquint > 0.001 ? sa : -1.0);
  float eb = max(max(roundBox(b, hs, r), lb), uSquint > 0.001 ? sb : -1.0);
  return min(ea, eb);
}

// ── Studio ─────────────────────────────────────────────────────────

float softbox(vec3 r, vec3 c, vec2 halfSize, float soft) {
  float k = dot(r, c);
  if (k <= 0.05) return 0.0;
  vec3 u = normalize(cross(vec3(0.0, 1.0, 0.0), c));
  vec3 v = cross(c, u);
  vec2 q = abs(vec2(dot(r, u), dot(r, v)) / k) - halfSize;
  float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
  return 1.0 - smoothstep(-soft, soft, d);
}

/** World-space studio: dark floor, soft horizon, a key box and a cool strip. */
vec3 studio(vec3 r, float soft) {
  vec3 c = mix(vec3(0.010), vec3(0.045, 0.047, 0.052), smoothstep(-0.45, 0.02, r.y));
  c = mix(c, vec3(0.15, 0.155, 0.17), smoothstep(0.02, 0.95, r.y));
  // A wide soft box above and left of the camera, a thin cool strip to the right.
  vec3 key = normalize(vec3(-0.3 + uEnvDrift.x, 0.46 + uEnvDrift.y, 0.84));
  vec3 strip = normalize(vec3(0.8 + uEnvDrift.x * 0.5, 0.05, 0.6));
  c += vec3(1.0, 0.985, 0.96) * 9.0 * softbox(r, key, vec2(0.5, 0.17), soft);
  c += vec3(0.84, 0.9, 1.0) * 2.6 * softbox(r, strip, vec2(0.045, 0.55), soft * 0.7);
  return c;
}

vec3 toWorld(vec3 v) { return v * uInvRot; }

// ── Materials ──────────────────────────────────────────────────────

vec3 shadeShell(vec3 p, vec3 n, vec3 v, vec2 dd, float pix) {
  vec3 nW = toWorld(n);
  float nv = clamp(dot(n, v), 0.0, 1.0);
  float fres = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
  // Frosted ceramic body: softly wrapped key light plus sky/floor irradiance.
  float wrap = clamp((dot(n, uLight) + 0.3) / 1.3, 0.0, 1.0);
  vec3 irr = mix(vec3(0.035, 0.037, 0.045), vec3(0.3, 0.31, 0.34), smoothstep(-0.9, 0.9, nW.y));
  vec3 albedo = vec3(0.86, 0.87, 0.885);
  vec3 c = albedo * (irr + vec3(1.0, 0.985, 0.96) * 1.2 * wrap);
  // Soft edge scattering: light passes through where the shell is thin
  // (its outer bevel) and glows most where the back light shines through.
  float thin = exp(-max(-dd.x, 0.0) / 0.055);
  float back = pow(clamp(dot(v, -normalize(uBack + n * 0.6)), 0.0, 1.0), 3.0);
  c += vec3(0.86, 0.92, 1.0) * thin * (0.1 + 0.55 * back);
  // Occlusion where the shell curves down into the face, and the eyes' bounce on that lip.
  float lip = 1.0 - smoothstep(0.0, 0.08, dd.y);
  c *= 1.0 - 0.45 * lip;
  float inward = clamp(-dot(n.xy, normalize(p.xy + 1e-4)), 0.0, 1.0);
  float bounce = exp(-max(eyes(p.xy), 0.0) / 0.2) * lip * inward;
  c += mix(COLD, WARN, uWarn * 0.6) * bounce * 0.2 * uEyeGlow;
  // Glaze: sharp studio reflection over the frosted body.
  vec3 rW = toWorld(reflect(-v, n));
  c = c * (1.0 - fres) + studio(rW, 0.2) * fres * 0.85;
  vec3 rim = mix(COLD, vec3(1.0, 0.96, 0.9), clamp(uRim * 0.5 + 0.5, 0.0, 1.0));
  rim = mix(rim, WARN, uWarn);
  c += rim * pow(1.0 - nv, 3.0) * (0.22 + 0.5 * uWarn);
  c += vec3(1.0) * uBloom * (0.2 + 0.25 * thin + 0.5 * pow(1.0 - nv, 2.0));
  if (uFilament > 0.001) {
    // Thought contours running through the shell, lit by a slow travelling sweep.
    float travel = pow(0.5 + 0.5 * sin(atan(p.y, p.x) * 2.0 - uSweepPhase), 8.0);
    float lines = 0.0;
    for (int k = 1; k <= 2; k++) {
      float level = -0.036 * float(k) - 0.008;
      lines += 1.0 - smoothstep(0.0, pix * 1.4, abs(dd.x - level));
    }
    c = mix(c, c * 0.82 + COLD * 0.45, uFilament * lines * (0.3 + 0.7 * travel));
    c += COLD * uFilament * travel * 0.14 * thin;
  }
  return c * (1.0 - 0.6 * uDim);
}

vec3 shadeFace(vec3 p, vec3 n, vec3 v, vec3 rd, vec2 dd, float pix) {
  // Dark glass, gently domed so it carries one soft reflection band.
  vec3 nd = normalize(n + vec3(p.xy * 0.2, 0.0));
  float nv = clamp(dot(nd, v), 0.0, 1.0);
  float fres = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
  // Behind the glass: smoky depth, a faint lift toward the bottom.
  vec3 c = vec3(0.012, 0.013, 0.017) + vec3(0.012, 0.016, 0.026) * smoothstep(0.2, -0.75, p.y);
  // The eyes sit slightly behind the glass: they shift against the reflection.
  vec3 pe = p + rd * (EYE_DEPTH / max(-rd.z, 0.3));
  float e = eyes(pe.xy);
  vec3 spill = mix(vec3(0.72, 0.82, 1.0), WARN, uWarn * 0.5);
  c += spill * exp(-max(e, 0.0) / 0.035) * 0.16 * uEyeGlow;
  c += spill * exp(-max(e, 0.0) / 0.16) * 0.035 * uEyeGlow;
  // Internal light: wanders while thinking, travels one way while working.
  if (uInner > 0.001) {
    vec2 w = pe.xy - uInnerPos;
    float blob = exp(-dot(w, w * vec2(1.0, 2.2)) / 0.09);
    float band = exp(-pow((pe.x - uScanX) / 0.14, 2.0)) * smoothstep(0.62, 0.1, abs(pe.y + 0.02));
    c += AZURE * uInner * mix(blob, band, uScan) * 0.2;
  }
  float inner = 1.0 - smoothstep(0.0, 0.035, -dd.y);
  c += vec3(0.7, 0.76, 0.85) * inner * 0.03;
  vec3 eyeCol = mix(vec3(1.0), vec3(0.84, 0.9, 1.0), smoothstep(-0.07, 0.0, e));
  eyeCol = mix(eyeCol, vec3(1.0, 0.87, 0.64), uWarn * 0.55);
  float inside = 1.0 - smoothstep(-pix, pix, e);
  c = mix(c, eyeCol * 1.7 * min(uEyeGlow, 1.1), inside);
  // The glass reflection sits on top of everything behind it.
  c += studio(toWorld(reflect(-v, nd)), 0.16) * fres * 0.45;
  return c * (1.0 - 0.5 * uDim);
}

/** Filmic shoulder: linear through the mids, highlights roll off smoothly to white. */
vec3 tonemap(vec3 x) {
  const float K = 0.72;
  vec3 over = K + (1.0 - K) * (1.0 - exp(-(x - K) / (1.0 - K)));
  return mix(x, over, step(K, x));
}

float ring(float d, float r, float width) {
  return 1.0 - smoothstep(0.0, width, abs(d - r));
}

vec3 toObject(vec3 w) {
  vec3 o = uInvRot * w;
  // Squash and stretch pivot on the base, like a soft object pressed down.
  return vec3(o.x / (uScale * uSquash.x), (o.y - uLift + 0.9) / (uScale * uSquash.y) - 0.9, o.z / uScale);
}

void main() {
  vec2 q = (vUv * 2.0 - 1.0) * uExtent;
  float pix = 2.0 * uExtent / uRes.y / uScale;

  // Perspective camera on +z; the ray is carried into object space.
  vec3 roW = vec3(0.0, 0.0, uCamDist);
  vec3 ro = toObject(roW);
  vec3 rd = normalize(toObject(vec3(q, 0.0)) - ro);

  // Only the slab that can hold the body is marched.
  float zb = H + PILLOW + 0.01;
  float t0 = (zb - ro.z) / rd.z;
  float t1 = (-zb - ro.z) / rd.z;
  vec3 pa = ro + rd * t0;
  vec3 pb = ro + rd * t1;
  vec2 fd = sdf2(ro.xy + rd.xy * (-ro.z / rd.z));
  float span = length(pb.xy - pa.xy);
  bool near = min(sdf2(pa.xy).x, sdf2(pb.xy).x) - 0.5 * span < 0.08;

  float t = t0;
  float minD = 1e9;
  float tMin = t0;
  bool hit = false;
  if (near) {
    for (int i = 0; i < 64; i++) {
      float d = map(ro + rd * t);
      if (d < minD) { minD = d; tMin = t; }
      if (d < 0.0006) { hit = true; break; }
      t += d * 0.85;
      if (t > t1) break;
    }
  }
  float cover = hit ? 1.0 : 1.0 - smoothstep(0.0, pix * 1.2, minD);
  vec3 body = vec3(0.0);
  if (cover > 0.0) {
    vec3 p = ro + rd * (hit ? t : tMin);
    vec3 n = normalAt(p);
    vec2 dd = sdf2(p.xy);
    bool face = dd.y < -0.004 && p.z < H - D * 0.5;
    body = face ? shadeFace(p, n, -rd, rd, dd, pix) : shadeShell(p, n, -rd, dd, pix);
    body = tonemap(body);
  }

  // Field: halo, shell layers, thought contours and event ripples.
  float out_ = max(fd.x, 0.0);
  float halo = (exp(-out_ / 0.11) * 0.075 * uHalo + exp(-out_ / 0.045) * 0.3 * uBloom) * step(0.0, fd.x);
  vec3 light = (WHITE + (WARN - WHITE) * uWarn * 0.75) * halo;
  float alpha = halo;
  if (uShellSep > 0.001) {
    float a = ring(fd.x, 0.02 + uShellSep * 0.05, pix * 1.2) * uShellSep * 0.4;
    light += WHITE * a;
    alpha += a;
  }
  if (uField > 0.001) {
    int count = uDetail > 0.5 ? 3 : 1;
    for (int k = 0; k < 3; k++) {
      if (k >= count) break;
      float ph = fract(uFlowPhase + float(k) / 3.0);
      float a = ring(fd.x, 0.03 + ph * 0.42, pix * 1.3) * uField * 0.5 * smoothstep(0.0, 0.15, ph) * (1.0 - ph);
      light += WHITE * a;
      alpha += a;
    }
  }
  for (int i = 0; i < ${PULSE_SLOTS}; i++) {
    vec2 pl = uPulses[i];
    if (pl.x < 0.0 || pl.x > ${f(PULSE_LIFE)}) continue;
    float ph = pl.x / ${f(PULSE_LIFE)};
    float a = ring(fd.x, 0.01 + ph * 0.5, pix * (1.3 + ph * 2.0)) * pl.y * 0.6 * (1.0 - ph) * (1.0 - ph);
    light += WHITE * a;
    alpha += a;
  }
  alpha = min(alpha, 1.0);
  float a = cover + alpha * (1.0 - cover);
  vec3 rgb = body * cover + light * (1.0 - cover);
  // Interleaved gradient noise: one LSB of dither against banding on black.
  float ign = fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
  rgb = clamp(rgb + (ign - 0.5) / 255.0 * a, 0.0, a);
  outColor = vec4(rgb, a);
}`;
