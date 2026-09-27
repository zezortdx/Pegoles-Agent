#!/usr/bin/env python3
"""Generate desktop application icon assets from the Pegoles brand mark.

Source of truth: assets/brand/source/pegoles-mark-source.png (1254x1254,
opaque black background, white/silver glass mark centered). This script never
redesigns, recolors, or restyles the mark -- it only resizes the existing
artwork and, for very small renders, tightens the soft outer glow so the
ring and eyes stay legible (a standard "generate small icons from a
crisper base" technique, not a redesign).

Outputs:
  - apps/desktop/src-tauri/icons/{32x32,128x128,128x128@2x}.png, icon.icns,
    icon.ico  (consumed by the Tauri bundler; referenced from
    tauri.conf.json `bundle.icon`)
  - assets/brand/icons/{same files} + iconset/ + README.md + preview.png
    (provenance copies + documentation + a visual contact sheet)

Requirements: Python 3, Pillow (PIL), numpy. `iconutil` (macOS) is used to
build the .icns from a generated .iconset directory.

Usage:
    python3 scripts/brand/make-icons.py
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_PNG = REPO_ROOT / "assets/brand/source/pegoles-mark-source.png"
GLYPH_SVG = REPO_ROOT / "assets/brand/pegoles-mark-glyph.svg"
TAURI_ICONS_DIR = REPO_ROOT / "apps/desktop/src-tauri/icons"
TAURI_INSTALLER_DIR = REPO_ROOT / "apps/desktop/src-tauri/windows"
BRAND_ICONS_DIR = REPO_ROOT / "assets/brand/icons"

sys.path.insert(0, str(Path(__file__).resolve().parent))
import brandlib as bl  # noqa: E402  (sibling module; needs the path above)

# Mark width / frame width. The source square frames the mark at ~50 %
# (the rest is its glow); Windows/Linux tiles crop tighter so the mark
# reads at taskbar sizes, and the tiny glyph fills most of its tile.
TILE_MARK_FRACTION = 0.64
MAC_MARK_FRACTION = 0.62
GLYPH_MARK_FRACTION = 0.86
# Silver ramp for the flat small-size glyph, sampled from the source ring
# (bright top rim -> darker lower body) and the eyes.
GLYPH_RING_TOP = (246, 247, 248)
GLYPH_RING_BOTTOM = (176, 178, 180)
GLYPH_EYES = (255, 255, 255)

# --- macOS Apple app-icon grid (reference values "at 1024") -----------------
MAC_CANVAS_1024 = 1024
MAC_MARGIN_1024 = 100   # ~100px transparent margin on a 1024 canvas
MAC_BODY_1024 = 824     # squircle body ~824x824
MAC_RADIUS_1024 = 185   # continuous-corner-ish radius, ~185px at 1024

# macOS iconset: (pixel size, filename, use crisp base?)
MAC_ICONSET_ENTRIES = [
    (16, "icon_16x16.png", True),
    (32, "icon_16x16@2x.png", True),
    (32, "icon_32x32.png", True),
    (64, "icon_32x32@2x.png", False),
    (128, "icon_128x128.png", False),
    (256, "icon_128x128@2x.png", False),
    (256, "icon_256x256.png", False),
    (512, "icon_256x256@2x.png", False),
    (512, "icon_512x512.png", False),
    (1024, "icon_512x512@2x.png", False),
]

# Windows .ico frames: (pixel size, use crisp base?)
ICO_SIZES = [(16, True), (24, True), (32, True), (48, False), (64, False), (128, False), (256, False)]

# Linux/other PNGs kept in the Tauri bundle.icon array.
LINUX_PNGS = [
    ("32x32.png", 32, True),
    ("128x128.png", 128, False),
    ("128x128@2x.png", 256, False),
]


def load_source() -> Image.Image:
    if not SOURCE_PNG.exists():
        sys.exit(f"source mark not found: {SOURCE_PNG}")
    im = Image.open(SOURCE_PNG).convert("RGB")
    if im.size[0] != im.size[1]:
        sys.exit(f"expected a square source image, got {im.size}")
    return im


def crisp_base(source: Image.Image) -> Image.Image:
    """Tighten the very soft outer glow so tiny renders stay legible.

    This is a levels/gamma adjustment applied to the *existing* pixels --
    it does not move, recolor, or redraw any part of the mark. It exists
    because naively box-filtering a wide soft glow down to 16-32px turns
    the ring into a blurry smear; pulling in the faint outer glow and
    lifting the mid-tones before downscaling keeps the ring + eyes crisp.
    """
    arr = np.asarray(source, dtype=np.float32) / 255.0
    cutoff = 0.035
    arr = np.clip((arr - cutoff) / (1.0 - cutoff), 0.0, 1.0)
    arr = arr**0.82
    out = np.clip(arr * 255.0 + 0.5, 0, 255).astype(np.uint8)
    return Image.fromarray(out, mode="RGB")


def glyph_polygons() -> tuple[list[np.ndarray], list[np.ndarray]]:
    """Ring and eye outlines of the shipped flat glyph (640-unit space)."""
    import re

    svg = GLYPH_SVG.read_text(encoding="utf-8")
    paths = dict(re.findall(r'<path id="(ring|eyes)"[^>]*? d="([^"]+)"', svg))
    ring = [bl.sample_cubics(sub, step=0.25) for sub in bl.parse_path(paths["ring"])]
    eyes = [bl.sample_cubics(sub, step=0.25) for sub in bl.parse_path(paths["eyes"])]
    return ring, eyes


def render_glyph(size: int, mark_fraction: float = GLYPH_MARK_FRACTION) -> Image.Image:
    """Small-size variant: the same traced geometry, flat silver on black.

    The soft glow cannot survive 16-32 px, so tiny frames draw the ring
    and eyes as solid shapes (exact outlines from the vector trace, 8x8
    supersampled coverage). Same geometry, same proportions; only the
    lighting is dropped.
    """
    ring, eyes = glyph_polygons()
    pts = np.vstack(ring)
    x0, x1 = pts[:, 0].min(), pts[:, 0].max()
    y0, y1 = pts[:, 1].min(), pts[:, 1].max()
    scale = size * mark_fraction / max(x1 - x0, y1 - y0)
    ox = (size - (x1 - x0) * scale) / 2 - x0 * scale
    oy = (size - (y1 - y0) * scale) / 2 - y0 * scale

    def place(polys: list[np.ndarray]) -> list[np.ndarray]:
        return [np.column_stack([q[:, 0] * scale + ox, q[:, 1] * scale + oy]) for q in polys]

    ring_cov = bl.raster_mask(place(ring), size=size, ss=8)[..., None]
    eye_cov = bl.raster_mask(place(eyes), size=size, ss=8)[..., None]
    t = np.clip((np.arange(size) + 0.5 - (y0 * scale + oy)) / ((y1 - y0) * scale), 0, 1)
    top, bottom = np.array(GLYPH_RING_TOP, float), np.array(GLYPH_RING_BOTTOM, float)
    ring_rgb = (top + (bottom - top) * t[:, None])[:, None, :]
    rgb = ring_rgb * ring_cov
    rgb = rgb * (1 - eye_cov) + np.array(GLYPH_EYES, float) * eye_cov
    out = np.clip(rgb + 0.5, 0, 255).astype(np.uint8)
    return Image.fromarray(out, mode="RGB").convert("RGBA")


def tight_crop(source: Image.Image, mark_fraction: float = TILE_MARK_FRACTION) -> Image.Image:
    """Square crop of the source centred on the mark, which then spans
    `mark_fraction` of the crop width (glow kept, only empty black cut)."""
    w = source.size[0]
    mark_w = 628.0  # traced outer width in source pixels (geometry.json)
    side = min(w, round(mark_w / mark_fraction))
    left = (w - side) // 2
    return source.crop((left, left, left + side, left + side))


def rounded_tile(content: Image.Image, size: int, margin_frac: float = 0.03, radius_frac: float = 0.22) -> Image.Image:
    """Dark rounded tile (transparent outside) carrying `content`."""
    margin = round(size * margin_frac) if size >= 32 else 0
    body = size - 2 * margin
    radius = max(1, round(body * radius_frac))
    tile = content.convert("RGBA").resize((body, body), Image.LANCZOS)
    tile.putalpha(squircle_mask(body, radius))
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(tile, (margin, margin), tile)
    return canvas


def make_square_frame(source: Image.Image, crisp_source: Image.Image, size: int, crisp: bool) -> Image.Image:
    """Windows/Linux frame: rounded dark tile. Tiny sizes (crisp) draw the
    flat small-size glyph; larger ones carry the tightly cropped source."""
    if crisp:
        return rounded_tile(render_glyph(size), size)
    content = tight_crop(crisp_source if size <= 64 else source)
    frame = rounded_tile(content, size)
    if size <= 128:
        frame = frame.filter(ImageFilter.UnsharpMask(radius=0.8, percent=60, threshold=3))
    return frame


def squircle_mask(body_px: int, radius_px: int, supersample: int = 4) -> Image.Image:
    big = body_px * supersample
    big_r = radius_px * supersample
    mask_big = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(mask_big)
    draw.rounded_rectangle([0, 0, big - 1, big - 1], radius=big_r, fill=255)
    return mask_big.resize((body_px, body_px), Image.LANCZOS)


def make_mac_icon(source: Image.Image, crisp_source: Image.Image, canvas_px: int, crisp: bool) -> Image.Image:
    """1024-canvas Apple app-icon grid render, scaled to `canvas_px`."""
    scale = canvas_px / MAC_CANVAS_1024
    margin = round(MAC_MARGIN_1024 * scale)
    body = canvas_px - 2 * margin
    radius = max(1, round(MAC_RADIUS_1024 * scale))

    if crisp:
        content = render_glyph(body).convert("RGB")
    else:
        content = tight_crop(source, MAC_MARK_FRACTION).resize((body, body), Image.LANCZOS)

    mask = squircle_mask(body, radius)
    body_rgba = Image.new("RGBA", (body, body))
    body_rgba.paste(content, (0, 0))
    body_rgba.putalpha(mask)

    canvas = Image.new("RGBA", (canvas_px, canvas_px), (0, 0, 0, 0))
    canvas.paste(body_rgba, (margin, margin), body_rgba)
    return canvas


def build_macos(source: Image.Image, crisp_source: Image.Image, out_dirs: list[Path]) -> Path:
    iconset_dir = BRAND_ICONS_DIR / "iconset" / "AppIcon.iconset"
    if iconset_dir.exists():
        shutil.rmtree(iconset_dir)
    iconset_dir.mkdir(parents=True, exist_ok=True)

    cache: dict[tuple[int, bool], Image.Image] = {}
    for size, filename, crisp in MAC_ICONSET_ENTRIES:
        key = (size, crisp)
        if key not in cache:
            cache[key] = make_mac_icon(source, crisp_source, size, crisp)
        cache[key].save(iconset_dir / filename, format="PNG", optimize=True)

    icns_path = BRAND_ICONS_DIR / "icon.icns"
    subprocess.run(
        ["iconutil", "-c", "icns", "-o", str(icns_path), str(iconset_dir)],
        check=True,
    )

    for out_dir in out_dirs:
        dest = out_dir / "icon.icns"
        if dest.resolve() != icns_path.resolve():
            shutil.copyfile(icns_path, dest)

    # Also keep a flat 1024 preview render (not shipped, just for docs/preview).
    master_1024 = cache[(1024, False)] if (1024, False) in cache else make_mac_icon(source, crisp_source, 1024, False)
    return icns_path, master_1024  # type: ignore[return-value]


def build_ico(source: Image.Image, crisp_source: Image.Image, out_paths: list[Path]) -> dict[int, Image.Image]:
    frames: dict[int, Image.Image] = {}
    for size, crisp in ICO_SIZES:
        frames[size] = make_square_frame(source, crisp_source, size, crisp)

    ordered_sizes = sorted(frames)
    base_size = ordered_sizes[-1]  # must be the largest so Pillow's bound check passes
    base_im = frames[base_size]
    append_images = [frames[s] for s in ordered_sizes if s != base_size]

    for path in out_paths:
        base_im.save(
            path,
            format="ICO",
            sizes=[(s, s) for s in ordered_sizes],
            append_images=append_images,
        )
    return frames


def build_linux_pngs(source: Image.Image, crisp_source: Image.Image, out_dirs: list[Path]) -> dict[str, Image.Image]:
    rendered: dict[str, Image.Image] = {}
    for filename, size, crisp in LINUX_PNGS:
        frame = make_square_frame(source, crisp_source, size, crisp)
        rendered[filename] = frame
        for out_dir in out_dirs:
            frame.save(out_dir / filename, format="PNG", optimize=True)
    return rendered


def build_installer_images(source: Image.Image, out_dirs: list[Path]) -> dict[str, Image.Image]:
    """NSIS wizard art (24-bit BMP, the formats NSIS/MUI2 accept):
    header 150x57 (inner pages) and sidebar 164x314 (welcome/finish).
    Black field, the mark with its own glow; no text (the wizard prints
    the product name itself, localized)."""
    crop = tight_crop(source, 0.5)
    header = Image.new("RGB", (150, 57), (0, 0, 0))
    mark = crop.resize((57, 57), Image.LANCZOS)
    header.paste(mark, (150 - 57 - 4, 0))
    sidebar = Image.new("RGB", (164, 314), (0, 0, 0))
    mark = crop.resize((150, 150), Image.LANCZOS)
    sidebar.paste(mark, (7, 64))
    images = {"nsis-header.bmp": header, "nsis-sidebar.bmp": sidebar}
    for out_dir in out_dirs:
        out_dir.mkdir(parents=True, exist_ok=True)
        for name, im in images.items():
            im.save(out_dir / name, format="BMP")
    return images


def build_small_glyphs(out_dir: Path) -> None:
    """Standalone small-size variant (transparent corners) for docs,
    tray/status use and anything drawn at 16-32 px."""
    for size in (16, 24, 32, 48):
        rounded_tile(render_glyph(size), size).save(out_dir / f"pegoles-glyph-{size}.png", format="PNG", optimize=True)


def _load_font(size: int):
    for candidate in (
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ):
        try:
            return ImageFont.truetype(candidate, size)
        except OSError:
            continue
    return ImageFont.load_default()


def build_preview(
    mac_frames: dict[int, Image.Image],
    ico_frames: dict[int, Image.Image],
    linux_frames: dict[str, Image.Image],
) -> None:
    """A contact sheet showing every generated size on light + dark backgrounds.

    Every tile is displayed at the same physical cell size, upscaled with
    nearest-neighbor (never a smoothing resample) so the sheet reveals the
    *actual* rendered pixels -- muddiness or aliasing at small sizes stays
    visible instead of being hidden by a second layer of smoothing.
    """
    LIGHT = (244, 247, 252)
    DARK = (2, 4, 10)  # Pegoles "Void" token
    TEXT_LIGHT = (20, 24, 32)
    TEXT_DARK = (139, 150, 168)  # Pegoles "Muted" token

    rows = [
        ("macOS (.icns squircle)", [(s, mac_frames[s]) for s in sorted(mac_frames)]),
        ("Windows (.ico)", [(s, ico_frames[s]) for s in sorted(ico_frames)]),
        (
            "Linux / other (PNG)",
            [
                (32, linux_frames["32x32.png"]),
                (128, linux_frames["128x128.png"]),
                (256, linux_frames["128x128@2x.png"]),
            ],
        ),
    ]

    cell = 176
    pad = 24
    label_h = 26
    header_h = 34
    row_h = header_h + cell + label_h + pad
    max_cols = max(len(items) for _, items in rows)
    half_w = pad * 2 + max_cols * (cell + pad)
    half_h = pad + len(rows) * row_h
    canvas_w = half_w
    canvas_h = half_h * 2

    font_header = _load_font(20)
    font_label = _load_font(16)

    sheet = Image.new("RGB", (canvas_w, canvas_h), LIGHT)
    draw = ImageDraw.Draw(sheet)
    draw.rectangle([0, half_h, canvas_w, canvas_h], fill=DARK)

    for half_index, (bg_color, text_color) in enumerate([(LIGHT, TEXT_LIGHT), (DARK, TEXT_DARK)]):
        y0 = half_index * half_h
        y = y0 + pad
        for title, items in rows:
            draw.text((pad, y), title, fill=text_color, font=font_header)
            y += header_h
            x = pad
            for size, frame in items:
                rgba = frame.convert("RGBA")
                available = cell - 16
                if size <= available:
                    # Integer nearest-neighbor upscale: reveals the actual
                    # shipped pixels (no smoothing hides muddiness/aliasing).
                    factor = max(1, available // size)
                    thumb = rgba.resize((size * factor, size * factor), Image.NEAREST)
                else:
                    # Large, already-detailed sizes: a smooth downscale is
                    # a fair representative preview, not a QA concern.
                    thumb = rgba.resize((available, available), Image.LANCZOS)
                tile_bg = Image.new("RGBA", (cell, cell), (*bg_color, 255))
                off = ((cell - thumb.width) // 2, (cell - thumb.height) // 2)
                tile_bg.alpha_composite(thumb, off)
                sheet.paste(tile_bg.convert("RGB"), (x, y))
                draw.text((x, y + cell + 4), f"{size}px", fill=text_color, font=font_label)
                x += cell + pad
            y += cell + label_h + pad

    sheet.save(BRAND_ICONS_DIR / "preview.png", format="PNG", optimize=True)


README_TEMPLATE = """# Pegoles app icons and installer art

Generated by `scripts/brand/make-icons.py` from the single source of truth:
`assets/brand/source/pegoles-mark-source.png` (1254x1254, the official
mark: a white/silver glass ring with two eyes on black, soft glow). The
script never redesigns or recolors the mark. It only frames and resizes
it, and below 48 px it draws the small-size variant described below.

```
python3 scripts/brand/make-icons.py
```

Outputs go to `apps/desktop/src-tauri/icons/` (Tauri `bundle.icon`),
`apps/desktop/src-tauri/windows/` (NSIS art) and `assets/brand/icons/`
(provenance copies, `iconset/`, `installer/`, small glyphs, this README,
`preview.png`).

## macOS: `icon.icns`

Apple app-icon grid: 1024 canvas, ~824 px rounded body, ~100 px
transparent margin. The body carries the source cropped so the mark spans
62 % of it (the rest is its own glow). `iconutil` builds the `.icns` from
`iconset/AppIcon.iconset/` (16 to 512@2x). A macOS 26 layered `.icon`
needs Xcode's Icon Composer and is not produced here.

## Windows: `icon.ico`

Frames 16, 24, 32, 48, 64, 128, 256 px: a dark rounded tile (transparent
corners) carrying the source cropped so the mark spans 64 % of the tile.
Desktop apps on Windows get no system corner mask, so the tile shape is
part of the icon. The same `.ico` is the installer icon, the taskbar and
the title-bar icon.

## Small-size variant (16, 24, 32 px)

The glow cannot survive tiny sizes; a downscale turns the ring into a
smudge. Those frames draw `pegoles-mark-glyph.svg` (the exact traced ring
and eyes) as flat shapes: a silver ramp on the ring, white eyes, black
field, 8x8 supersampled coverage. Same geometry and proportions, no
lighting. Standalone copies: `pegoles-glyph-{16,24,32,48}.png`.

## Installer art (NSIS)

`nsis-header.bmp` (150x57, inner pages) and `nsis-sidebar.bmp` (164x314,
welcome and finish pages): 24-bit BMP, black field with the mark and its
glow, no text (the wizard prints the localized product name).

## Linux / other PNGs

`32x32.png`, `128x128.png`, `128x128@2x.png`: same tiles as the Windows
frames. Referenced by `bundle.icon`.

## `preview.png`

Contact sheet of every generated size on light and dark backgrounds,
upscaled with nearest-neighbor so the shipped pixels stay visible.
"""


def write_readme() -> None:
    (BRAND_ICONS_DIR / "README.md").write_text(README_TEMPLATE, encoding="utf-8")


def main() -> None:
    TAURI_ICONS_DIR.mkdir(parents=True, exist_ok=True)
    BRAND_ICONS_DIR.mkdir(parents=True, exist_ok=True)

    source = load_source()
    crisp_source = crisp_base(source)

    print("Building macOS iconset + icon.icns ...")
    _icns_path, _master_1024 = build_macos(source, crisp_source, [TAURI_ICONS_DIR, BRAND_ICONS_DIR])

    print("Building Windows icon.ico ...")
    ico_frames = build_ico(source, crisp_source, [TAURI_ICONS_DIR / "icon.ico", BRAND_ICONS_DIR / "icon.ico"])

    print("Building NSIS installer art + small glyphs ...")
    build_installer_images(source, [TAURI_INSTALLER_DIR, BRAND_ICONS_DIR / "installer"])
    build_small_glyphs(BRAND_ICONS_DIR)

    print("Building Linux/other PNGs ...")
    linux_frames = build_linux_pngs(source, crisp_source, [TAURI_ICONS_DIR, BRAND_ICONS_DIR])

    # Reload the actual saved iconset PNGs (16..1024) for the preview so it
    # reflects exactly what shipped, keyed by pixel size.
    iconset_dir = BRAND_ICONS_DIR / "iconset" / "AppIcon.iconset"
    mac_frames: dict[int, Image.Image] = {}
    for size, filename, _crisp in MAC_ICONSET_ENTRIES:
        mac_frames[size] = Image.open(iconset_dir / filename).convert("RGBA")

    print("Building preview.png contact sheet ...")
    build_preview(mac_frames, ico_frames, linux_frames)

    print("Writing README.md ...")
    write_readme()

    print("Done.")
    for p in sorted(TAURI_ICONS_DIR.iterdir()):
        print(f"  {p.relative_to(REPO_ROOT)}  ({p.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
