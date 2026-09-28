#!/usr/bin/env bash
# Rebuild every Pegoles brand artifact from the source PNG (build time only).
#
#   trace.py    PNG -> fitted vector geometry + sampled colors (geometry.json)
#   build.py    geometry.json + look.json -> SVGs + packages/ui/src/brand/markGeometry.generated.ts
#   verify.py   rasterize the shipped SVG (pure Python + Chromium) -> IoU (fidelity.json); fails < 0.97
#   variants.py source pixels -> transparent PNG/WebP variants (raster/)
#   presence-art.py source pixels -> the app's living mark (body without eyes + eye sprites)
#   make-icons.py source pixels + glyph -> app icons (.icns/.ico/PNGs), installer art, small sizes
#
# tune_look.py (optional, slow, needs Chromium) refits the lighting recipe in look.json.
# Requires: python3 with Pillow, NumPy, SciPy. Chromium is optional (verify cross-check).
set -euo pipefail
cd "$(dirname "$0")"
python3 trace.py
python3 build.py
python3 verify.py
python3 variants.py
python3 presence-art.py
python3 make-icons.py
