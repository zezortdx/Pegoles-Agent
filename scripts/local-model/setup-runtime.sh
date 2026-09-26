#!/usr/bin/env bash
# Set up the Pegoles Local inference runtime (MLX worker) on this Mac:
# a Python 3.12 virtualenv under the Pegoles data directory with exactly
# the hash-pinned packages in workers/mlx/requirements.lock.
#
#   bash scripts/local-model/setup-runtime.sh
#
# Needs a Python 3.10-3.12 interpreter (Homebrew: brew install python@3.12)
# and network access once. PEGOLES_DATA_DIR overrides the data directory.
# The desktop app finds the runtime at <data>/runtime/mlx-venv.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DATA="${PEGOLES_DATA_DIR:-$HOME/Library/Application Support/Pegoles}"
VENV="$DATA/runtime/mlx-venv"
LOCK="$ROOT/workers/mlx/requirements.lock"

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || { echo "Pegoles Local needs macOS on Apple silicon"; exit 1; }

PY=""
for cand in "${PEGOLES_PYTHON:-}" /opt/homebrew/bin/python3.12 /usr/local/bin/python3.12 \
  /Library/Frameworks/Python.framework/Versions/3.12/bin/python3.12 python3.12; do
  [ -n "$cand" ] && command -v "$cand" >/dev/null 2>&1 && { PY="$(command -v "$cand")"; break; }
done
[ -n "$PY" ] || { echo "Python 3.12 not found (brew install python@3.12, or set PEGOLES_PYTHON)"; exit 1; }
echo "python: $PY ($("$PY" --version))"

rm -rf "$VENV.tmp"
mkdir -p "$DATA/runtime"
"$PY" -m venv "$VENV.tmp"
"$VENV.tmp/bin/python" -m pip install --quiet --upgrade "pip==26.2.1"
# Wheels only, every file checked against the lock's SHA-256 digests.
"$VENV.tmp/bin/python" -m pip install --quiet --require-hashes --only-binary=:all: \
  --no-deps -r "$LOCK"
"$VENV.tmp/bin/python" -I -c 'import mlx.core as mx, mlx_vlm; assert mx.metal.is_available(), "Metal unavailable"; print("mlx", mx.__version__, "mlx-vlm", mlx_vlm.__version__)'
# Atomic swap: a half-built runtime is never the one the app uses.
rm -rf "$VENV.old"
[ -d "$VENV" ] && mv "$VENV" "$VENV.old"
mv "$VENV.tmp" "$VENV"
rm -rf "$VENV.old"
echo "runtime ready at $VENV"
