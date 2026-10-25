/**
 * Guest → preview coordinate mapping (pure).
 *
 * Agent actions carry NORMALIZED guest positions (0..1 of the guest
 * framebuffer, see pegoles-protocol `ComputerAction`). The preview draws
 * the framebuffer with `object-fit: contain` inside its box, so the only
 * mapping needed is: where does the frame land inside the box, then scale.
 * Normalized input makes the guest resolution, the guest's own DPI scaling
 * (Windows) and the host's Retina factor irrelevant: all of them cancel
 * out before a coordinate reaches the UI, and CSS pixels do the rest.
 */

export interface Size {
  readonly width: number;
  readonly height: number;
}

export interface Rect extends Size {
  readonly x: number;
  readonly y: number;
}

export interface Point {
  readonly x: number;
  readonly y: number;
}

function positive(n: number): boolean {
  return Number.isFinite(n) && n > 0;
}

/** How the preview draws the frame in its box (CSS `object-fit`). */
export type FrameFit = "contain" | "cover";

/**
 * The rectangle a `content`-sized frame occupies relative to `box`, centred,
 * under `object-fit: contain` (letterboxed or pillarboxed) or `cover`
 * (cropped: the rectangle overflows the box). Without a usable content size
 * the frame fills the box.
 */
export function frameRect(box: Size, content: Size | null, fit: FrameFit = "contain"): Rect {
  const bw = positive(box.width) ? box.width : 0;
  const bh = positive(box.height) ? box.height : 0;
  if (!content || !positive(content.width) || !positive(content.height) || bw === 0 || bh === 0) {
    return { x: 0, y: 0, width: bw, height: bh };
  }
  const pick = fit === "cover" ? Math.max : Math.min;
  const scale = pick(bw / content.width, bh / content.height);
  const width = content.width * scale;
  const height = content.height * scale;
  return { x: (bw - width) / 2, y: (bh - height) / 2, width, height };
}

/** `object-fit: contain` placement (the computer panel). */
export function containRect(box: Size, content: Size | null): Rect {
  return frameRect(box, content, "contain");
}

/** Clamp a normalized coordinate to the frame (hostile or stray values never leave it). */
export function clampUnit(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.min(1, Math.max(0, n));
}

/** Normalized guest point → overlay CSS px, through the frame rectangle. */
export function toOverlay(point: Point, frame: Rect): Point {
  return { x: frame.x + clampUnit(point.x) * frame.width, y: frame.y + clampUnit(point.y) * frame.height };
}

/** Snap to the device pixel grid so a resting pointer stays crisp. */
export function snapToDevice(n: number, devicePixelRatio: number): number {
  const dpr = positive(devicePixelRatio) ? devicePixelRatio : 1;
  return Math.round(n * dpr) / dpr;
}
