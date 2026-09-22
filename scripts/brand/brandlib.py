"""Shared helpers for the Pegoles mark vector pipeline.

Build-time only (never shipped, never run by the app). Requires Python 3 +
Pillow + NumPy + SciPy. See assets/brand/README.md for the pipeline.

Geometry model (all units = source-PNG pixels, image coordinates, y down):

* ``outer``  — rounded octagon: 8 vertices symmetric about its own axis,
  every vertex filleted with a circular arc (emitted as one cubic each).
* ``inner``  — rounded rectangle whose corners are single cubics with free
  radii and handle ratios (top/bottom independent) — i.e. elliptical to
  squircle-like corners — symmetric about its own axis.
* ``eyes``   — two stadiums ("pills"): rx = width / 2.

Every shape becomes a closed list of cubic Bézier segments; straight edges
are degenerate cubics. The exact same segments are written to the SVG, so
what is fitted is what ships.
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage as ndi

REPO = Path(__file__).resolve().parents[2]
SOURCE_PNG = REPO / "assets/brand/source/pegoles-mark-source.png"
BRAND_DIR = REPO / "assets/brand"
GEOMETRY_JSON = BRAND_DIR / "geometry.json"
SOURCE_SIZE = 1254

# Cubic approximation constant for a quarter ellipse.
KAPPA = 0.5522847498307936

Point = tuple[float, float]
Cubic = tuple[Point, Point, Point, Point]


# --------------------------------------------------------------------------
# Geometry builders
# --------------------------------------------------------------------------


def _sub(a: Point, b: Point) -> Point:
    return (a[0] - b[0], a[1] - b[1])


def _add(a: Point, b: Point) -> Point:
    return (a[0] + b[0], a[1] + b[1])


def _mul(a: Point, k: float) -> Point:
    return (a[0] * k, a[1] * k)


def _unit(a: Point) -> Point:
    n = math.hypot(a[0], a[1])
    return (a[0] / n, a[1] / n)


def _line(a: Point, b: Point) -> Cubic:
    return (a, _add(a, _mul(_sub(b, a), 1 / 3)), _add(a, _mul(_sub(b, a), 2 / 3)), b)


def rounded_polygon(vertices: list[Point], radii: list[float]) -> list[Cubic]:
    """Closed polygon with circular fillets at every vertex (cubic arcs)."""
    n = len(vertices)
    fillets = []
    for i in range(n):
        prev_v, v, next_v = vertices[i - 1], vertices[i], vertices[(i + 1) % n]
        u = _unit(_sub(v, prev_v))
        w = _unit(_sub(next_v, v))
        cos_phi = max(-1.0, min(1.0, u[0] * w[0] + u[1] * w[1]))
        phi = math.acos(cos_phi)
        r = radii[i]
        t = r * math.tan(phi / 2)
        t1 = _sub(v, _mul(u, t))
        t2 = _add(v, _mul(w, t))
        k = 4 / 3 * math.tan(phi / 4) * r
        fillets.append((t1, _add(t1, _mul(u, k)), _sub(t2, _mul(w, k)), t2))
    segs: list[Cubic] = []
    for i in range(n):
        segs.append(fillets[i])
        segs.append(_line(fillets[i][3], fillets[(i + 1) % n][0]))
    return segs


OUTER_KEYS = ["cx", "hw", "y0", "y1", "tx", "ty", "bx", "by", "rt1", "rt2", "rb1", "rb2"]
INNER_KEYS = ["cx", "hw", "y0", "y1", "rxt", "ryt", "rxb", "ryb", "kxt", "kyt", "kxb", "kyb"]
EYE_KEYS = ["cx", "cy", "w", "h"]


def outer_shape(p: dict[str, float]) -> list[Cubic]:
    cx, hw, y0, y1 = p["cx"], p["hw"], p["y0"], p["y1"]
    tx, ty, bx, by = p["tx"], p["ty"], p["bx"], p["by"]
    verts = [
        (cx - hw + tx, y0),
        (cx + hw - tx, y0),
        (cx + hw, y0 + ty),
        (cx + hw, y1 - by),
        (cx + hw - bx, y1),
        (cx - hw + bx, y1),
        (cx - hw, y1 - by),
        (cx - hw, y0 + ty),
    ]
    radii = [p["rt1"], p["rt1"], p["rt2"], p["rb2"], p["rb1"], p["rb1"], p["rb2"], p["rt2"]]
    return rounded_polygon(verts, radii)


def inner_shape(p: dict[str, float]) -> list[Cubic]:
    """Rounded rect, clockwise from the top edge's left end."""
    cx, hw, y0, y1 = p["cx"], p["hw"], p["y0"], p["y1"]
    rxt, ryt, rxb, ryb = p["rxt"], p["ryt"], p["rxb"], p["ryb"]
    # Corner handle ratios: KAPPA = elliptical quarter; larger = squircle-like.
    kxt, kyt = p.get("kxt", KAPPA), p.get("kyt", KAPPA)
    kxb, kyb = p.get("kxb", KAPPA), p.get("kyb", KAPPA)
    x0, x1 = cx - hw, cx + hw
    segs: list[Cubic] = []
    segs.append(_line((x0 + rxt, y0), (x1 - rxt, y0)))
    segs.append(((x1 - rxt, y0), (x1 - rxt + kxt * rxt, y0), (x1, y0 + ryt - kyt * ryt), (x1, y0 + ryt)))
    segs.append(_line((x1, y0 + ryt), (x1, y1 - ryb)))
    segs.append(((x1, y1 - ryb), (x1, y1 - ryb + kyb * ryb), (x1 - rxb + kxb * rxb, y1), (x1 - rxb, y1)))
    segs.append(_line((x1 - rxb, y1), (x0 + rxb, y1)))
    segs.append(((x0 + rxb, y1), (x0 + rxb - kxb * rxb, y1), (x0, y1 - ryb + kyb * ryb), (x0, y1 - ryb)))
    segs.append(_line((x0, y1 - ryb), (x0, y0 + ryt)))
    segs.append(((x0, y0 + ryt), (x0, y0 + ryt - kyt * ryt), (x0 + rxt - kxt * rxt, y0), (x0 + rxt, y0)))
    return segs


def eye_shape(p: dict[str, float]) -> list[Cubic]:
    r = p["w"] / 2
    return inner_shape(
        {
            "cx": p["cx"],
            "hw": r,
            "y0": p["cy"] - p["h"] / 2,
            "y1": p["cy"] + p["h"] / 2,
            "rxt": r,
            "ryt": r,
            "rxb": r,
            "ryb": r,
        }
    )


# --------------------------------------------------------------------------
# Sampling, distances, rasterization
# --------------------------------------------------------------------------


def sample_cubics(segs: list[Cubic], step: float = 0.5) -> np.ndarray:
    pts = []
    for p0, p1, p2, p3 in segs:
        length = (
            math.dist(p0, p1) + math.dist(p1, p2) + math.dist(p2, p3) + math.dist(p0, p3)
        ) / 2
        n = max(2, int(math.ceil(length / step)))
        t = np.linspace(0, 1, n, endpoint=False)[:, None]
        a, b, c, d = (np.array(q) for q in (p0, p1, p2, p3))
        pts.append(
            ((1 - t) ** 3) * a + 3 * ((1 - t) ** 2) * t * b + 3 * (1 - t) * t * t * c + t**3 * d
        )
    return np.vstack(pts)


def polyline_distance(poly: np.ndarray, pts: np.ndarray) -> np.ndarray:
    """Unsigned distance from pts to a closed polyline (exact per segment)."""
    from scipy.spatial import cKDTree

    tree = cKDTree(poly)
    _, idx = tree.query(pts)
    n = len(poly)
    best = np.full(len(pts), np.inf)
    for off in (-1, 0):
        a = poly[(idx + off) % n]
        b = poly[(idx + off + 1) % n]
        ab = b - a
        denom = np.maximum((ab * ab).sum(1), 1e-12)
        t = np.clip(((pts - a) * ab).sum(1) / denom, 0, 1)
        proj = a + ab * t[:, None]
        best = np.minimum(best, np.hypot(*(pts - proj).T))
    return best


def raster_mask(polys: list[np.ndarray], size: int = SOURCE_SIZE, ss: int = 4) -> np.ndarray:
    """Exact even-odd scanline fill of closed polygons -> coverage in [0,1].

    Coordinates are SVG user units where pixel i spans [i, i+1). Each pixel
    is point-sampled at ss x ss sub-pixel centres (no edge bias).
    """
    x0 = np.concatenate([p[:, 0] for p in polys])
    y0 = np.concatenate([p[:, 1] for p in polys])
    x1 = np.concatenate([np.roll(p[:, 0], -1) for p in polys])
    y1 = np.concatenate([np.roll(p[:, 1], -1) for p in polys])
    n = size * ss
    toggles = np.zeros((n, n + 1), dtype=np.int32)
    ylo, yhi = np.minimum(y0, y1), np.maximum(y0, y1)
    for row in range(n):
        yc = (row + 0.5) / ss
        span = (ylo <= yc) & (yhi > yc)
        if not span.any():
            continue
        t = (yc - y0[span]) / (y1[span] - y0[span])
        xc = x0[span] + t * (x1[span] - x0[span])
        k = np.clip(np.ceil(xc * ss - 0.5), 0, n).astype(np.int64)
        np.add.at(toggles[row], k, 1)
    inside = (np.cumsum(toggles[:, :n], axis=1) % 2).astype(bool)
    return inside.reshape(size, ss, size, ss).mean(axis=(1, 3))


def iou(a: np.ndarray, b: np.ndarray) -> float:
    inter = np.logical_and(a, b).sum()
    union = np.logical_or(a, b).sum()
    return float(inter / union) if union else 1.0


# --------------------------------------------------------------------------
# SVG path emission + parsing (the verifier parses the shipped file)
# --------------------------------------------------------------------------


def _fmt(v: float) -> str:
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s == "-0" else s


def cubics_to_path(segs: list[Cubic], origin: Point = (0.0, 0.0)) -> str:
    ox, oy = origin
    out = [f"M{_fmt(segs[0][0][0] - ox)} {_fmt(segs[0][0][1] - oy)}"]
    for p0, p1, p2, p3 in segs:
        is_line = (
            math.dist(p1, _add(p0, _mul(_sub(p3, p0), 1 / 3))) < 1e-6
            and math.dist(p2, _add(p0, _mul(_sub(p3, p0), 2 / 3))) < 1e-6
        )
        if is_line:
            if math.dist(p0, p3) < 1e-3:
                continue
            out.append(f"L{_fmt(p3[0] - ox)} {_fmt(p3[1] - oy)}")
        else:
            out.append(
                "C"
                + " ".join(
                    f"{_fmt(q[0] - ox)} {_fmt(q[1] - oy)}" for q in (p1, p2, p3)
                )
            )
    out.append("Z")
    return "".join(out)


_TOKEN = re.compile(r"[MLCZmlcz]|-?\d*\.?\d+(?:e-?\d+)?")


def parse_path(d: str) -> list[list[Cubic]]:
    """Parse the absolute M/L/C/Z subset we emit into cubic subpaths."""
    toks = _TOKEN.findall(d)
    subpaths: list[list[Cubic]] = []
    cur: list[Cubic] = []
    pos: Point = (0.0, 0.0)
    start: Point = (0.0, 0.0)
    i = 0
    cmd = ""

    def num() -> float:
        nonlocal i
        v = float(toks[i])
        i += 1
        return v

    while i < len(toks):
        if re.fullmatch(r"[MLCZmlcz]", toks[i]):
            cmd = toks[i]
            i += 1
        if cmd in "mlcz" and cmd != "":
            raise ValueError("relative path commands are not supported by the verifier")
        if cmd == "M":
            if cur:
                subpaths.append(cur)
                cur = []
            pos = (num(), num())
            start = pos
            cmd = "L"
        elif cmd == "L":
            nxt = (num(), num())
            cur.append(_line(pos, nxt))
            pos = nxt
        elif cmd == "C":
            p1 = (num(), num())
            p2 = (num(), num())
            p3 = (num(), num())
            cur.append((pos, p1, p2, p3))
            pos = p3
        elif cmd == "Z":
            if math.dist(pos, start) > 1e-6:
                cur.append(_line(pos, start))
            pos = start
            subpaths.append(cur)
            cur = []
            cmd = ""
        else:
            raise ValueError(f"unexpected token {toks[i]!r}")
    if cur:
        subpaths.append(cur)
    return subpaths


# --------------------------------------------------------------------------
# Source segmentation (edge-bounded regions)
# --------------------------------------------------------------------------

BG, OPEN, EYE_L, EYE_R, RING = 1, 2, 3, 4, 5
EDGE_THRESHOLD = 300.0  # Sobel magnitude of the smoothed R+G+B sum


@dataclass
class SourceMasks:
    labels: np.ndarray  # full-resolution label image (BG/OPEN/EYE_L/EYE_R/RING)

    @property
    def ring(self) -> np.ndarray:
        return self.labels == RING

    @property
    def eye_l(self) -> np.ndarray:
        return self.labels == EYE_L

    @property
    def eye_r(self) -> np.ndarray:
        return self.labels == EYE_R

    @property
    def eyes(self) -> np.ndarray:
        return self.eye_l | self.eye_r

    @property
    def silhouette(self) -> np.ndarray:
        return self.labels != BG

    @property
    def opening(self) -> np.ndarray:
        return (self.labels == OPEN) | self.eyes


def load_source() -> np.ndarray:
    return np.asarray(Image.open(SOURCE_PNG).convert("RGB")).astype(np.float64)


def segment_source(rgb: np.ndarray) -> SourceMasks:
    """Split the PNG into regions bounded by its crisp contour edges.

    The lower ring is shaded darker than its own outer glow, so no global
    intensity threshold can isolate it. Instead we threshold the gradient
    magnitude (the mark's contours are ~1 px sharp steps), label the
    edge-free regions from known seeds, and assign the edge band to the
    nearest region (which splits the band down its middle = the edge).
    """
    s = ndi.gaussian_filter(rgb.sum(axis=2), 0.8)
    g = np.hypot(ndi.sobel(s, 1), ndi.sobel(s, 0))
    edges = g > EDGE_THRESHOLD
    lab, _ = ndi.label(~edges)
    c = SOURCE_SIZE // 2
    seeds = {
        BG: (5, 5),
        OPEN: (c - 27, c),
        EYE_L: (c - 27, 513),
        EYE_R: (c - 27, 741),
        RING: (c - 27, 340),
    }
    known = np.zeros(lab.shape, np.int32)
    for value, (y, x) in seeds.items():
        region = lab[y, x]
        if region == 0:
            raise RuntimeError(f"seed {value} landed on an edge pixel")
        known[lab == region] = value
    _, (iy, ix) = ndi.distance_transform_edt(known == 0, return_indices=True)
    return SourceMasks(labels=known[iy, ix])


def ray_contour(
    s_coeffs: np.ndarray,
    labels: np.ndarray,
    center: Point,
    inside,
    outward_drop: bool,
    n: int,
    rmax: float,
    window: float = 4.0,
) -> np.ndarray:
    """Sub-pixel contour points: steepest intensity step along rays.

    `inside(labels)` marks the region whose boundary we want; the search
    window is centred on the region's last (or first) pixel along the ray.
    """
    pts = []
    for th in np.linspace(0, 2 * np.pi, n, endpoint=False):
        d = np.array([np.cos(th), np.sin(th)])
        rs = np.arange(0, rmax, 0.5)
        xs = center[0] + rs * d[0]
        ys = center[1] + rs * d[1]
        ok = (xs >= 0) & (xs < SOURCE_SIZE - 1) & (ys >= 0) & (ys < SOURCE_SIZE - 1)
        hit = np.nonzero(inside(labels[np.round(ys[ok]).astype(int), np.round(xs[ok]).astype(int)]))[0]
        if len(hit) == 0:
            continue
        r0 = rs[ok][hit[-1]] if outward_drop else rs[ok][hit[0]]
        rr = np.arange(r0 - window, r0 + window, 0.05)
        vals = ndi.map_coordinates(
            s_coeffs, [center[1] + rr * d[1], center[0] + rr * d[0]], order=3, prefilter=False
        )
        dv = np.gradient(vals, rr)
        k = int(np.argmin(dv) if outward_drop else np.argmax(dv))
        off = 0.0
        if 0 < k < len(dv) - 1:
            y0, y1, y2 = dv[k - 1], dv[k], dv[k + 1]
            den = y0 - 2 * y1 + y2
            off = 0.5 * (y0 - y2) / den if den != 0 else 0.0
        r = rr[k] + off * 0.05
        pts.append((center[0] + r * d[0], center[1] + r * d[1]))
    return np.array(pts)


ARTBOARD = 640  # square artboard (units = source px) the mark is centred in


def svg_origin(geo: dict) -> Point:
    """Source-space point that maps to the artboard origin (0, 0).

    The outer silhouette's bounding-box centre lands on the artboard centre.
    """
    o = geo["outer"]
    cx = o["cx"]
    cy = (o["y0"] + o["y1"]) / 2
    return (round(cx - ARTBOARD / 2, 2), round(cy - ARTBOARD / 2, 2))


def mark_bounds(geo: dict) -> dict[str, float]:
    """Outer silhouette bounds in artboard units."""
    o = geo["outer"]
    ox, oy = svg_origin(geo)
    return {
        "x": round(o["cx"] - o["hw"] - ox, 2),
        "y": round(o["y0"] - oy, 2),
        "width": round(2 * o["hw"], 2),
        "height": round(o["y1"] - o["y0"], 2),
    }


def write_json(path: Path, data: dict) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=False) + "\n")
