#!/usr/bin/env bash
# Build the Pegoles Local inference runtime that ships inside the app:
# a relocatable CPython (python-build-standalone, pinned by release and
# SHA-256) plus exactly the wheels in workers/mlx/requirements.lock
# (every file pinned by SHA-256, wheels only, no dependency resolution).
#
#   bash scripts/local-model/build-runtime.sh [out-dir]
#
# Output (default target/pegoles-runtime): <out>/python/ (bin/python3.12,
# lib/, site-packages) and <out>/runtime-manifest.json. BUILD TIME ONLY:
# the app never runs pip, never resolves dependencies and never downloads
# Python code. scripts/package-macos.sh copies <out>/python into
# "Pegoles Agent.app/Contents/Resources/runtime/python" and signs every
# Mach-O in it.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/target/pegoles-runtime}"
CACHE="$ROOT/target/pegoles-runtime-cache"
LOCK="$ROOT/workers/mlx/requirements.lock"

# Pinned interpreter: python-build-standalone release 20260924, CPython
# 3.12.14, aarch64-apple-darwin, install_only_stripped. The digest is the
# one published in the release's SHA256SUMS and in the GitHub asset
# metadata; a different file fails the build.
PBS_RELEASE="20260924"
PY_VERSION="3.12.14"
PBS_FILE="cpython-${PY_VERSION}+${PBS_RELEASE}-aarch64-apple-darwin-install_only_stripped.tar.gz"
PBS_URL="https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_RELEASE}/cpython-${PY_VERSION}%2B${PBS_RELEASE}-aarch64-apple-darwin-install_only_stripped.tar.gz"
PBS_SHA256="c2edb321cd32ec2b170df208db0446dccc4398db602ca27cf2079098fb1f7d9d"

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || { echo "build on macOS / Apple silicon" >&2; exit 1; }

sha256() { shasum -a 256 "$1" | awk '{print $1}'; }

mkdir -p "$CACHE"
TARBALL="$CACHE/$PBS_FILE"
if [ ! -f "$TARBALL" ] || [ "$(sha256 "$TARBALL")" != "$PBS_SHA256" ]; then
  rm -f "$TARBALL.part"
  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
    --output "$TARBALL.part" "$PBS_URL"
  mv "$TARBALL.part" "$TARBALL"
fi
got="$(sha256 "$TARBALL")"
[ "$got" = "$PBS_SHA256" ] || { echo "python-build-standalone digest mismatch: $got" >&2; exit 1; }

STAGE="$OUT.tmp"
rm -rf "$STAGE"
mkdir -p "$STAGE"
tar -xzf "$TARBALL" -C "$STAGE" --no-same-owner
PY="$STAGE/python/bin/python3.12"
[ -x "$PY" ] || { echo "unexpected archive layout" >&2; exit 1; }

SITE="$STAGE/python/lib/python3.12/site-packages"
# The installer is the pip bundled in the interpreter archive, so it is
# authenticated by PBS_SHA256 above (no separate, unverified bootstrap).
# Wheels only, no dependency resolution, every downloaded file checked
# against the lock's SHA-256 digests (a re-uploaded or substituted wheel
# fails closed). pip itself is deleted from the runtime below.
env -i PATH=/usr/bin:/bin HOME="$HOME" "$PY" -I -m pip install --quiet \
  --disable-pip-version-check --no-deps --require-hashes --only-binary=:all: \
  --no-compile --no-input -r "$LOCK"

# Strip what the worker never needs and what must not ship: installers,
# console scripts with absolute shebangs, the test suite, GUI toolkits.
rm -rf "$SITE"/pip "$SITE"/pip-* "$SITE"/setuptools "$SITE"/setuptools-* \
  "$SITE"/_distutils_hack "$SITE"/distutils-precedence.pth "$SITE"/bin
find "$STAGE/python/bin" -mindepth 1 ! -name python3.12 -exec rm -rf {} +
# Their RECORD entries hash absolute shebangs (the build path): drop them so
# the tree does not depend on where it was built.
find "$SITE" -path '*.dist-info/RECORD' -type f -exec sed -i '' '/^\.\.\/\.\.\/\.\.\/bin\//d' {} +
LIB="$STAGE/python/lib/python3.12"
rm -rf "$LIB/test" "$LIB/idlelib" "$LIB/tkinter" "$LIB/turtledemo" "$LIB/ensurepip" \
  "$LIB/lib2to3" "$LIB/pydoc_data" "$LIB/turtle.py" \
  "$LIB"/lib-dynload/_tkinter*.so "$STAGE/python/lib"/libtcl* "$STAGE/python/lib"/libtk* \
  "$STAGE/python/lib"/tcl* "$STAGE/python/lib"/tk* "$STAGE/python/lib/itcl"* \
  "$STAGE/python/lib/thread"* "$STAGE/python/include" "$STAGE/python/share" \
  "$STAGE/python/lib/pkgconfig"
find "$STAGE/python" -name '__pycache__' -type d -prune -exec rm -rf {} +
find "$STAGE/python" \( -name '*.a' -o -name '*.pyi' \) -type f -delete

# Precompile bytecode deterministically (hash-checked .pyc, no mtimes), so
# the sandboxed worker, which cannot write next to its sources, still
# starts quickly.
SOURCE_DATE_EPOCH=0 "$PY" -I -m compileall -f -q -j 0 --invalidation-mode unchecked-hash \
  -d "" "$STAGE/python/lib/python3.12" >/dev/null || {
  echo "bytecode compilation failed" >&2; exit 1; }

# Smoke: the runtime imports and sees Metal, isolated from the build env.
env -i PATH=/usr/bin:/bin HOME="$HOME" PYTHONDONTWRITEBYTECODE=1 "$PY" -I -c \
  'import mlx.core as mx, mlx_vlm, transformers; assert mx.metal.is_available(), "Metal unavailable"; print("mlx", mx.__version__, "mlx-vlm", mlx_vlm.__version__, "transformers", transformers.__version__)'

# No symlink may point outside the runtime, and nothing may be writable by
# group/other.
while IFS= read -r link; do
  target="$(cd "$(dirname "$link")" && realpath "$(readlink "$link")" 2>/dev/null || true)"
  case "$target" in
    "$STAGE/python"/*) ;;
    *) echo "symlink escapes the runtime: $link -> $(readlink "$link")" >&2; exit 1 ;;
  esac
done < <(find "$STAGE/python" -type l)
chmod -R go-w "$STAGE/python"

# Manifest: what was built, from what. Inputs are pinned; the tree digest
# covers every file path and content (order-independent of the filesystem).
TREE_SHA="$(cd "$STAGE/python" && find . -type f -print0 | LC_ALL=C sort -z \
  | xargs -0 shasum -a 256 | shasum -a 256 | awk '{print $1}')"
PACKAGES="$(grep -E '^[A-Za-z0-9_.-]+==' "$LOCK" | sed 's/ .*//' | paste -sd, -)"
LOCK_SHA="$(sha256 "$LOCK")"
SIZE_KB="$(du -sk "$STAGE/python" | awk '{print $1}')"
cat > "$STAGE/runtime-manifest.json" <<EOF
{
  "schema": 1,
  "python": "$PY_VERSION",
  "python_build_standalone": {"release": "$PBS_RELEASE", "file": "$PBS_FILE", "sha256": "$PBS_SHA256"},
  "requirements_lock_sha256": "$LOCK_SHA",
  "packages": "$PACKAGES",
  "tree_sha256": "$TREE_SHA",
  "size_kb": $SIZE_KB
}
EOF

rm -rf "$OUT"
mv "$STAGE" "$OUT"
echo "runtime ready at $OUT/python ($((SIZE_KB / 1024)) MB, tree $TREE_SHA)"
