#!/usr/bin/env bash
# Pegoles benchmark harness (Phase 3.6 §58-59): measurement, not gates.
# Records timings, memory, disk. No hard performance gates (hosted
# hardware varies); the report is for humans and future comparison.
#
# Prerequisites (real hardware): macOS arm64, helper built+signed,
# derived image Ready. Then:
#   PEGOLES_VM_HOST="$PWD/native/macos/pegoles-vm-host/.build/release/pegoles-vm-host" \
#     bash scripts/bench.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="${1:-/tmp/pegoles-bench.txt}"

if [[ -z "${PEGOLES_VM_HOST:-}" ]]; then
  echo "set PEGOLES_VM_HOST to the signed helper binary" >&2
  exit 2
fi

# Sample helper RSS in the background while the e2e runs.
SAMPLES=/tmp/pegoles-bench-rss.txt
: > "$SAMPLES"
(
  while true; do
    RSS=$(ps -ax -o pid=,rss=,command= | awk '/pegoles-vm-host$/ {print $2}' | head -1)
    if [[ -n "$RSS" ]]; then echo "$RSS" >> "$SAMPLES"; fi
    sleep 2
  done
) &
SAMPLER=$!
trap 'kill $SAMPLER 2>/dev/null || true' EXIT

{
  echo "Pegoles benchmark — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -m) $(sw_vers -productVersion 2>/dev/null || true)"
  echo
  PEGOLES_REAL_GUEST_TEST=1 cargo test -q -p pegoles-computer real_guest -- --nocapture 2>&1 \
    | grep -E "create took|start -> Running|Ready in|ping -> pong|system info|stop -> Stopped" || true
} | tee "$OUT"

kill $SAMPLER 2>/dev/null || true
echo "" >> "$OUT"
if [[ -s "$SAMPLES" ]]; then
  MAX_RSS=$(sort -n "$SAMPLES" | tail -1)
  echo "helper RSS max during e2e: $((MAX_RSS / 1024)) MiB (samples: $(wc -l < "$SAMPLES" | tr -d ' '))" | tee -a "$OUT"
else
  echo "helper RSS: no samples (helper ran too briefly to catch)" | tee -a "$OUT"
fi
echo "" >> "$OUT"
echo "disk usage:" | tee -a "$OUT"
du -sh ~/Library/Application\ Support/Pegoles/images/pegoles-base-0.1/ 2>/dev/null | tee -a "$OUT" || true
echo "" >> "$OUT"
echo "report: $OUT"
