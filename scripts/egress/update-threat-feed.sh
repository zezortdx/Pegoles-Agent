#!/usr/bin/env bash
# Refresh the compiled-in threat snapshot of pegoles-egress.
#
#   bash scripts/egress/update-threat-feed.sh
#
# Downloads the licence-compatible feeds listed in
# crates/pegoles-egress/data/README.md, normalizes (lowercase ASCII/punycode
# hostnames, no IPs, no query strings), dedups (an entry already covered by a
# listed parent domain is dropped), writes deterministic `gzip -9 -n` files
# to crates/pegoles-egress/data/, the manifest (date, counts, sizes) and the
# SHA-256 constants compiled into the binary
# (crates/pegoles-egress/src/threat/digests.rs), then prints the new digests.
# A snapshot change is a reviewed change: read the diff of manifest.txt and
# the counts, run `cargo test -p pegoles-egress`, release.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DATA="$ROOT/crates/pegoles-egress/data"
DIGESTS="$ROOT/crates/pegoles-egress/src/threat/digests.rs"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

MAX_TOTAL=$((8 * 1024 * 1024))

URLHAUS_TEXT="https://urlhaus.abuse.ch/downloads/text/"
URLHAUS_HOSTS="https://urlhaus.abuse.ch/downloads/hostfile/"
THREATFOX_DOMAINS="https://threatfox.abuse.ch/export/json/domains/recent/"
PHISHING_DOMAINS="https://raw.githubusercontent.com/mitchellkrogza/Phishing.Database/master/phishing-domains-ACTIVE.txt"
ADULT_HOSTS="https://raw.githubusercontent.com/StevenBlack/hosts/master/alternates/porn-only/hosts"
GAMBLING_HOSTS="https://raw.githubusercontent.com/StevenBlack/hosts/master/alternates/gambling-only/hosts"

fetch() { curl -fsSL --retry 3 --max-time 300 -o "$TMP/$1" "$2"; }

echo "==> downloading"
fetch urlhaus-text "$URLHAUS_TEXT"
fetch urlhaus-hosts "$URLHAUS_HOSTS"
fetch threatfox "$THREATFOX_DOMAINS"
fetch phishing "$PHISHING_DOMAINS"
fetch adult "$ADULT_HOSTS"
fetch gambling "$GAMBLING_HOSTS"

# --- normalization -----------------------------------------------------
# valid(h): lowercase LDH hostname, >= 2 labels, labels <= 63, total <= 253,
# last label has a letter (so no dotted-quad / numeric "TLD").
AWK_COMMON='
function valid(h,   n, a, i) {
  if (length(h) > 253 || h !~ /^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/) return 0
  n = split(h, a, ".")
  for (i = 1; i <= n; i++) if (length(a[i]) > 63) return 0
  if (a[n] !~ /[a-z]/) return 0
  return 1
}
function norm(h) { h = tolower(h); sub(/\.$/, "", h); return h }
'

# plain list / hosts file / ThreatFox JSON -> one hostname per line
hosts_to_domains() {
  awk "$AWK_COMMON"'
    /^[ \t]*#/ || /^[ \t]*$/ { next }
    { sub(/\r$/, ""); sub(/[ \t]*#.*/, "")
      h = ($1 ~ /^(0\.0\.0\.0|127\.0\.0\.1)$/) ? $2 : $1
      h = norm(h); if (h != "localhost" && valid(h)) print h }' "$1"
}
threatfox_to_domains() {
  grep -o '"ioc_value": *"[^"]*"' "$1" | sed -E 's/.*: *"([^"]*)"/\1/' |
    awk "$AWK_COMMON"'{ h = norm($0); if (valid(h)) print h }'
}
# Drop entries whose parent domain is also listed; sorted unique output.
dedup_file() {
  LC_ALL=C sort -u > "$1.sorted"
  awk '
    NR == FNR { set[$0] = 1; next }
    { h = $0; skip = 0
      while ((i = index(h, ".")) > 0) {
        h = substr(h, i + 1)
        if (index(h, ".") == 0) break
        if (h in set) { skip = 1; break }
      }
      if (!skip) print }' "$1.sorted" "$1.sorted"
}

echo "==> normalizing"
{
  hosts_to_domains "$TMP/urlhaus-hosts"
  threatfox_to_domains "$TMP/threatfox"
} | dedup_file "$TMP/malware-domains" > "$TMP/malware-domains.txt"

hosts_to_domains "$TMP/phishing" | dedup_file "$TMP/phishing-domains" > "$TMP/phishing-domains.txt"
hosts_to_domains "$TMP/adult" | dedup_file "$TMP/adult-domains" > "$TMP/adult-domains.txt"
hosts_to_domains "$TMP/gambling" | dedup_file "$TMP/gambling-domains" > "$TMP/gambling-domains.txt"

# URLhaus URLs -> "host/path" (scheme, userinfo-less, port, query, fragment
# removed; IP hosts dropped because IP literals are denied anyway; the path
# is kept verbatim and case-sensitive).
awk "$AWK_COMMON"'
  /^#/ || /^[ \t]*$/ { next }
  { sub(/\r$/, ""); u = $0
    if (!match(u, /^[A-Za-z][A-Za-z0-9+.-]*:\/\//)) next
    u = substr(u, RLENGTH + 1)
    e = match(u, /[\/?#]/)
    if (e == 0) { host = u; path = "/" } else { host = substr(u, 1, e - 1); path = substr(u, e) }
    if (host ~ /@/) next
    sub(/:[0-9]+$/, "", host)
    host = norm(host); if (!valid(host)) next
    sub(/[?#].*/, "", path); if (path == "") path = "/"
    if (path ~ /[ \t]/) next
    print host path }' "$TMP/urlhaus-text" | LC_ALL=C sort -u > "$TMP/malware-urls.txt"

# --- sanity: a failed/empty download must never replace the snapshot ------
check_min() {
  local n
  n=$(wc -l < "$TMP/$1.txt" | tr -d ' ')
  if [ "$n" -lt "$2" ]; then echo "refusing: $1 has only $n entries (< $2)" >&2; exit 1; fi
}
check_min malware-domains 100
check_min malware-urls 1000
check_min phishing-domains 10000
check_min adult-domains 5000
check_min gambling-domains 500

echo "==> writing $DATA"
mkdir -p "$DATA" "$(dirname "$DIGESTS")"
NAMES=(malware-domains malware-urls phishing-domains adult-domains gambling-domains)
total=0
for n in "${NAMES[@]}"; do
  gzip -9 -n -c "$TMP/$n.txt" > "$DATA/$n.txt.gz"
  total=$((total + $(wc -c < "$DATA/$n.txt.gz")))
done
if [ "$total" -gt "$MAX_TOTAL" ]; then echo "refusing: vendored data is $total bytes (> $MAX_TOTAL)" >&2; exit 1; fi

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
  echo "# generated by scripts/egress/update-threat-feed.sh; do not edit"
  echo "generated_utc $NOW"
  echo "total_gz_bytes $total"
  echo "source_malware_urls $URLHAUS_TEXT"
  echo "source_malware_domains $URLHAUS_HOSTS $THREATFOX_DOMAINS"
  echo "source_phishing_domains $PHISHING_DOMAINS"
  echo "source_adult_domains $ADULT_HOSTS"
  echo "source_gambling_domains $GAMBLING_HOSTS"
  echo "# name entries gz_bytes sha256(gz)"
  for n in "${NAMES[@]}"; do
    echo "$n $(wc -l < "$TMP/$n.txt" | tr -d ' ') $(wc -c < "$DATA/$n.txt.gz" | tr -d ' ') $(sha "$DATA/$n.txt.gz")"
  done
} > "$DATA/manifest.txt"

{
  echo "// Generated by scripts/egress/update-threat-feed.sh; do not edit."
  echo "// SHA-256 of each compiled-in snapshot file (data/*.txt.gz)."
  for n in "${NAMES[@]}"; do
    up="$(echo "$n" | tr 'a-z-' 'A-Z_')"
    echo "pub(super) const ${up}_SHA256: &str = \"$(sha "$DATA/$n.txt.gz")\";"
  done
} > "$DIGESTS"

echo "==> done ($total bytes gz)"
cat "$DATA/manifest.txt"
