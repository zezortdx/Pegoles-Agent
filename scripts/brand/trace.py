"""Trace the Pegoles mark PNG and fit the parametric vector geometry.

Usage: python3 scripts/brand/trace.py
Writes: assets/brand/geometry.json (fitted parameters + residuals + colors)

Steps
1. Segment the PNG into background / ring / opening / eyes (edge-bounded
   regions, see brandlib.segment_source).
2. Extract sub-pixel contour points along rays (steepest step of R+G+B).
3. Least-squares fit of each parametric shape to its contour points.
4. Sample reference colors from the source (ring vertical ramp, rims,
   eye ramp, glow) for the SVG gradients.
"""

from __future__ import annotations

import numpy as np
from scipy import ndimage as ndi
from scipy.optimize import least_squares

import brandlib as bl


def contour_points(rgb: np.ndarray, masks: bl.SourceMasks) -> dict[str, np.ndarray]:
    s = ndi.spline_filter(ndi.gaussian_filter(rgb.sum(axis=2), 0.6), order=3)
    lab = masks.labels
    ys, xs = np.nonzero(masks.silhouette)
    center = (float(xs.mean()), float(ys.mean()))
    out = {
        "outer": bl.ray_contour(s, lab, center, lambda l: l != bl.BG, True, 3600, 520),
        "inner": bl.ray_contour(s, lab, center, lambda l: l == bl.RING, False, 3600, 520),
    }
    for key, value in (("eye_l", bl.EYE_L), ("eye_r", bl.EYE_R)):
        ey, ex = np.nonzero(lab == value)
        c = (float(ex.mean()), float(ey.mean()))
        out[key] = bl.ray_contour(s, lab, c, lambda l, v=value: l == v, True, 1440, 140)
    # Index space (pixel centre at integer) -> SVG space (pixel i spans [i, i+1)).
    return {k: v + 0.5 for k, v in out.items()}


def fit(builder, keys: list[str], init: dict[str, float], pts: np.ndarray) -> tuple[dict, dict]:
    x0 = np.array([init[k] for k in keys], dtype=float)

    def resid(x: np.ndarray) -> np.ndarray:
        params = dict(zip(keys, x))
        poly = bl.sample_cubics(builder(params), step=0.5)
        return bl.polyline_distance(poly, pts)

    res = least_squares(resid, x0, diff_step=1e-4, loss="soft_l1", f_scale=1.0, max_nfev=4000)
    params = {k: round(float(v), 3) for k, v in zip(keys, res.x)}
    d = resid(np.array([params[k] for k in keys]))
    stats = {
        "points": int(len(pts)),
        "mean_px": round(float(d.mean()), 3),
        "p95_px": round(float(np.percentile(d, 95)), 3),
        "max_px": round(float(d.max()), 3),
    }
    return params, stats


def bbox(pts: np.ndarray) -> tuple[float, float, float, float]:
    return float(pts[:, 0].min()), float(pts[:, 0].max()), float(pts[:, 1].min()), float(pts[:, 1].max())


def hexcolor(c: np.ndarray) -> str:
    r, g, b = (int(round(float(v))) for v in np.clip(c, 0, 255))
    return f"#{r:02X}{g:02X}{b:02X}"


def sample_colors(rgb: np.ndarray, masks: bl.SourceMasks, outer: dict) -> dict:
    """Reference colors (sRGB hex) sampled from the source pixels.

    The ring reads as a lit glass tube: a pale rim just inside the outer
    contour that fades inward, over a body color that darkens from top to
    bottom. We sample both as vertical ramps at fixed depths from the
    outer contour (EDT depth), plus the inner-edge light, eye ramp, and the
    outer glow falloff.
    """
    depth = ndi.distance_transform_edt(masks.silhouette)
    from_opening = ndi.distance_transform_edt(~masks.opening)
    rows = np.arange(rgb.shape[0])[:, None]
    y_top, y_bot = outer["y0"], outer["y1"]

    def ramp(band: tuple[float, float], extra=None, step: int = 45) -> list[dict]:
        stops = []
        for y0 in range(int(y_top), int(y_bot), step):
            sel = masks.ring & (rows >= y0) & (rows < y0 + step)
            sel &= (depth >= band[0]) & (depth < band[1])
            if extra is not None:
                sel &= extra
            if sel.sum() < 20:
                continue
            yc = float(np.nonzero(sel)[0].mean())
            stops.append(
                {
                    "offset": round((yc - y_top) / (y_bot - y_top), 3),
                    "color": hexcolor(np.median(rgb[sel], axis=0)),
                }
            )
        return stops

    away_from_opening = from_opening > 4
    eyes = ndi.binary_erosion(masks.eyes, iterations=2)
    ey = np.nonzero(eyes.any(axis=1))[0]
    eye_ramp = []
    for f in np.linspace(0, 1, 5):
        y = int(round(ey.min() + f * (ey.max() - ey.min())))
        eye_ramp.append({"offset": round(float(f), 3), "color": hexcolor(np.median(rgb[y][eyes[y]], axis=0))})

    outside = ndi.distance_transform_edt(~masks.silhouette)
    glow = []
    for d in (2, 6, 12, 24, 48, 96, 160, 240):
        sel = (outside >= d) & (outside < d + 2)
        glow.append({"distance_px": d, "color": hexcolor(np.median(rgb[sel], axis=0))})
    opening_core = ndi.binary_erosion(masks.labels == bl.OPEN, iterations=30)
    return {
        "ring_rim": ramp((1, 3), away_from_opening),
        "ring_depth_10": ramp((8, 12), away_from_opening),
        "ring_depth_23": ramp((20, 26), away_from_opening),
        "ring_body": ramp((36, 44), away_from_opening),
        "ring_inner_edge": ramp((0, 1e9), (from_opening < 3) & (depth > 6)),
        "eye_vertical_ramp": eye_ramp,
        "glow_outside": glow,
        "opening": hexcolor(np.median(rgb[opening_core], axis=0)),
        "background_corner": hexcolor(np.median(rgb[:40, :40].reshape(-1, 3), axis=0)),
    }


def main() -> None:
    rgb = bl.load_source()
    masks = bl.segment_source(rgb)
    pts = contour_points(rgb, masks)

    ox0, ox1, oy0, oy1 = bbox(pts["outer"])
    outer_init = {
        "cx": (ox0 + ox1) / 2, "hw": (ox1 - ox0) / 2, "y0": oy0, "y1": oy1,
        "tx": 148, "ty": 148, "bx": 135, "by": 135,
        "rt1": 150, "rt2": 150, "rb1": 150, "rb2": 150,
    }
    ix0, ix1, iy0, iy1 = bbox(pts["inner"])
    inner_init = {
        "cx": (ix0 + ix1) / 2, "hw": (ix1 - ix0) / 2, "y0": iy0, "y1": iy1,
        "rxt": 170, "ryt": 141, "rxb": 180, "ryb": 145,
        "kxt": bl.KAPPA, "kyt": bl.KAPPA, "kxb": bl.KAPPA, "kyb": bl.KAPPA,
    }
    outer, outer_stats = fit(bl.outer_shape, bl.OUTER_KEYS, outer_init, pts["outer"])
    inner, inner_stats = fit(bl.inner_shape, bl.INNER_KEYS, inner_init, pts["inner"])
    eyes, eye_stats = [], []
    for key in ("eye_l", "eye_r"):
        x0, x1, y0, y1 = bbox(pts[key])
        init = {"cx": (x0 + x1) / 2, "cy": (y0 + y1) / 2, "w": x1 - x0, "h": y1 - y0}
        params, stats = fit(bl.eye_shape, bl.EYE_KEYS, init, pts[key])
        eyes.append(params)
        eye_stats.append(stats)

    data = {
        "source": {
            "file": "assets/brand/source/pegoles-mark-source.png",
            "size": bl.SOURCE_SIZE,
            "sha256_prefix": "2c1660e1",
        },
        "units": "source pixels; SVG user space (pixel i spans [i, i+1)), y down",
        "segmentation": {
            "method": "Sobel(|R+G+B| smoothed sigma=0.8) > threshold, seeded region labels, edge band split by nearest region",
            "edge_threshold": bl.EDGE_THRESHOLD,
        },
        "outer": outer,
        "inner": inner,
        "eyes": eyes,
        "fit_residuals": {
            "outer": outer_stats,
            "inner": inner_stats,
            "eye_l": eye_stats[0],
            "eye_r": eye_stats[1],
        },
        "colors": sample_colors(rgb, masks, outer),
    }
    bl.write_json(bl.GEOMETRY_JSON, data)
    print(f"wrote {bl.GEOMETRY_JSON.relative_to(bl.REPO)}")
    for k, v in data["fit_residuals"].items():
        print(f"  {k:6s} {v}")


if __name__ == "__main__":
    main()
