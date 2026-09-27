#!/usr/bin/env bash
# Write THIRD_PARTY_NOTICES.md: the licenses of everything Pegoles Agent.app
# redistributes. BUILD TIME ONLY; package-macos.sh copies the result into
# Contents/Resources.
#
#   bash scripts/release/third-party-notices.sh [out-file]
#       (default target/THIRD_PARTY_NOTICES.md)
#
# Sections: the Python runtime (CPython and the native libraries
# python-build-standalone links into it, then every wheel with the license
# files it ships, which also stay inside the bundle's dist-info folders);
# the frontend packages bundled into the web UI (pnpm, production only);
# the Rust crates compiled into the app (cargo-about, full license texts).
# Needs: the runtime built (scripts/local-model/build-runtime.sh), pnpm
# dependencies installed, and cargo-about (brew install cargo-about).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/target/THIRD_PARTY_NOTICES.md}"
RUNTIME="$ROOT/target/pegoles-runtime/python"
[ -x "$RUNTIME/bin/python3.12" ] || { echo "build the runtime first (scripts/local-model/build-runtime.sh)" >&2; exit 1; }
command -v cargo-about >/dev/null || { echo "cargo-about is required (brew install cargo-about)" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- Python runtime --------------------------------------------------------
env -i PATH=/usr/bin:/bin "$RUNTIME/bin/python3.12" -I - "$RUNTIME" > "$TMP/python.md" <<'PY'
import ssl, sqlite3, sys, _decimal, os, glob
from importlib import metadata
runtime = sys.argv[1]
print("# Python runtime (Pegoles Local)\n")
print("The model worker runs a relocatable CPython from python-build-standalone")
print(f"(CPython {sys.version.split()[0]}) with the packages below, all shipped inside")
print("`Contents/Resources/runtime/python`. Each package's own license files are")
print("also kept in its `site-packages/<name>.dist-info` folder in the bundle.\n")
print("## CPython and the native libraries linked into it\n")
print("| Component | Version | License |")
print("|---|---|---|")
print(f"| CPython | {sys.version.split()[0]} | PSF-2.0 (lib/python3.12/LICENSE.txt, reproduced below) |")
print(f"| OpenSSL (static) | {ssl.OPENSSL_VERSION.split()[1]} | Apache-2.0 |")
print(f"| SQLite (static) | {sqlite3.sqlite_version} | Public domain |")
print(f"| mpdecimal (static) | {_decimal.__libmpdec_version__} | BSD-2-Clause |")
print("| XZ Utils / liblzma (static) | as built by python-build-standalone | 0BSD |")
print("| bzip2 (static) | as built by python-build-standalone | bzip2-1.0.6 |")
print("| libffi (static) | as built by python-build-standalone | MIT |")
print("| zlib, libedit, libuuid | macOS system libraries (not redistributed) | — |\n")
print("## Packages\n")
print("| Package | Version | License |")
print("|---|---|---|")
dists = sorted(metadata.distributions(), key=lambda d: d.metadata["Name"].lower())
for d in dists:
    m = d.metadata
    lic = m.get("License-Expression") or ""
    if not lic:
        classifiers = [c.split(" :: ")[-1] for c in m.get_all("Classifier") or [] if c.startswith("License ::")]
        lic = "; ".join(classifiers) or (m.get("License") or "").splitlines()[0][:80] if (classifiers or m.get("License")) else "see dist-info"
    print(f"| {m['Name']} | {m['Version']} | {lic} |")
print("\n## CPython license\n\n```")
with open(os.path.join(runtime, "lib/python3.12/LICENSE.txt"), encoding="utf-8") as f:
    print(f.read().strip())
print("```\n")
print("## Package license texts\n")
for d in dists:
    files = [str(f) for f in (d.files or []) if "licen" in str(f).lower() or "copying" in str(f).lower() or str(f).lower().endswith(("notice", "notice.txt", "authors.txt"))]
    if not files:
        continue
    print(f"### {d.metadata['Name']} {d.metadata['Version']}\n")
    for f in files:
        path = d.locate_file(f)
        try:
            text = open(path, encoding="utf-8", errors="replace").read().strip()
        except (IsADirectoryError, FileNotFoundError):
            continue
        print(f"`{f}`\n\n```\n{text}\n```\n")
PY

# --- Frontend (bundled into the web UI) ------------------------------------------
if ! pnpm --silent --filter @pegoles/desktop licenses list --prod --json > "$TMP/npm.json"; then
  echo "pnpm licenses list failed: $(head -c 600 "$TMP/npm.json")" >&2
  exit 1
fi
python3 - "$TMP/npm.json" > "$TMP/npm.md" <<'PY'
import json, sys, os
data = json.load(open(sys.argv[1]))
print("# Frontend packages (bundled into the web UI)\n")
print("| Package | Version | License |")
print("|---|---|---|")
rows = []
for lic, pkgs in data.items():
    for p in pkgs:
        for v in p.get("versions", [p.get("version", "")]):
            rows.append((p["name"], v, lic, (p.get("paths") or [None])[0]))
for name, v, lic, _ in sorted(rows):
    print(f"| {name} | {v} | {lic} |")
print("\n## License texts\n")
for name, v, lic, path in sorted(rows):
    if not path or not os.path.isdir(path):
        continue
    for f in sorted(os.listdir(path)):
        if f.lower().startswith(("license", "licence", "copying", "notice")):
            text = open(os.path.join(path, f), encoding="utf-8", errors="replace").read().strip()
            print(f"### {name} {v} ({f})\n\n```\n{text}\n```\n")
PY

# --- Rust -------------------------------------------------------------------------------
cargo about generate --locked --config scripts/release/about/about.toml \
  --manifest-path apps/desktop/src-tauri/Cargo.toml scripts/release/about/about.hbs > "$TMP/rust.md"

{
  echo "# Third-party notices for Pegoles Agent"
  echo
  echo "Pegoles Agent is MIT licensed (see LICENSE). It redistributes the"
  echo "third-party components listed below under their own licenses."
  echo
  cat "$TMP/python.md" "$TMP/npm.md" "$TMP/rust.md"
} > "$OUT"
echo "wrote $OUT ($(wc -c < "$OUT" | tr -d ' ') bytes)"
