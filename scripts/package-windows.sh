#!/usr/bin/env bash
# Build the Windows installer from the current checkout. Nothing is signed
# or published.
#
#   bash scripts/package-windows.sh          (Git Bash, Windows x64)
#
# Output: target/release-artifacts/Pegoles-Setup-x64.exe and its .sha256.
#
# Needs: Rust (MSVC toolchain), Node + pnpm, CMake, LLVM (libclang for
# llama.cpp's bindings; set LIBCLANG_PATH if it is not found) and the
# Vulkan SDK (VULKAN_SDK set: llama.cpp compiles its shaders with glslc).
# Run it from an "x64 Native Tools" environment (MSVC and Ninja on PATH):
# llama.cpp's Vulkan shader generator does not install under the Visual
# Studio generator, so the worker is built with Ninja.
#
# Install layout (per machine, "C:\Program Files\Pegoles Agent\", which
# only administrators can write):
#   pegoles-desktop.exe              the app (runs as the user, no elevation)
#   pegoles-vm-host.exe              unprivileged VM helper, one per app session
#   pegoles-broker.exe               PegolesVmBroker service (LocalSystem, demand start)
#   pegoles-llm-worker.exe           Pegoles Local worker (AppContainer + job object)
#   llama.dll, ggml*.dll, mtmd.dll   llama.cpp, loaded from this folder only
#   msvcp140*.dll, vcruntime140*.dll Visual C++ runtime for llama.cpp (app-local)
#
# The installer is unsigned unless a later step signs it
# (docs/RELEASE_GATES.md): SmartScreen warns on first run, and Smart App
# Control blocks unsigned apps outright.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) ;; *) echo "package on Windows (Git Bash)" >&2; exit 1 ;; esac
[ "$(uname -m)" = "x86_64" ] || { echo "package on Windows x64" >&2; exit 1; }
TRIPLE="x86_64-pc-windows-msvc"
TAURI_DIR="$ROOT/apps/desktop/src-tauri"
BIN="$TAURI_DIR/binaries"
GENERATED="$TAURI_DIR/tauri.windows.bundle.generated.json"
OUT="$ROOT/target/release-artifacts"
# llama.cpp's Vulkan shader generator is a nested CMake build whose
# paths overflow Windows' 260-character limit under a deep checkout:
# build the worker in a short target directory.
LLAMA_TARGET_DIR="${PEGOLES_LLAMA_TARGET_DIR:-/c/pl}"
LLAMA_TARGET="$LLAMA_TARGET_DIR/release"

# Python as installed on Windows ("python3" may be a Store alias there).
PY="$(command -v python || command -v python3)" || { echo "python not found" >&2; exit 1; }

# --- versions must agree -------------------------------------------------
json_version() { "$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$1"; }
VERSION="$(json_version "$TAURI_DIR/tauri.conf.json")"
CARGO_VERSION="$(sed -n '/^version = "/{s/^version = "\(.*\)"/\1/p;q;}' "$TAURI_DIR/Cargo.toml")"
for v in "$CARGO_VERSION" "$(json_version "$ROOT/apps/desktop/package.json")" "$(json_version "$ROOT/package.json")"; do
  [ "$v" = "$VERSION" ] || { echo "version mismatch: tauri.conf.json=$VERSION vs $v" >&2; exit 1; }
done

# No developer paths in shipped binaries (see package-macos.sh). The Rust
# executables link the C runtime statically (a clean Windows has no Visual
# C++ runtime); llama.cpp's libraries use the dynamic one, deployed next to
# them below.
win() { cygpath -w "$1"; }
REMAP="$(printf '%s\x1f%s\x1f%s' "--remap-path-prefix=$(win "$HOME")=/home" \
  "--remap-path-prefix=$(win "$ROOT")=/pegoles" "--remap-path-prefix=$(win "${CARGO_HOME:-$HOME/.cargo}")=/cargo")"
STATIC_CRT="$(printf '%s\x1f%s' "$REMAP" "-Ctarget-feature=+crt-static")"
unset RUSTFLAGS

command -v ninja >/dev/null && command -v cl >/dev/null || {
  echo "run from an x64 Native Tools environment (needs cl.exe and ninja.exe on PATH)" >&2
  exit 1
}
export CMAKE_GENERATOR=Ninja

# --- native parts ----------------------------------------------------------
CARGO_ENCODED_RUSTFLAGS="$STATIC_CRT" cargo build --release --locked -p pegoles-broker -p pegoles-vm-host-windows
CARGO_ENCODED_RUSTFLAGS="$REMAP" CARGO_TARGET_DIR="$(win "$LLAMA_TARGET_DIR")" \
  cargo build --release --locked --manifest-path workers/llama/Cargo.toml

rm -rf "$BIN" "$GENERATED"
mkdir -p "$BIN/llama"
install -m 0755 target/release/pegoles-broker.exe "$BIN/pegoles-broker-$TRIPLE.exe"
install -m 0755 target/release/pegoles-vm-host.exe "$BIN/pegoles-vm-host-$TRIPLE.exe"
install -m 0755 "$LLAMA_TARGET/pegoles-llm-worker.exe" "$BIN/pegoles-llm-worker-$TRIPLE.exe"

# llama.cpp: the shared libraries the worker links against (copied next to
# it by llama-cpp-sys-2) and the backend modules it loads at start (CPU
# variants, Vulkan) from the newest build's backends directory.
BACKENDS="$(ls -1dt "$LLAMA_TARGET"/build/llama-cpp-sys-2-*/out/backends 2>/dev/null | head -1)"
[ -n "$BACKENDS" ] || { echo "llama.cpp backend modules not found" >&2; exit 1; }
shopt -s nullglob
dlls=("$LLAMA_TARGET"/*.dll "$BACKENDS"/*.dll)
shopt -u nullglob
[ "${#dlls[@]}" -gt 0 ] || { echo "llama.cpp DLLs not found" >&2; exit 1; }
for dll in "${dlls[@]}"; do install -m 0644 "$dll" "$BIN/llama/"; done
ls "$BIN/llama"/ggml-vulkan.dll >/dev/null || { echo "Vulkan backend missing" >&2; exit 1; }
ls "$BIN/llama"/ggml-cpu-*.dll >/dev/null || { echo "CPU backends missing" >&2; exit 1; }

# The Visual C++ runtime for those libraries, deployed app-locally (a
# deployment Microsoft supports), from this machine's Visual Studio.
VSWHERE="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
VS="$(cygpath -u "$("$VSWHERE" -latest -products '*' \
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | tr -d '\r')")"
CRT="$(find "$VS/VC/Redist/MSVC" -maxdepth 3 -type d -path '*/x64/Microsoft.VC14*.CRT' | sort -V | tail -1)"
[ -n "$CRT" ] || { echo "Visual C++ redistributable DLLs not found" >&2; exit 1; }
for dll in msvcp140.dll msvcp140_1.dll msvcp140_2.dll vcruntime140.dll vcruntime140_1.dll; do
  install -m 0644 "$CRT/$dll" "$BIN/llama/"
done

# The sidecars and DLLs go next to the app, listed file by file.
"$PY" - "$GENERATED" "$BIN/llama" <<'PY'
import json, os, sys
out, llama = sys.argv[1], sys.argv[2]
resources = {f"binaries/llama/{n}": n for n in sorted(os.listdir(llama)) if n.endswith(".dll")}
config = {
    "bundle": {
        "externalBin": ["binaries/pegoles-vm-host", "binaries/pegoles-broker", "binaries/pegoles-llm-worker"],
        "resources": resources,
    }
}
with open(out, "w") as f:
    json.dump(config, f, indent=2)
print(f"{len(resources)} DLLs staged (llama.cpp + Visual C++ runtime)")
PY

# --- app + installer --------------------------------------------------------
CARGO_ENCODED_RUSTFLAGS="$STATIC_CRT" pnpm --filter @pegoles/desktop tauri build --bundles nsis --config "$(win "$GENERATED")"
shopt -s nullglob
setups=("$ROOT"/target/release/bundle/nsis/*"_${VERSION}_x64-setup.exe")
shopt -u nullglob
[ "${#setups[@]}" -eq 1 ] || { echo "expected one NSIS installer for $VERSION, found ${#setups[@]}" >&2; exit 1; }
mkdir -p "$OUT"
install -m 0644 "${setups[0]}" "$OUT/Pegoles-Setup-x64.exe"
(cd "$OUT" && sha256sum Pegoles-Setup-x64.exe > Pegoles-Setup-x64.exe.sha256 && cat Pegoles-Setup-x64.exe.sha256)
echo "built $OUT/Pegoles-Setup-x64.exe (unsigned, version $VERSION)"
