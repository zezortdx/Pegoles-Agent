"""Optimized static raster variants of the Pegoles mark (build time only).

Usage: python3 scripts/brand/variants.py
Writes: assets/brand/raster/pegoles-mark-{64,128,256,512}.{png,webp}
        assets/brand/raster/manifest.json

The variants are made from the SOURCE PIXELS (not the vector), so static
contexts show the exact artwork:
1. Un-blend from black: the source is light on an opaque black ground,
   i.e. already premultiplied over black. alpha = max(R, G, B) / 255 and
   color = RGB / alpha recovers a transparent mark that composites back
   to the source exactly over black (and as light over dark UI).
2. Square crop centred on the mark with room for its glow; the faint far
   halo is tapered to zero in the outer 15 % of the crop so no hard edge
   shows on non-black backgrounds.
3. Resize in premultiplied space (Lanczos) — no dark/bright fringes.
The 1254 px source never ships in the app bundle.
"""

from __future__ import annotations

import json

import numpy as np
from PIL import Image

import brandlib as bl

OUT_DIR = bl.BRAND_DIR / "raster"
SIZES = (64, 128, 256, 512)
CROP_SCALE = 1.34  # crop side = mark width x this (glow room)
TAPER_START = 0.85  # fraction of the half-side where the halo taper begins


def smoothstep(e0: float, e1: float, x: np.ndarray) -> np.ndarray:
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def premultiplied_crop(geo: dict) -> tuple[np.ndarray, np.ndarray]:
    rgb = bl.load_source() / 255.0
    alpha = rgb.max(axis=2)
    o = geo["outer"]
    # Geometry is in SVG space (pixel i spans [i, i+1)); convert to indices.
    cx, cy = o["cx"] - 0.5, (o["y0"] + o["y1"]) / 2 - 0.5
    half = round(o["hw"] * CROP_SCALE)
    x0, y0 = int(round(cx)) - half, int(round(cy)) - half
    side = 2 * half
    ys, xs = np.mgrid[0:side, 0:side]
    t = np.maximum(np.abs(xs + 0.5 - half), np.abs(ys + 0.5 - half)) / half
    taper = 1 - smoothstep(TAPER_START, 1.0, t)
    premul = rgb[y0 : y0 + side, x0 : x0 + side] * taper[..., None]
    a = alpha[y0 : y0 + side, x0 : x0 + side] * taper
    return premul, a


def to_rgba(premul: np.ndarray, alpha: np.ndarray) -> Image.Image:
    safe = np.where(alpha > 1e-6, alpha, 1)[..., None]
    color = np.clip(premul / safe, 0, 1)
    rgba = np.dstack([color, alpha[..., None]])
    return Image.fromarray(np.round(rgba * 255).astype(np.uint8), "RGBA")


def resize_premultiplied(premul: np.ndarray, alpha: np.ndarray, size: int) -> tuple[np.ndarray, np.ndarray]:
    def rs(channel: np.ndarray) -> np.ndarray:
        img = Image.fromarray(channel.astype(np.float32), "F")
        return np.asarray(img.resize((size, size), Image.Resampling.LANCZOS), dtype=np.float64)

    p = np.dstack([rs(premul[..., i]) for i in range(3)])
    a = np.clip(rs(alpha), 0, 1)
    return np.clip(p, 0, a[..., None]), a


def main() -> None:
    geo = json.loads(bl.GEOMETRY_JSON.read_text())
    premul, alpha = premultiplied_crop(geo)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    manifest = {
        "source": geo["source"],
        "method": "unblend-from-black alpha, centred square crop, far-halo taper, premultiplied Lanczos",
        "crop_side_source_px": int(premul.shape[0]),
        "files": [],
    }
    for size in SIZES:
        p, a = resize_premultiplied(premul, alpha, size)
        img = to_rgba(p, a)
        png = OUT_DIR / f"pegoles-mark-{size}.png"
        webp = OUT_DIR / f"pegoles-mark-{size}.webp"
        img.save(png, optimize=True)
        img.save(webp, lossless=size <= 128, quality=90, method=6)
        for f in (png, webp):
            manifest["files"].append({"file": f"raster/{f.name}", "px": size, "bytes": f.stat().st_size})
    bl.write_json(OUT_DIR / "manifest.json", manifest)
    for entry in manifest["files"]:
        print(f"  {entry['file']:32s} {entry['bytes']:>7d} B")


if __name__ == "__main__":
    main()
