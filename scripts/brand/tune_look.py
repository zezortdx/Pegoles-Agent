"""Fit the full-color SVG's lighting parameters to the source PNG.

Usage: python3 scripts/brand/tune_look.py [--evals N]   (needs Chromium)
Writes: assets/brand/look.json  (then re-run build.py + verify.py)

Only the lighting recipe is tuned (glow/light/rim opacities, widths, blur
radii, body gain). Geometry and sampled colors are never touched. Each
evaluation renders the real SVG in Chromium over black and measures the
weighted MSE against the source pixels (mark + near glow at weight 1,
far glow at weight 0.3).
"""

from __future__ import annotations

import argparse
import json

import numpy as np
from scipy import ndimage as ndi
from scipy.optimize import minimize

import brandlib as bl
import build
import verify

BOUNDS: dict[str, tuple[float, float]] = {
    "glow_far_opacity": (0.0, 1.0),
    "glow_far_std": (20.0, 200.0),
    "glow_near_opacity": (0.0, 1.0),
    "glow_near_std": (3.0, 60.0),
    "glow_tight_opacity": (0.0, 1.0),
    "glow_tight_std": (1.0, 12.0),
    "light_width": (6.0, 70.0),
    "light_std": (3.0, 30.0),
    "light_opacity": (0.0, 1.0),
    "rim_width": (1.0, 12.0),
    "rim_std": (0.3, 4.0),
    "rim_opacity": (0.0, 1.0),
    "edge_width": (1.0, 12.0),
    "edge_opacity": (0.0, 1.0),
    "shade_opacity": (0.0, 1.0),
    "shade_std": (4.0, 90.0),
    "eye_glow_opacity": (0.0, 1.0),
    "eye_glow_std": (2.0, 20.0),
    "body_gain": (0.5, 1.5),
}
KEYS = list(BOUNDS)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--evals", type=int, default=600)
    args = ap.parse_args()
    chrome = verify.find_chrome()
    if not chrome:
        raise SystemExit("Chromium not found (set PEGOLES_CHROME)")
    src = bl.load_source()
    masks = bl.segment_source(src)
    near = ndi.binary_dilation(masks.silhouette, iterations=12)
    far = ndi.binary_dilation(masks.silhouette, iterations=160) & ~near
    geo = json.loads(bl.GEOMETRY_JSON.read_text())
    origin = bl.svg_origin(geo)
    p = build.paths(geo)
    pal = build.palette(geo)
    lo = np.array([BOUNDS[k][0] for k in KEYS])
    hi = np.array([BOUNDS[k][1] for k in KEYS])

    def render(look: dict[str, float]) -> np.ndarray:
        svg = build.full_svg(geo, p, pal, look)
        left, top = origin[0] - 120, origin[1] - 120
        html = (
            "<!doctype html><html><body style='margin:0;background:#000;overflow:hidden'>"
            f"<div style='position:absolute;left:{left}px;top:{top}px;width:880px;height:880px'>{svg}</div>"
            "</body></html>"
        )
        return verify.chrome_screenshot(chrome, html)

    def loss(z: np.ndarray) -> float:
        look = dict(zip(KEYS, lo + np.clip(z, 0, 1) * (hi - lo)))
        img = render(look)
        d2 = ((img - src) ** 2).sum(axis=2)
        return float(d2[near].mean() + 0.3 * d2[far].mean())

    start = build.load_look()
    z0 = np.array([(start[k] - BOUNDS[k][0]) / (BOUNDS[k][1] - BOUNDS[k][0]) for k in KEYS])
    first = loss(z0)
    res = minimize(loss, z0, method="Powell", bounds=[(0, 1)] * len(KEYS),
                   options={"maxfev": args.evals, "xtol": 1e-3, "ftol": 1e-4})
    best = {k: round(float(v), 3) for k, v in zip(KEYS, lo + np.clip(res.x, 0, 1) * (hi - lo))}
    bl.write_json(build.LOOK_JSON, {
        "note": "Lighting recipe fitted by scripts/brand/tune_look.py against the source PNG.",
        "loss_start": round(first, 1),
        "loss_final": round(float(res.fun), 1),
        "evaluations": int(res.nfev),
        "params": best,
    })
    print(f"loss {first:.1f} -> {res.fun:.1f} in {res.nfev} renders")
    print(json.dumps(best, indent=2))


if __name__ == "__main__":
    main()
