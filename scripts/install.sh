#!/bin/bash
# Build Pegoles from this checkout and install it as ~/Applications/Pegoles.app.
#
#   git clone https://github.com/zezortdx/Pegoles-Agent.git
#   cd Pegoles-Agent
#   ./scripts/install.sh
#
# Needs: a Mac with Apple silicon, macOS 14 or later, the Xcode Command Line
# Tools (`xcode-select --install`), an internet connection and about 15 GB
# of free disk space for the build. Nothing else: the build tools (Rust,
# Node.js, pnpm, the Pegoles Local Python runtime) are downloaded into this
# checkout's target/ directory, each pinned by version and SHA-256, and are
# never installed system-wide. No sudo, no Apple Developer account, no API
# key.
#
# The app is signed ad hoc on this Mac (not notarized). It is built here,
# so it never carries a quarantine attribute and Gatekeeper is neither
# involved nor changed. The installed app does not use this checkout: you
# can delete the checkout afterwards. The computer image and the local model
# are downloaded and verified by the app itself, on first use.
#
# Re-running the script rebuilds and replaces the installed app (quit
# Pegoles first). Your data in ~/Library/Application Support/Pegoles is
# never touched. To uninstall: delete ~/Applications/Pegoles.app and,
# optionally, ~/Library/Application Support/Pegoles.
set -euo pipefail

# --- clean environment ------------------------------------------------------
# Developer settings (RUSTFLAGS, CARGO_*, NODE_OPTIONS, PYTHON*, DYLD_*,
# PEGOLES_* overrides, a custom PATH) must not reach the build: re-run this
# script with only what it needs.
if [ "${PEGOLES_INSTALL_ENV:-}" != "clean" ]; then
  exec /usr/bin/env -i \
    PEGOLES_INSTALL_ENV=clean \
    HOME="${HOME:-}" \
    USER="${USER:-}" \
    LOGNAME="${LOGNAME:-}" \
    TMPDIR="${TMPDIR:-/tmp}" \
    TERM="${TERM:-dumb}" \
    LANG=en_US.UTF-8 \
    LC_ALL=en_US.UTF-8 \
    PATH=/usr/bin:/bin:/usr/sbin:/sbin \
    /bin/bash "${BASH_SOURCE[0]}" "$@"
fi
umask 022

# --- pinned build tools -------------------------------------------------------
RUST_VERSION="1.97.1"
RUST_DIST="https://static.rust-lang.org/dist/2026-07-16"
RUSTC_SHA256="6076cad38ccabaa24325f26a74080a363a2633a9cd34c473a8977255d8a593cb"
CARGO_SHA256="2d84a74e9558192a7de674aca6aa3ab7464bed2df97e0377156ddb7e09a0fd7a"
RUST_STD_SHA256="a4895f5c6995e83cab8687e46b14324592398049def71ce75ca308c981cf200d"
NODE_VERSION="24.21.0"
NODE_SHA256="6239d4cf92d864487ec8cd3615038f7b67e7f58b77b21cd2f09ea9fbd68065fe"
PNPM_VERSION="9.15.9"
PNPM_SHA512="68046141893c66fad01c079231128e9afb89ef87e2691d69e4d40eee228988295fd4682181bae55b58418c3a253bde65a505ec7c5f9403ece5cc3cd37dcf2531"
CARGO_ABOUT_VERSION="0.9.2"

MIN_MACOS_MAJOR=14
MIN_FREE_GB=15
BUNDLE_ID="ai.pegoles.agent"
APP_NAME="Pegoles.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

# --- where we are ---------------------------------------------------------------
# The checkout is found from this script's own location (canonical, symlinks
# resolved), never from the current directory.
SELF="$(realpath "${BASH_SOURCE[0]}" 2>/dev/null)" || die "cannot resolve the installer's path"
ROOT="$(dirname "$(dirname "$SELF")")"
case "$ROOT" in /*) ;; *) die "cannot resolve the checkout directory" ;; esac
# Paths reach several build tools; refuse characters that some of them
# would interpret (spaces are fine).
unsafe_path() {
  case "$1" in *'"'* | *'$'* | *'`'* | *'\'* | *"
"*) return 0 ;; esac
  return 1
}
unsafe_path "$ROOT" && die "the checkout path contains a quote, \$, backtick, backslash or newline: move it"
unsafe_path "${HOME:-}" && die "HOME contains a quote, \$, backtick, backslash or newline"
for f in Cargo.toml Cargo.lock pnpm-lock.yaml scripts/package-macos.sh \
  apps/desktop/src-tauri/tauri.conf.json workers/mlx/requirements.lock; do
  [ -f "$ROOT/$f" ] || die "$ROOT does not look like a Pegoles checkout (missing $f)"
done
grep -q "\"identifier\": \"$BUNDLE_ID\"" "$ROOT/apps/desktop/src-tauri/tauri.conf.json" \
  || die "unexpected app identifier in tauri.conf.json"
VERSION="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$ROOT/apps/desktop/src-tauri/tauri.conf.json" | head -1)"
[ -n "$VERSION" ] || die "cannot read the version from tauri.conf.json"
TARGET_DIR="$ROOT/target"
TOOLS="$TARGET_DIR/bootstrap"

# --- preflight --------------------------------------------------------------------
preflight() {
  [ "$(uname -s)" = "Darwin" ] || die "Pegoles runs on macOS only"
  [ "$(uname -m)" = "arm64" ] || die "Pegoles needs a Mac with Apple silicon (this shell reports $(uname -m))"
  if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
    die "this shell runs under Rosetta; run the installer from a native (arm64) terminal"
  fi
  local os major
  os="$(sw_vers -productVersion)"
  major="${os%%.*}"
  case "$major" in '' | *[!0-9]*) die "cannot read the macOS version ($os)" ;; esac
  [ "$major" -ge "$MIN_MACOS_MAJOR" ] || die "Pegoles needs macOS $MIN_MACOS_MAJOR or later (this Mac runs $os)"
  [ "$(id -u)" != 0 ] || die "do not run the installer as root or with sudo"

  case "$HOME" in /*) ;; *) die "HOME is not set to an absolute path" ;; esac
  [ -d "$HOME" ] || die "HOME ($HOME) does not exist"
  [ "$(stat -f %u "$HOME")" = "$(id -u)" ] || die "HOME ($HOME) is not owned by $(id -un)"

  if ! { xcode-select -p && xcrun --find swift && xcrun --find clang; } >/dev/null 2>&1; then
    die "the Xcode Command Line Tools are required: run 'xcode-select --install', then run this script again"
  fi
  local tool
  for tool in curl shasum tar codesign ditto python3 xattr stat df; do
    command -v "$tool" >/dev/null || die "missing system tool: $tool"
  done

  # A checkout that came from a downloaded archive carries quarantine
  # attributes. The installer does not remove them for you: clone with git
  # (which never sets them), or clear them yourself if you trust the source.
  local quarantined
  quarantined="$(find "$ROOT" \( -path "$TARGET_DIR" -o -path "$ROOT/node_modules" -o -path "$ROOT/.git" \) -prune \
    -o -xattrname com.apple.quarantine -print 2>/dev/null | head -3)"
  if [ -n "$quarantined" ]; then
    die "files in this checkout are quarantined (downloaded from the internet), e.g.:
$quarantined
Clone the repository with git instead (see README.md), or, if you trust this
copy, remove the attribute yourself: xattr -dr com.apple.quarantine \"$ROOT\""
  fi

  mkdir -p "$TARGET_DIR"
  # A rebuild reuses the tools and most of the build cache.
  local need="$MIN_FREE_GB" free_gb
  [ -f "$TOOLS/rust/.pegoles-stamp" ] && [ -d "$TARGET_DIR/release" ] && need=5
  free_gb="$(df -Pk "$TARGET_DIR" | awk 'NR==2 {print int($4 / 1048576)}')"
  [ "$free_gb" -ge "$need" ] \
    || die "not enough free disk space: ${free_gb} GB free, the build needs about ${need} GB"
}

# One installer at a time per checkout.
LOCK="$TARGET_DIR/.install.lock"
take_lock() {
  mkdir -p "$TARGET_DIR"
  if ! mkdir "$LOCK" 2>/dev/null; then
    local pid
    pid="$(cat "$LOCK/pid" 2>/dev/null || true)"
    case "$pid" in '' | *[!0-9]*) pid="" ;; esac
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      die "another install is running (pid $pid)"
    fi
    rm -rf "$LOCK"
    mkdir "$LOCK" || die "cannot take the install lock $LOCK"
  fi
  echo $$ >"$LOCK/pid"
}

# --- downloads -------------------------------------------------------------------------
digest() { shasum -a "$1" "$2" | awk '{print $1}'; }

# fetch <url> <file> <sha256|sha512> <hex digest>
# Downloads over HTTPS only, into a .part file, and keeps the file only if
# its digest matches the pin. A cached file is re-checked every time.
fetch() {
  local url="$1" out="$2" algo="$3" want="$4" bits
  case "$algo" in sha256) bits=256 ;; sha512) bits=512 ;; *) die "bad digest algorithm $algo" ;; esac
  if [ -f "$out" ] && [ "$(digest "$bits" "$out")" = "$want" ]; then
    return 0
  fi
  rm -f "$out" "$out.part"
  say "download $(basename "$out")"
  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
    --retry 3 --retry-delay 2 --connect-timeout 30 \
    --output "$out.part" "$url" || die "download failed: $url"
  local got
  got="$(digest "$bits" "$out.part")"
  if [ "$got" != "$want" ]; then
    rm -f "$out.part"
    die "checksum mismatch for $url (got $got, expected $want)"
  fi
  mv "$out.part" "$out"
}

# fresh_dir <path under $TOOLS>: remove and recreate a staging directory.
fresh_dir() {
  case "$1" in "$TOOLS"/?*) ;; *) die "refusing to reset $1 (outside $TOOLS)" ;; esac
  rm -rf "$1"
  mkdir -p "$1"
}

# install_tool <name> <stamp> <build-function>: build a tool into a staging
# directory and move it into place only when complete, so an interrupted
# run never leaves a half-installed tool behind.
install_tool() {
  local name="$1" stamp="$2" builder="$3" dest="$TOOLS/$1"
  if [ -f "$dest/.pegoles-stamp" ] && [ "$(cat "$dest/.pegoles-stamp")" = "$stamp" ]; then
    return 0
  fi
  local stage="$TOOLS/.$name.stage"
  fresh_dir "$stage"
  "$builder" "$stage"
  echo "$stamp" >"$stage/.pegoles-stamp"
  case "$dest" in "$TOOLS"/?*) rm -rf "$dest" ;; esac
  mv "$stage" "$dest"
}

build_rust() {
  local stage="$1" part tmp
  tmp="$TOOLS/.rust.extract"
  for part in "rustc:$RUSTC_SHA256" "cargo:$CARGO_SHA256" "rust-std:$RUST_STD_SHA256"; do
    local comp="${part%%:*}" sha="${part#*:}"
    local file="$comp-$RUST_VERSION-aarch64-apple-darwin.tar.xz"
    fetch "$RUST_DIST/$file" "$TOOLS/downloads/$file" sha256 "$sha"
    fresh_dir "$tmp"
    tar -xJf "$TOOLS/downloads/$file" -C "$tmp" --no-same-owner
    /bin/bash "$tmp/$comp-$RUST_VERSION-aarch64-apple-darwin/install.sh" \
      --prefix="$stage" --disable-ldconfig >/dev/null
  done
  rm -rf "$tmp"
  "$stage/bin/rustc" --version | grep -q "^rustc $RUST_VERSION " || die "unexpected rustc"
}

build_node() {
  local stage="$1" file="node-v$NODE_VERSION-darwin-arm64.tar.xz"
  fetch "https://nodejs.org/dist/v$NODE_VERSION/$file" "$TOOLS/downloads/$file" sha256 "$NODE_SHA256"
  tar -xJf "$TOOLS/downloads/$file" -C "$stage" --no-same-owner --strip-components 1
  [ "$("$stage/bin/node" --version)" = "v$NODE_VERSION" ] || die "unexpected node"
}

build_pnpm() {
  local stage="$1" file="pnpm-$PNPM_VERSION.tgz"
  fetch "https://registry.npmjs.org/pnpm/-/$file" "$TOOLS/downloads/$file" sha512 "$PNPM_SHA512"
  tar -xzf "$TOOLS/downloads/$file" -C "$stage" --no-same-owner --strip-components 1
  [ -f "$stage/bin/pnpm.cjs" ] || die "unexpected pnpm package layout"
}

build_cargo_about() {
  # From crates.io, with the lockfile it was published with.
  cargo install --quiet --locked --root "$1" "cargo-about@$CARGO_ABOUT_VERSION"
}

bootstrap() {
  mkdir -p "$TOOLS/downloads" "$TOOLS/bin"
  say "build tools (pinned, in target/bootstrap)"
  install_tool rust "$RUST_VERSION:$RUSTC_SHA256:$CARGO_SHA256:$RUST_STD_SHA256" build_rust
  install_tool node "$NODE_VERSION:$NODE_SHA256" build_node
  install_tool pnpm "$PNPM_VERSION:$PNPM_SHA512" build_pnpm
  # The shim finds node and pnpm next to itself (no path baked in).
  cat >"$TOOLS/bin/pnpm" <<'EOF'
#!/bin/sh
tools="$(cd -P "$(dirname "$0")/.." && pwd -P)" || exit 1
exec "$tools/node/bin/node" "$tools/pnpm/bin/pnpm.cjs" "$@"
EOF
  chmod 0755 "$TOOLS/bin/pnpm"

  export CARGO_HOME="$TOOLS/cargo-home"
  export PATH="$TOOLS/bin:$TOOLS/rust/bin:$TOOLS/node/bin:$TOOLS/cargo-about/bin:$PATH"
  # pnpm's content store and caches stay inside the checkout.
  export npm_config_store_dir="$TOOLS/pnpm-store"
  export npm_config_cache="$TOOLS/npm-cache"
  export XDG_CACHE_HOME="$TOOLS/cache"
  export NEXT_TELEMETRY_DISABLED=1
  if [ ! -f "$TOOLS/cargo-about/.pegoles-stamp" ]; then say "build cargo-about $CARGO_ABOUT_VERSION (license notices)"; fi
  install_tool cargo-about "$CARGO_ABOUT_VERSION:$RUST_VERSION" build_cargo_about
  say "rustc $("$TOOLS/rust/bin/rustc" --version | cut -d' ' -f2), node $(node --version), pnpm $(pnpm --version)"
}

# --- build -------------------------------------------------------------------------------
build_app() {
  cd "$ROOT"
  say "JavaScript dependencies (lockfile, integrity-checked)"
  pnpm install --frozen-lockfile --ignore-scripts
  say "build Pegoles $VERSION (Rust app, VM helper, local model runtime); this takes a while"
  /bin/bash "$ROOT/scripts/package-macos.sh" app
}

# --- install -------------------------------------------------------------------------------
BUILT="$TARGET_DIR/release/bundle/macos/Pegoles Agent.app"

bundle_id() { /usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$1/Contents/Info.plist" 2>/dev/null || true; }

install_app() {
  [ -d "$BUILT" ] || die "the build did not produce $BUILT"
  [ "$(bundle_id "$BUILT")" = "$BUNDLE_ID" ] || die "built bundle has an unexpected identifier"

  local apps="$HOME/Applications"
  if [ ! -e "$apps" ]; then
    mkdir -m 0755 "$apps"
  fi
  local dir
  dir="$(cd -P "$apps" 2>/dev/null && pwd -P)" || die "$apps is not a usable directory"
  [ "$(stat -f %u "$dir")" = "$(id -u)" ] || die "$dir is not owned by $(id -un)"
  case "$(stat -f %Lp "$dir")" in
    *[2367]? | *[2367]) die "$dir is writable by other users" ;;
  esac
  local dest="$dir/$APP_NAME"

  if [ -L "$dest" ]; then
    die "$dest is a symbolic link; move it away and run the installer again"
  elif [ -e "$dest" ]; then
    [ -d "$dest" ] && [ "$(bundle_id "$dest")" = "$BUNDLE_ID" ] \
      || die "$dest exists and is not Pegoles; move it away and run the installer again"
    if ps -axo command= | grep -qF "$dest/Contents/"; then
      die "Pegoles is running from $dest; quit it and run the installer again"
    fi
  fi

  # Leftovers of an interrupted earlier install (exact names only).
  local old
  for old in "$dir"/.Pegoles.app.installing.* "$dir"/.Pegoles.app.previous.*; do
    [ -d "$old" ] || continue
    case "$(basename "$old")" in
      .Pegoles.app.installing.[0-9]* | .Pegoles.app.previous.[0-9]*) rm -rf "$old" ;;
    esac
  done

  local stage="$dir/.Pegoles.app.installing.$$" prev="$dir/.Pegoles.app.previous.$$"
  say "install $dest"
  ditto "$BUILT" "$stage"
  codesign --verify --deep --strict "$stage" || { rm -rf "$stage"; die "the staged app does not verify"; }
  if [ -e "$dest" ]; then
    mv "$dest" "$prev"
    if ! mv "$stage" "$dest"; then
      mv "$prev" "$dest"
      rm -rf "$stage"
      die "could not replace $dest (the previous version was restored)"
    fi
    rm -rf "$prev"
  else
    mv "$stage" "$dest"
  fi
  INSTALLED="$dest"
}

cleanup() {
  local status=$?
  [ -d "$LOCK" ] && [ "$(cat "$LOCK/pid" 2>/dev/null)" = "$$" ] && rm -rf "$LOCK"
  if [ "$status" != 0 ]; then
    echo "The installation did not complete; any installed Pegoles.app was left unchanged." >&2
  fi
}

main() {
  case "${1:-}" in
    -h | --help)
      sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed -e '$d' -e 's/^# \{0,1\}//'
      exit 0
      ;;
    "") ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
  preflight
  trap cleanup EXIT
  take_lock
  local started
  started="$(date +%s)"
  bootstrap
  build_app
  install_app
  local secs=$(($(date +%s) - started))
  say "done in $((secs / 60)) min $((secs % 60)) s"
  cat <<EOF

Pegoles $VERSION is installed at:
  $INSTALLED

Open it from ~/Applications, or run:
  open "$INSTALLED"

On first launch, choose "Set up computer" (downloads and verifies the
computer image) and set up Pegoles Local in Settings (downloads and
verifies the model). No API key is needed.

This checkout is no longer needed by the app. Its build cache (target/)
can be deleted to free space.
EOF
}

main "$@"
