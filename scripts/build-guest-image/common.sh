#!/usr/bin/env bash
# Shared by the build scripts in this directory; sourced, never run.
# BUILD TIME ONLY.

# Container images pinned by digest (the tag is informational only): a
# moved or poisoned tag cannot change the toolchain that builds the guest
# runtime that is later sealed into the image. Bump deliberately:
#   docker buildx imagetools inspect <image>:<tag>   (use the index digest)
RUST_IMAGE="rust:1.89-bookworm@sha256:948f9b08a66e7fe01b03a98ef1c7568292e07ec2e4fe90d88c07bb14563c84ff"
DEBIAN_IMAGE="debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a"
export RUST_IMAGE DEBIAN_IMAGE

# out_dir [path]: print the directory a build step writes its products to.
# No path: a fresh private directory (mktemp -d, mode 0700); its path is
# printed so it can be handed to the next step. An explicit path is created
# 0700 when missing and refused when it is a symlink, not owned by the
# current user, or writable by group/other: a predictable shared path
# (e.g. /tmp/pgbuild) can be pre-created by another local account to swap
# a binary between the build and the seal.
out_dir() {
  local dir="${1:-}" base="${TMPDIR:-/tmp}"
  if [ -z "$dir" ]; then
    mktemp -d "${base%/}/pegoles-build.XXXXXX"
    return
  fi
  if [ -L "$dir" ]; then
    echo "refusing output dir that is a symlink: $dir" >&2
    return 1
  fi
  # shellcheck disable=SC2174 # only the output dir itself needs 0700
  mkdir -p -m 0700 "$dir"
  if [ ! -O "$dir" ]; then
    echo "refusing output dir not owned by $(id -un): $dir" >&2
    return 1
  fi
  if [ -n "$(find "$dir" -maxdepth 0 \( -perm -g+w -o -perm -o+w \) -print)" ]; then
    echo "refusing group/other-writable output dir: $dir" >&2
    return 1
  fi
  (cd "$dir" && pwd -P)
}
