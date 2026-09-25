import { MARK_ARTBOARD, MARK_GEOMETRY } from "@pegoles/ui";

/**
 * Signed distance fields of the mark's outer silhouette and face opening,
 * built at runtime from the same fitted paths the SVG draws, so the 3D body
 * can never drift from the brand geometry.
 *
 * Units are mark space: the artboard maps to [−1, 1], y up. The grid covers
 * [−SDF_EXTENT, SDF_EXTENT] so the thought field has room around the body.
 * Row 0 is the top of the grid (y = +SDF_EXTENT).
 */
export const SDF_EXTENT = 1.8;
export const SDF_SIZE = 512;

const HALF = MARK_ARTBOARD / 2;
const INF = 1e20;

let cache: Float32Array | null = null;

/** RG pairs per texel: (outer, inner) signed distance, negative inside. Cached. */
export function markSdf(): Float32Array {
  if (cache) return cache;
  const outer = coverage(MARK_GEOMETRY.paths.outer);
  const inner = coverage(MARK_GEOMETRY.paths.inner);
  const a = smooth(signedField(outer));
  const b = smooth(signedField(inner));
  const data = new Float32Array(SDF_SIZE * SDF_SIZE * 2);
  for (let i = 0; i < SDF_SIZE * SDF_SIZE; i += 1) {
    data[i * 2] = a[i];
    data[i * 2 + 1] = b[i];
  }
  cache = data;
  return data;
}

function coverage(path: string): Float32Array {
  const canvas = document.createElement("canvas");
  canvas.width = SDF_SIZE;
  canvas.height = SDF_SIZE;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) throw new Error("2D canvas unavailable for the presence field");
  const span = 2 * SDF_EXTENT * HALF;
  const origin = HALF - SDF_EXTENT * HALF;
  const k = SDF_SIZE / span;
  ctx.setTransform(k, 0, 0, k, -origin * k, -origin * k);
  ctx.fillStyle = "#fff";
  ctx.fill(new Path2D(path));
  const pixels = ctx.getImageData(0, 0, SDF_SIZE, SDF_SIZE).data;
  const out = new Float32Array(SDF_SIZE * SDF_SIZE);
  for (let i = 0; i < out.length; i += 1) out[i] = pixels[i * 4 + 3] / 255;
  return out;
}

/** Exact Euclidean distance transform (Felzenszwalb & Huttenlocher), squared. */
function edt(feature: (i: number) => boolean): Float64Array {
  const n = SDF_SIZE;
  const grid = new Float64Array(n * n);
  for (let i = 0; i < grid.length; i += 1) grid[i] = feature(i) ? 0 : INF;
  const f = new Float64Array(n);
  const d = new Float64Array(n);
  const v = new Int32Array(n);
  const z = new Float64Array(n + 1);
  for (let x = 0; x < n; x += 1) {
    for (let y = 0; y < n; y += 1) f[y] = grid[y * n + x];
    line(f, d, v, z, n);
    for (let y = 0; y < n; y += 1) grid[y * n + x] = d[y];
  }
  for (let y = 0; y < n; y += 1) {
    for (let x = 0; x < n; x += 1) f[x] = grid[y * n + x];
    line(f, d, v, z, n);
    for (let x = 0; x < n; x += 1) grid[y * n + x] = d[x];
  }
  return grid;
}

function line(f: Float64Array, d: Float64Array, v: Int32Array, z: Float64Array, n: number): void {
  let k = 0;
  v[0] = 0;
  z[0] = -INF;
  z[1] = INF;
  for (let q = 1; q < n; q += 1) {
    let s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
    while (s <= z[k]) {
      k -= 1;
      s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
    }
    k += 1;
    v[k] = q;
    z[k] = s;
    z[k + 1] = INF;
  }
  k = 0;
  for (let q = 0; q < n; q += 1) {
    while (z[k + 1] < q) k += 1;
    d[q] = (q - v[k]) * (q - v[k]) + f[v[k]];
  }
}

function signedField(cover: Float32Array): Float32Array {
  const toOutside = edt((i) => cover[i] < 0.5);
  const toInside = edt((i) => cover[i] >= 0.5);
  const unit = (2 * SDF_EXTENT) / SDF_SIZE;
  const out = new Float32Array(cover.length);
  for (let i = 0; i < cover.length; i += 1) {
    const c = cover[i];
    let texels: number;
    if (c > 0 && c < 1) texels = 0.5 - c; // anti-aliased edge: sub-texel position
    else if (c >= 0.5) texels = -(Math.sqrt(toOutside[i]) - 0.5);
    else texels = Math.sqrt(toInside[i]) - 0.5;
    out[i] = texels * unit;
  }
  return out;
}

/**
 * An exact transform of a raster is only exact to the texel: its gradient
 * steps between texels, which shows as faint facets on the bevel. Two
 * separable box passes (≈ gaussian, σ ≈ 1.2 texels) keep it a valid
 * distance field while making normals continuous.
 */
function smooth(field: Float32Array): Float32Array {
  const n = SDF_SIZE;
  let src: Float32Array = field;
  let dst: Float32Array = new Float32Array(field.length);
  for (let pass = 0; pass < 4; pass += 1) {
    const horizontal = pass % 2 === 0;
    for (let y = 0; y < n; y += 1) {
      for (let x = 0; x < n; x += 1) {
        let sum = 0;
        for (let k = -1; k <= 1; k += 1) {
          const xx = horizontal ? Math.min(n - 1, Math.max(0, x + k)) : x;
          const yy = horizontal ? y : Math.min(n - 1, Math.max(0, y + k));
          sum += src[yy * n + xx];
        }
        dst[y * n + x] = sum / 3;
      }
    }
    [src, dst] = [dst, src];
  }
  return src;
}
