# Threat snapshot (always-deny layer)

Compiled into `pegoles-egress` with `include_bytes!` and verified at startup
against SHA-256 constants (`src/threat/digests.rs`); a mismatch makes egress
refuse to start. Regenerate only with
`bash scripts/egress/update-threat-feed.sh`, review the diff of
`manifest.txt`, run `cargo test -p pegoles-egress`, release. Files are
deterministic `gzip -9 -n` of one normalized entry per line (lowercase
ASCII/punycode hostnames, no IPs, no query strings, entries covered by a
listed parent domain dropped). Total vendored size: about 3.9 MB (limit 8 MiB).

| File | Source | Licence | Used in |
|---|---|---|---|
| `malware-domains.txt.gz` | abuse.ch URLhaus host file + ThreatFox domain IOCs (last 7 days) | CC0 1.0 (abuse.ch terms) | both modes |
| `malware-urls.txt.gz` | abuse.ch URLhaus URL list, as `host/path` (IP hosts dropped, query removed) | CC0 1.0 | both modes |
| `phishing-domains.txt.gz` | mitchellkrogza/Phishing.Database `phishing-domains-ACTIVE.txt` | MIT | both modes |
| `adult-domains.txt.gz` | StevenBlack/hosts `alternates/porn-only` | MIT | `open_web` only |
| `gambling-domains.txt.gz` | StevenBlack/hosts `alternates/gambling-only` | MIT | `open_web` only |

The project is MIT (`LICENSE`); CC0 and MIT data are compatible with it.
StevenBlack/hosts is itself a merge of community lists (see that repository's
`readme.md` and `data/` for the upstream sources); the merged files are
distributed by it under MIT. Keep the MIT notices for the two MIT sources in
the release's third-party notices.

The phishing *links* list (Phishing.Database `phishing-links-ACTIVE.txt`, about
60 MB) is not vendored: it does not fit the size budget, and its hosts are
already covered by the domain list.

## Current snapshot

Generated 2026-09-30T16:56:23Z (see `manifest.txt` for the source URLs).

| File | Entries | gz bytes | SHA-256 of the gz file |
|---|---:|---:|---|
| `malware-domains.txt.gz` | 1144 | 10126 | `8ef35313e40f10b5de0e5112a75cc7a0e832d4576de3084d5c40a0b55a2bb8e2` |
| `malware-urls.txt.gz` | 15457 | 354214 | `e6b776c8f6d211db4c3dda60095997a951c187adccc4a83d2ecbe571ece96970` |
| `phishing-domains.txt.gz` | 371300 | 3343372 | `3c97f7d2aa258186f89539f7193a2756e3083706aba5ad03de11fe149d5cf7e2` |
| `adult-domains.txt.gz` | 47658 | 228135 | `734892b9a97e49f970d06d1516b3ec6936bfb366dcc0fe0a76d280dcdee1d429` |
| `gambling-domains.txt.gz` | 4330 | 18225 | `76d98bba5e8369f37acc4917e398e0db35041883cd6ba5f73c23d9599afd8f7a` |

## Matching

Domain lists match the host and every parent domain with at least two labels
(`a.b.evil.example` is caught by `evil.example`). URLs match exactly on
`host/path` (no parent hosts, no query). Lookups hash the name with SHA-256
(first 8 bytes) into a sorted `Vec<u64>` and binary-search it: O(log n), about
3.4 MB of memory for the whole snapshot.

## Known limits

- ThreatFox is fetched as its "recent" export (7 days); URLhaus and the
  phishing list carry only online/active entries. The snapshot ages until the
  next release.
- Feeds contain false positives; there is deliberately no runtime override
  (the always-deny layer is part of the immutable protocol).
