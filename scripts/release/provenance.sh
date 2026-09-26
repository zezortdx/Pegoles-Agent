#!/usr/bin/env bash
# Write the provenance files that accompany a release next to its DMG:
#
#   build-manifest.json         what was built, from which commit/tag, with
#                               which toolchains and pinned inputs (lockfile
#                               digests, bundled runtime manifest, guest image
#                               catalog, default model pin)
#   pegoles-<ver>.cdx.json      CycloneDX SBOM (syft) of Cargo.lock,
#                               pnpm-lock.yaml and workers/mlx/requirements.lock
#   SHA256SUMS                  over every asset above (written last)
#
#   bash scripts/release/provenance.sh [release-dir]   (default target/release-artifacts)
#   bash scripts/release/provenance.sh --no-dmg [dir]  manifest + SBOM only (release
#                                                      build job, before signing)
#   bash scripts/release/provenance.sh --sums [dir]    add the DMG to an existing
#                                                      manifest, write SHA256SUMS
#                                                      (signing job: runs no tool
#                                                      other than python3/shasum)
#
# The recorded tag is PEGOLES_RELEASE_TAG when set (CI), else the tag that
# points exactly at HEAD, else none. The SBOM needs syft (brew install
# syft); without it the SBOM is skipped with a warning locally and is an
# error in CI (CI=true).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
MODE=full
case "${1:-}" in
  --no-dmg) MODE=no-dmg; shift ;;
  --sums) MODE=sums; shift ;;
esac
DIST="${1:-$ROOT/target/release-artifacts}"
TAURI_DIR="$ROOT/apps/desktop/src-tauri"

fail() { echo "provenance: $*" >&2; exit 1; }

[ "$MODE" = no-dmg ] && mkdir -p "$DIST"
[ -d "$DIST" ] || fail "no release dir at $DIST (run scripts/package-macos.sh first)"
VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$TAURI_DIR/tauri.conf.json")"
DMG="$DIST/Pegoles_${VERSION}_arm64.dmg"
[ "$MODE" = no-dmg ] || [ -f "$DMG" ] || fail "no DMG at $DMG"
SBOM="pegoles-${VERSION}.cdx.json"

write_sums() {
  (
    cd "$DIST"
    assets=("$(basename "$DMG")" build-manifest.json)
    [ ! -f "$SBOM" ] || assets+=("$SBOM")
    shasum -a 256 "${assets[@]}" > SHA256SUMS
    shasum -a 256 -c SHA256SUMS >/dev/null
  )
  echo "provenance written to $DIST:"
  (cd "$DIST" && cat SHA256SUMS)
}

if [ "$MODE" = sums ]; then
  [ -f "$DIST/build-manifest.json" ] || fail "no build-manifest.json in $DIST (run --no-dmg in the build job)"
  python3 - "$DIST/build-manifest.json" "$DMG" <<'PY'
import hashlib, json, os, sys
out, dmg = sys.argv[1:3]
h = hashlib.sha256()
with open(dmg, "rb") as f:
    for chunk in iter(lambda: f.read(1 << 20), b""):
        h.update(chunk)
with open(out) as f:
    manifest = json.load(f)
manifest["assets"] = [a for a in manifest.get("assets", []) if a.get("name") != os.path.basename(dmg)]
manifest["assets"].append({"name": os.path.basename(dmg), "bytes": os.path.getsize(dmg), "sha256": h.hexdigest()})
with open(out, "w") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
PY
  write_sums
  exit 0
fi

# Tool versions; a missing tool is recorded as such, never guessed.
tool() { "$@" 2>/dev/null | head -1 || true; }
RUSTC_V="$(tool rustc --version)"
CARGO_V="$(tool cargo --version)"
NODE_V="$(tool node --version)"
PNPM_V="$(tool pnpm --version)"
SWIFT_V="$(swift --version 2>&1 | grep -m1 -i 'swift version' || true)"
XCODE_V="$(xcodebuild -version 2>/dev/null | paste -sd ' ' - || true)"
CLT_V="$(pkgutil --pkg-info=com.apple.pkg.CLTools_Executables 2>/dev/null | sed -n 's/^version: //p' || true)"
SDK_V="$(tool xcrun --sdk macosx --show-sdk-version)"
TAURI_V="$(tool pnpm --silent --filter @pegoles/desktop exec tauri --version)"
SYFT_V="$(syft version -o json 2>/dev/null || true)"
COMMIT="$(git rev-parse HEAD)"
DIRTY=false
[ -z "$(git status --porcelain --untracked-files=no)" ] || DIRTY=true
TAG="${PEGOLES_RELEASE_TAG:-$(git describe --tags --exact-match HEAD 2>/dev/null || true)}"

# Runtime manifest: the one inside the bundle is what ships.
RUNTIME_MANIFEST="$ROOT/target/release/bundle/macos/Pegoles Agent.app/Contents/Resources/runtime/runtime-manifest.json"
[ -f "$RUNTIME_MANIFEST" ] || RUNTIME_MANIFEST="$ROOT/target/pegoles-runtime/runtime-manifest.json"

# --- SBOM ---------------------------------------------------------------------
if command -v syft >/dev/null 2>&1; then
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  # Only the lockfiles are catalogued (not node_modules, target/ or venvs);
  # syft recognizes the pip lock under its conventional name.
  mkdir -p "$STAGE/workers/mlx"
  cp Cargo.lock pnpm-lock.yaml "$STAGE/"
  cp workers/mlx/requirements.lock "$STAGE/workers/mlx/requirements.txt"
  SYFT_CHECK_FOR_APP_UPDATE=false syft scan "dir:$STAGE" \
    --source-name pegoles-agent --source-version "$VERSION" \
    -o "cyclonedx-json=$DIST/$SBOM"
  [ -s "$DIST/$SBOM" ] || fail "syft produced no SBOM"
else
  [ "${CI:-}" != "true" ] || fail "syft is required in CI"
  echo "WARNING: syft not installed; no SBOM written (brew install syft)" >&2
  SBOM=""
fi

# --- build manifest ----------------------------------------------------------------
# Values reach Python through the environment only (a tag name or a tool
# banner is data, never code).
PV_VERSION="$VERSION" PV_COMMIT="$COMMIT" PV_TAG="$TAG" PV_DIRTY="$DIRTY" \
PV_HOST="$(uname -sm) $(sw_vers -productVersion 2>/dev/null || true)" \
PV_RUSTC="$RUSTC_V" PV_CARGO="$CARGO_V" PV_NODE="$NODE_V" PV_PNPM="$PNPM_V" \
PV_TAURI="$TAURI_V" PV_SWIFT="$SWIFT_V" PV_XCODE="$XCODE_V" PV_CLT="$CLT_V" \
PV_SDK="$SDK_V" PV_SYFT="$SYFT_V" \
python3 - "$DIST/build-manifest.json" "$DMG" "$RUNTIME_MANIFEST" <<'PY'
import hashlib, json, os, re, sys

out, dmg, runtime_manifest = sys.argv[1:4]
root = os.getcwd()
env = lambda k: os.environ.get(k) or None

def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()

def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except FileNotFoundError:
        return None

def product_image_id():
    # PEGOLES_PRODUCT_IMAGE_ID, resolved through one constant alias.
    with open(os.path.join(root, "crates/pegoles-computer/src/image.rs")) as f:
        consts = dict(re.findall(r'pub const (\w+): &str = ("[^"]*"|\w+);', f.read()))
    value = consts.get("PEGOLES_PRODUCT_IMAGE_ID")
    if value and not value.startswith('"'):
        value = consts.get(value)
    return value.strip('"') if value else None

def syft_version():
    try:
        return json.loads(os.environ.get("PV_SYFT") or "{}").get("version")
    except ValueError:
        return None

models = load(os.path.join(root, "crates/pegoles-inference/catalog/models.json")) or {}
default_id = models.get("default_model")
default_model = next((m for m in models.get("models", []) if m.get("id") == default_id), {})
source = default_model.get("source", {})
run = env("GITHUB_RUN_ID")

manifest = {
    "schema": 1,
    "product": "Pegoles Agent",
    "version": env("PV_VERSION"),
    "git": {"commit": env("PV_COMMIT"), "tag": env("PV_TAG"), "dirty": env("PV_DIRTY") == "true"},
    "build": {
        "host": env("PV_HOST"),
        "github_run": (
            f"{os.environ.get('GITHUB_SERVER_URL')}/{os.environ.get('GITHUB_REPOSITORY')}/actions/runs/{run}"
            if run else None
        ),
    },
    "toolchains": {
        "rustc": env("PV_RUSTC"),
        "cargo": env("PV_CARGO"),
        "node": env("PV_NODE"),
        "pnpm": env("PV_PNPM"),
        "tauri_cli": env("PV_TAURI"),
        "swift": env("PV_SWIFT"),
        "xcode": env("PV_XCODE"),
        "command_line_tools": env("PV_CLT"),
        "macos_sdk": env("PV_SDK"),
        "syft": syft_version(),
    },
    "inputs": {
        path: sha256(os.path.join(root, path))
        for path in ["Cargo.lock", "pnpm-lock.yaml", "workers/mlx/requirements.lock",
                     "crates/pegoles-inference/catalog/models.json"]
    },
    "runtime_manifest": load(runtime_manifest),
    "guest_image": {
        "product_image_id": product_image_id(),
        "catalog": load(os.path.join(root, "crates/pegoles-computer/catalog/images.json")),
    },
    "default_model": {
        "id": default_id,
        "repo": source.get("repo"),
        "revision": source.get("revision"),
    },
    "assets": [
        {"name": os.path.basename(dmg), "bytes": os.path.getsize(dmg), "sha256": sha256(dmg)},
    ] if os.path.exists(dmg) else [],
}
with open(out, "w") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
PY

# --- checksums -------------------------------------------------------------------------
if [ "$MODE" = no-dmg ]; then
  echo "manifest and SBOM written to $DIST (checksums follow after signing)"
else
  write_sums
fi
