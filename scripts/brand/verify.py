"""Verify the shipped SVG against the source PNG (shape IoU + appearance).

Usage: python3 scripts/brand/verify.py [--diff OUT.png] [--no-chrome]
Writes: assets/brand/fidelity.json

Two independent rasterizations of the SHIPPED file (not of the fit
parameters):
1. pure-Python: parse assets/brand/pegoles-mark-glyph.svg path data
   (M/L/C/Z), flatten, exact even-odd scanline fill with 4x4 supersampling;
2. Chromium headless (if found): render the same path data in a real SVG
   engine at the source frame, threshold coverage at 50 %.
Both are compared with the source masks from brandlib.segment_source().
The full-color SVG is also rendered by Chromium over black and compared
with the source pixels (informational appearance metric).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image

import brandlib as bl

GLYPH = bl.BRAND_DIR / "pegoles-mark-glyph.svg"
FULL = bl.BRAND_DIR / "pegoles-mark.svg"
REPORT = bl.BRAND_DIR / "fidelity.json"
TARGET_IOU = 0.97


def find_chrome() -> str | None:
    env = os.environ.get("PEGOLES_CHROME")
    if env and Path(env).exists():
        return env
    cache = Path.home() / "Library/Caches/ms-playwright"
    for pattern in (
        "chromium_headless_shell-*/chrome-headless-shell-*/chrome-headless-shell",
        "chromium-*/chrome-mac*/Chromium.app/Contents/MacOS/Chromium",
    ):
        hits = sorted(cache.glob(pattern))
        if hits:
            return str(hits[-1])
    for cand in (
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        shutil.which("chromium") or "",
        shutil.which("google-chrome") or "",
    ):
        if cand and Path(cand).exists():
            return cand
    return None


def chrome_screenshot(chrome: str, html: str, size: int = bl.SOURCE_SIZE) -> np.ndarray:
    with tempfile.TemporaryDirectory() as tmp:
        page = Path(tmp) / "page.html"
        shot = Path(tmp) / "shot.png"
        page.write_text(html)
        subprocess.run(
            [
                chrome,
                "--headless",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                f"--window-size={size},{size}",
                f"--screenshot={shot}",
                page.as_uri(),
            ],
            check=True,
            capture_output=True,
            timeout=120,
        )
        return np.asarray(Image.open(shot).convert("RGB")).astype(np.float64)


def glyph_paths() -> dict[str, str]:
    text = GLYPH.read_text()
    out = {}
    for pid in ("ring", "eyes"):
        m = re.search(rf'<path id="{pid}"[^>]*\sd="([^"]+)"', text)
        if not m:
            raise RuntimeError(f"path #{pid} not found in {GLYPH}")
        out[pid] = m.group(1)
    return out


def python_masks(d: dict[str, str], origin: bl.Point) -> dict[str, np.ndarray]:
    def polys(path: str) -> list[np.ndarray]:
        return [bl.sample_cubics(sp, step=0.25) + np.array(origin) for sp in bl.parse_path(path)]

    ring = polys(d["ring"])
    eyes = polys(d["eyes"])
    eyes.sort(key=lambda p: p[:, 0].mean())
    return {
        "ring": bl.raster_mask(ring) >= 0.5,
        "silhouette": bl.raster_mask(ring[:1]) >= 0.5,
        "eyes": bl.raster_mask(eyes) >= 0.5,
        "eye_left": bl.raster_mask(eyes[:1]) >= 0.5,
        "eye_right": bl.raster_mask(eyes[1:]) >= 0.5,
    }


def chrome_masks(chrome: str, d: dict[str, str], origin: bl.Point) -> dict[str, np.ndarray]:
    s = bl.SOURCE_SIZE
    html = (
        "<!doctype html><html><body style='margin:0;background:#000'>"
        f"<svg xmlns='http://www.w3.org/2000/svg' width='{s}' height='{s}' viewBox='0 0 {s} {s}' "
        "style='display:block'>"
        f"<g transform='translate({origin[0]} {origin[1]})' shape-rendering='geometricPrecision'>"
        f"<path fill='#ff0000' fill-rule='evenodd' d='{d['ring']}'/>"
        f"<path fill='#00ff00' d='{d['eyes']}'/>"
        "</g></svg></body></html>"
    )
    img = chrome_screenshot(chrome, html)
    ring = img[..., 0] >= 128
    eyes = img[..., 1] >= 128
    xs = np.arange(s)[None, :]
    mid = (xs < s / 2)
    return {"ring": ring, "eyes": eyes, "eye_left": eyes & mid, "eye_right": eyes & ~mid}


def appearance(chrome: str, origin: bl.Point, masks: bl.SourceMasks, src: np.ndarray, diff: Path | None) -> dict:
    s = bl.SOURCE_SIZE
    svg = FULL.read_text()
    left, top = origin[0] - 120, origin[1] - 120
    html = (
        "<!doctype html><html><body style='margin:0;background:#000;overflow:hidden'>"
        f"<div style='position:absolute;left:{left}px;top:{top}px;width:880px;height:880px'>{svg}</div>"
        "</body></html>"
    )
    img = chrome_screenshot(chrome, html)
    from scipy import ndimage as ndi

    region = ndi.binary_dilation(masks.silhouette, iterations=12)
    err = np.abs(img - src)[region]
    mse = float(((img - src)[region] ** 2).mean())
    if diff is not None:
        side = np.concatenate([src, img, np.clip(np.abs(img - src) * 3, 0, 255)], axis=1)
        Image.fromarray(side.astype(np.uint8)).save(diff)
    return {
        "region": "source silhouette dilated by 12 px, rendered over #000",
        "mean_abs_error_8bit": round(float(err.mean()), 2),
        "psnr_db": round(float(10 * np.log10(255**2 / mse)), 2) if mse > 0 else None,
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--diff", type=Path, default=None, help="write source|render|diff PNG")
    ap.add_argument("--no-chrome", action="store_true")
    args = ap.parse_args()

    src = bl.load_source()
    masks = bl.segment_source(src)
    geo = json.loads(bl.GEOMETRY_JSON.read_text())
    origin = bl.svg_origin(geo)
    d = glyph_paths()
    ref = {
        "ring": masks.ring,
        "silhouette": masks.silhouette,
        "eyes": masks.eyes,
        "eye_left": masks.eye_l,
        "eye_right": masks.eye_r,
    }
    py = python_masks(d, origin)
    report: dict = {
        "source": geo["source"],
        "target_iou": TARGET_IOU,
        "reference_masks": geo["segmentation"],
        "fit_residuals_px": geo["fit_residuals"],
        "iou_python_renderer": {k: round(bl.iou(v, ref[k]), 4) for k, v in py.items()},
    }
    chrome = None if args.no_chrome else find_chrome()
    if chrome:
        cm = chrome_masks(chrome, d, origin)
        report["iou_chromium"] = {k: round(bl.iou(v, ref[k]), 4) for k, v in cm.items()}
        report["appearance_chromium"] = appearance(chrome, origin, masks, src, args.diff)
    else:
        report["iou_chromium"] = "not run (no Chromium found; set PEGOLES_CHROME)"
    bl.write_json(REPORT, report)
    print(json.dumps({k: report[k] for k in ("iou_python_renderer", "iou_chromium")}, indent=2))
    if "appearance_chromium" in report:
        print("appearance", report["appearance_chromium"])
    worst = min(v for v in report["iou_python_renderer"].values())
    if worst < TARGET_IOU:
        raise SystemExit(f"FAIL: IoU {worst} < {TARGET_IOU}")


if __name__ == "__main__":
    main()
