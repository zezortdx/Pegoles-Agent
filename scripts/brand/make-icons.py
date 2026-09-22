#!/usr/bin/env python3
"""Generate desktop application icon assets from the Pegoles brand mark.

Source of truth: assets/brand/source/pegoles-mark-source.png (1254x1254,
opaque black background, glowing blue mark centered). This script never
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
TAURI_ICONS_DIR = REPO_ROOT / "apps/desktop/src-tauri/icons"
BRAND_ICONS_DIR = REPO_ROOT / "assets/brand/icons"

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


def make_square_frame(source: Image.Image, crisp_source: Image.Image, size: int, crisp: bool) -> Image.Image:
    """Full-bleed square render at `size`, faithful to the source."""
    base = crisp_source if crisp else source
    frame = base.resize((size, size), Image.LANCZOS)
    if crisp:
        frame = frame.filter(ImageFilter.UnsharpMask(radius=1.2, percent=140, threshold=2))
    elif size <= 128:
        frame = frame.filter(ImageFilter.UnsharpMask(radius=0.8, percent=60, threshold=3))
    return frame.convert("RGBA")


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

    content_src = crisp_source if crisp else source
    content = content_src.resize((body, body), Image.LANCZOS)
    if crisp:
        content = content.filter(ImageFilter.UnsharpMask(radius=1.0, percent=120, threshold=2))

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


README_TEMPLATE = """# Pegoles desktop app icons

Generated by `scripts/brand/make-icons.py` from the single source of truth:
`assets/brand/source/pegoles-mark-source.png` (1254x1254, opaque black
background, glowing electric-blue mark centered). The script never
redesigns, recolors, or restyles the mark -- it only resizes the existing
artwork and, for the smallest renders, tightens the soft outer glow so the
ring and eyes stay readable (see "Small-size legibility" below).

Run it with:

```
python3 scripts/brand/make-icons.py
```

It writes the same outputs to two places:

- `apps/desktop/src-tauri/icons/` -- consumed by the Tauri bundler
  (`tauri.conf.json` -> `bundle.icon`).
- `assets/brand/icons/` -- provenance copies + this README + `preview.png`
  + the intermediate `iconset/AppIcon.iconset/` used to build the `.icns`.

## macOS -- `icon.icns`

Built to Apple's app-icon grid: a 1024x1024 canvas, a rounded-rect
("squircle-ish") body ~824x824 centered (~100px transparent margin per
side), corner radius ~185px at 1024 scale (radius/margin/body all scale
together for the smaller grid sizes). The body is filled by resizing the
*entire* source square (its black background travels with it) into the
824x824 box and clipping with the rounded-rect mask -- so the glow is
never cropped, only the square's already-near-black corners are rounded
off. Outside the squircle is transparent.

`iconutil -c icns` builds `icon.icns` from `iconset/AppIcon.iconset/`,
covering the full standard set: 16, 16@2x, 32, 32@2x, 128, 128@2x, 256,
256@2x, 512, 512@2x.

**Deferred:** macOS 26 "Liquid Glass" layered icons (the multi-layer
`.icon` format edited in Xcode's Icon Composer) are NOT produced here --
Icon Composer is an Xcode GUI tool and this host only has the Command
Line Tools installed (no Xcode). `icon.icns` is what the app bundle uses
today; a layered `.icon` is a follow-up once Xcode is available, not
something faked or approximated by this script.

**Known limitation:** at the 16px iconset entry the squircle body is only
~13x13px (824/1024 scale of a 16px canvas), which is not enough pixels to
render both the ring and the two eyes distinctly -- it reads as a glowing
blue emblem rather than a crisp ring+eyes. This matches Apple's own
16px app icons, which lose fine detail at that size too; 16px macOS icons
are rarely seen at native resolution outside small Finder rows. See
`preview.png` for a direct look.

## Windows -- `icon.ico`

Frames: 16, 24, 32, 48, 64, 128, 256px, all full-bleed square (no
rounding). This matches Windows convention: Explorer, the taskbar, and
Start already apply their own corner treatment/shadow chrome around
pinned tiles and file icons, so a source `.ico` that is itself already
rounded gets double-rounded or shows mismatched corners at small sizes.
Shipping a plain square keeps every consumer's masking correct.

## Small-size legibility

At 16/24/32px, a literal box-filter downscale of the full glow turns the
ring into a blurry smudge. For those sizes only, the script derives the
frame from a "crisper base": the same source pixels run through a small
levels/gamma adjustment that pulls in the faintest outer glow and lifts
the mid-tones, then downscales, then applies a light unsharp mask. This
is standard icon-authoring practice (generating small mip levels from a
tuned base rather than naively shrinking the largest asset) -- it does
not move, recolor, or redraw the ring/eyes. 48px and up use a faithful
direct resize of the source (with a very light unsharp mask through
128px) since detail survives fine at those sizes. The same crisp-base
treatment is applied to the macOS 16/16@2x/32 iconset entries.

## Linux / other -- PNGs

`32x32.png`, `128x128.png`, `128x128@2x.png` (256px), full-bleed square,
same treatment as the Windows frames (crisp base at 32px, faithful
resize at 128/256px). These three PNGs are also what Tauri uses as the
runtime window icon on Linux/X11/Wayland and are referenced directly by
`bundle.icon`.

`Square*Logo*.png` (MSIX/Windows Store tiles) are intentionally **not**
generated: `bundle.targets` does not include an MSIX/Store target, so
Tauri's Windows packaging (`msi`/`nsis`) does not need them. Generating
them anyway would just be unused clutter in this directory.

## `bundle.icon` (tauri.conf.json)

```json
"icon": [
  "icons/32x32.png",
  "icons/128x128.png",
  "icons/128x128@2x.png",
  "icons/icon.icns",
  "icons/icon.ico"
]
```

## `preview.png`

A contact sheet showing every generated size (macOS squircle renders,
Windows `.ico` frames, Linux PNGs) on both a light and a dark background,
so clipping, muddy small sizes, or off-center framing are visible at a
glance. Regenerate it by re-running the script.
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
