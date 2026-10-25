"""The app's living mark, cut from the SOURCE PIXELS (build time only).

Usage: python3 scripts/brand/presence-art.py   (after trace.py)
Reads:  assets/brand/source/pegoles-mark-source.png, assets/brand/geometry.json
Writes: apps/desktop/src/presence/art/body-{128,512}.webp   ring, face, glow; no eyes
        apps/desktop/src/presence/art/eye-{left,right}.webp the two eyes, pill box only
Option: --check PATH  also writes source | body side by side (not committed)

The presence animates the eyes (blinks, glances, lids), so the artwork is
split into a body without eyes and one sprite per eye. Nothing is redrawn:
1. Eyes and their halo are removed from the face by row-wise inpainting
   from face pixels of the same row (the face is a vertical gradient, so
   each row is filled from its own neighbours), then lightly smoothed.
2. Inside the traced silhouette the pixels stay opaque (the black face and
   the glass ring look exactly as in the source on any background); outside
   it the glow is un-blended from black (alpha = max(R, G, B), as
   variants.py does) and fades out radially, so it reads as light on dark UI.
3. The crop is the 640-unit artboard of markGeometry.generated.ts plus
   BLEED on every side for the glow, so the app places the eye sprites
   with the same traced geometry that draws its masks.
The eye halo itself is recreated in CSS (a drop-shadow sized from
look.json's eye glow) so it follows the lids; see presence.css.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

import brandlib as bl

OUT_DIR = bl.REPO / "apps/desktop/src/presence/art"
BLEED = 160  # artboard units of glow room on every side (art side = 640 + 2 * BLEED)
BODY_SIZES = (128, 512)
EYE_SIZE = (64, 144)  # ~1:1 with the source eyes (63.8 x 143.3 px)
ERASE_R0 = 44.0  # px from an eye's outline: fully replaced inside this
ERASE_R1 = 64.0  # ... blended back to the source by here
SAMPLE_MIN_D = 66.0  # inpainting samples lie at least this far from both eyes
FACE_INSET = 14  # iterations of erosion: keep the ring's inner lip out of the samples
TAPER_START = 0.6  # far-halo radial taper: starts at this fraction of the half side
TAPER_END = 0.98


def smoothstep(e0: float, e1: float, x: np.ndarray) -> np.ndarray:
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def masks(geo: dict) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    inner = bl.raster_mask([bl.sample_cubics(bl.inner_shape(geo["inner"]))]) > 0.5
    eyes = bl.raster_mask([bl.sample_cubics(bl.eye_shape(e)) for e in geo["eyes"]]) > 0.5
    silhouette = bl.raster_mask([bl.sample_cubics(bl.outer_shape(geo["outer"]))])
    return inner, eyes, silhouette


def erase_eyes(rgb: np.ndarray, inner: np.ndarray, eyes: np.ndarray) -> np.ndarray:
    """Replace the eyes and their halo with the face around them."""
    dist = ndimage.distance_transform_edt(~eyes)
    samples = ndimage.binary_erosion(inner, iterations=FACE_INSET) & (dist > SAMPLE_MIN_D)
    zone = dist < ERASE_R1
    fill = rgb.copy()
    cols = np.arange(rgb.shape[1])
    for y in np.nonzero(zone.any(axis=1))[0]:
        good = np.nonzero(samples[y])[0]
        if good.size < 8:
            raise SystemExit(f"row {y}: not enough face pixels to inpaint the eyes")
        # A running mean along the row keeps single noisy pixels out.
        row = ndimage.uniform_filter1d(rgb[y], size=9, axis=0)
        for c in range(3):
            fill[y, :, c] = np.interp(cols, good, row[good, c])
    fill = ndimage.gaussian_filter(fill, sigma=(3, 3, 0))
    w = (1 - smoothstep(ERASE_R0, ERASE_R1, dist))[..., None] * zone[..., None]
    return rgb * (1 - w) + fill * w


def artboard_crop(rgb: np.ndarray, side_units: float, origin: tuple[float, float]) -> np.ndarray:
    """Resample [origin, origin + side) of source SVG space to a 1 unit = 1 px grid."""
    side = round(side_units)
    img = [Image.fromarray(rgb[..., c].astype(np.float32), "F") for c in range(rgb.shape[2])]
    # SVG space: pixel i spans [i, i + 1); PIL's affine maps output pixel
    # centres (x + 0.5) to input coordinates, also pixel-centre based.
    ox, oy = origin
    data = (1, 0, ox, 0, 1, oy)
    out = [np.asarray(ch.transform((side, side), Image.Transform.AFFINE, data, Image.Resampling.BICUBIC)) for ch in img]
    return np.clip(np.dstack(out), 0, 1).astype(np.float64)


def premultiplied(rgb: np.ndarray, silhouette: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Premultiplied colour and alpha: opaque inside the silhouette (its
    anti-aliased coverage), un-blended glow outside, tapered radially."""
    n = rgb.shape[0]
    half = n / 2
    ys, xs = np.mgrid[0:n, 0:n]
    k = 1 - smoothstep(TAPER_START, TAPER_END, np.hypot(xs + 0.5 - half, ys + 0.5 - half) / half)
    glow = rgb.max(axis=2) * k
    alpha = np.maximum(glow, silhouette)
    # Over black the source is already premultiplied; outside, the taper scales it with the alpha.
    premul = rgb * np.where(silhouette > 0, 1.0, k)[..., None]
    return np.minimum(premul, alpha[..., None]), alpha


def resized(premul: np.ndarray, alpha: np.ndarray, size: tuple[int, int]) -> Image.Image:
    def rs(channel: np.ndarray) -> np.ndarray:
        img = Image.fromarray(channel.astype(np.float32), "F")
        return np.asarray(img.resize(size, Image.Resampling.LANCZOS), dtype=np.float64)

    p = np.dstack([rs(premul[..., i]) for i in range(3)])
    a = np.clip(rs(alpha), 0, 1)
    p = np.clip(p, 0, a[..., None])
    safe = np.where(a > 1e-6, a, 1)[..., None]
    color = np.clip(p / safe, 0, 1)
    rgba = np.dstack([color, a[..., None]])
    return Image.fromarray(np.round(rgba * 255).astype(np.uint8), "RGBA")


def eye_sprite(rgb: np.ndarray, eye: dict) -> Image.Image:
    x0, y0 = eye["cx"] - eye["w"] / 2, eye["cy"] - eye["h"] / 2
    sx, sy = eye["w"] / EYE_SIZE[0], eye["h"] / EYE_SIZE[1]
    img = [Image.fromarray(rgb[..., c].astype(np.float32), "F") for c in range(3)]
    data = (sx, 0, x0, 0, sy, y0)
    out = np.dstack([np.asarray(ch.transform(EYE_SIZE, Image.Transform.AFFINE, data, Image.Resampling.BICUBIC)) for ch in img])
    out = np.clip(out, 0, 1)
    # The pill is opaque; its box corners (outside the pill) are clipped by CSS.
    return Image.fromarray(np.round(out * 255).astype(np.uint8), "RGB").convert("RGBA")


def check_sheet(path: Path, src: np.ndarray, body: np.ndarray, origin: tuple[float, float], side: float) -> None:
    """Source | body without eyes, same crop, for eyeballing the inpainting."""
    a = artboard_crop(src, side, origin)
    b = artboard_crop(body, side, origin)
    sheet = np.concatenate([a, b], axis=1)
    img = Image.fromarray(np.round(sheet * 255).astype(np.uint8), "RGB").resize((1280, 640), Image.Resampling.LANCZOS)
    img.save(path, optimize=True)


def main() -> None:
    geo = json.loads(bl.GEOMETRY_JSON.read_text())
    src = bl.load_source() / 255.0
    inner, eyes, silhouette = masks(geo)
    body = erase_eyes(src, inner, eyes)
    ox, oy = bl.svg_origin(geo)
    side = bl.ARTBOARD + 2 * BLEED
    origin = (ox - BLEED, oy - BLEED)
    layers = artboard_crop(np.dstack([body, silhouette]), side, origin)
    premul, alpha = premultiplied(layers[..., :3], layers[..., 3])
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for size in BODY_SIZES:
        resized(premul, alpha, (size, size)).save(OUT_DIR / f"body-{size}.webp", quality=82, alpha_quality=85, method=6)
    for name, eye in zip(("left", "right"), sorted(geo["eyes"], key=lambda e: e["cx"])):
        eye_sprite(src, eye).save(OUT_DIR / f"eye-{name}.webp", lossless=True, method=6)
    if "--check" in sys.argv:
        check_sheet(Path(sys.argv[sys.argv.index("--check") + 1]), src, body, (ox, oy), bl.ARTBOARD)
    for f in sorted(OUT_DIR.iterdir()):
        print(f"  {f.relative_to(bl.REPO)}  {f.stat().st_size} B")


if __name__ == "__main__":
    main()
