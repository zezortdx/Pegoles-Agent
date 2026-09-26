# Release gates: 0.1.0-rc.1

Every gate for the first public release, with its status and the evidence
behind it. Status is one of **PASS**, **FAIL**, **BLOCKED** (cannot be run
without something external, named in the row), or **NOT TESTED**. Nothing
is marked PASS without evidence from this repository or a run on this
machine (M4 Pro, 24 GB, macOS 26.5.2, 2026-09-26/27). Commands are in
`CLAUDE.md` and the files named below.

**Verdict: not ready for a public release.** The engineering gates pass;
the distribution gates (Developer ID signing, notarization, Gatekeeper on
a quarantined download, a clean-machine install) are blocked on Apple
credentials that do not exist yet, and on the GitHub repository
(`zezortdx/Pegoles-Agent`, private on GitHub Free) becoming public. This
tree is a release candidate to be signed and published by the release
workflow once they are in place.

## P0 gates

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | Real Pegoles Local E2E (no API key, product path, real VM) | PASS (3 of 5 runs, incl. the last run on the final code) | `apps/desktop/src-tauri/examples/local_e2e.rs`, release build, bundled-runtime layout: the task completed and was verified in the guest (38.7 s, 53.0 s, and 36.1 s on the final code); Stop 2.2–2.5 s; worker `kill -9` mid-task → recovered, task completed; teardown leaves no VM or worker; 0 internet sockets. The 2 failed runs were the 2B model's own mistakes, stopped by the deterministic brakes: 3 invalid replies in a row on the first screen, and a mistyped command (`cd ~touch hello.txt`) followed by a loop the loop brake ended. `agent_e2e` (scripted planner, same product path) passes on the final code. |
| 2 | Real packaged app | PASS (ad-hoc) / NOT TESTED (UI flow) | `scripts/package-macos.sh` → `Pegoles Agent.app` + 170 MB DMG; `verify-artifact.sh` passes (62 Mach-O signed, exact entitlements, runtime, worker, notices, no developer paths); the packaged app launches and quits cleanly with no helper, VM or worker left. The UI was not driven end to end in the packaged app (the Mac was in use; a live visual pass is still to do). |
| 3 | Local MLX runtime bundled | PASS | `scripts/local-model/build-runtime.sh`: pinned CPython 3.12.14 (python-build-standalone 20260924, SHA-256) + 33 hash-locked wheels, no pip at run time; two builds into different directories give the same tree digest `304eab88…`; both offered models run real inference with it under the sandbox; the bundle carries it in `Contents/Resources/runtime`. |
| 4 | Guest image distribution verified | PASS (mechanism) / BLOCKED (hosting) | `crates/pegoles-computer/src/image_release.rs` + `catalog/images.json`; 14 unit tests (tampered, oversized, other-bytes, corrupt, resume, mirror fallback, cancel, tampered install, symlinked disk, unpublished URL) + pin enforcement test; the real 562 MB archive installs through the product installer in 11.2 s, byte-identical to the sealed disk. Download from a real host: blocked until the archive is uploaded to the GitHub repository (catalog URL still says UNPUBLISHED; the release workflow refuses to build until it is replaced). |
| 5 | No critical/high security defect unresolved | PASS | Round 1: 7 independent adversarial reviewers, every medium+ finding checked by two verifiers (trace, refute): 71 findings, 0 critical, the one high (a guest hello-flood crashing the app) fixed with a regression test, all verified mediums fixed. Round 2, after the fixes: 4 red-team attackers on the final tree (host/VM, AI/policy, web/Tauri, supply chain): 0 critical, 0 high, one confirmed medium (webview network egress via preconnect/WebRTC) fixed and proven with a probe; lows fixed or recorded with the accepted residuals in `docs/THREAT_MODEL.md`. |
| 6 | Clean dependency scans | PASS | `cargo audit`: 0 vulnerabilities (warnings: unmaintained `proc-macro-error` and `unic-*` via Tauri build tooling, unsound `glib` in the Linux-only GTK tree, none in the macOS app); `cargo deny check`: advisories, bans (wildcards denied), licenses, sources ok; `pnpm audit` and `pnpm audit --prod`: no known vulnerabilities. |
| 7 | CI green | PASS (GitHub) | PR #1 on `zezortdx/Pegoles-Agent`, head `0744f6b`: every `ci.yml` job green on GitHub-hosted runners: Rust on macOS arm64 (fmt, clippy `-D warnings`, 549 tests), release-profile tests and release build, Swift helper, frontend (lint, typecheck, tests, build), guest aarch64 check, portability on ubuntu-24.04 (505 tests incl. 23 guest-runtime tests on x86_64) and windows-2025, supply chain (cargo-deny, cargo-audit, pnpm audit, lockfile rules), hygiene (script, actionlint, shellcheck), gitleaks over the full history. The first run found one real defect, a guest-runtime `memfd_create` call that only type-checked where `c_char` is unsigned (aarch64); fixed in `0744f6b`. Dependency review is skipped while the repository is private (unsupported there). |
| 8 | Developer ID signing valid | BLOCKED | No Developer ID Application certificate on this Mac (`security find-identity -v -p codesigning` → 0 identities). The ad-hoc build is not distributable. |
| 9 | Hardened runtime valid | BLOCKED (partly verified) | App and helper are signed with the hardened runtime (flags `runtime`) even ad-hoc; the bundled interpreter can only use it with a Team ID (ad-hoc library validation refuses its own libraries: verified), so the full check needs the Developer ID build (`verify-artifact.sh --signed`). |
| 10 | Notarization accepted | BLOCKED | Needs the Developer ID build and App Store Connect notary credentials (`scripts/release/notarize.sh`). |
| 11 | Gatekeeper accepts the quarantined downloaded artifact | BLOCKED | `spctl --assess` rejects the ad-hoc build, as it must. Needs the notarized DMG downloaded through a browser. |
| 12 | Release artifact hashes generated | PASS (script) | `scripts/release/provenance.sh` on the ad-hoc DMG: `SHA256SUMS`, `build-manifest.json` (commit, toolchains, runtime tree digest, image pin, model revision), CycloneDX SBOM (816 components). The release workflow runs it in the build and sign jobs. |
| 13 | No secret exposure | PASS | Before the first push: gitleaks 8.30.1 over every commit of `main` and `release/v0.1.0` found only four policy tripwire fixtures (truncated key headers typed at the policy in tests), listed by exact fingerprint in `.gitleaksignore`; a scan of every blob in that history for personal paths, hostnames, emails, tokens and private artifacts found stray `__pycache__` bytecode with an absolute home path, purged from history with `git filter-repo` (every other commit verified unchanged). The `secrets` CI job repeats the full-history scan on every run. Working-tree scan: only third-party strings inside `target/` build caches. `.gitignore` covers keys, certificates, provisioning profiles, keychains, `.env`; the bundle verifier refuses keys, weights, venvs; the published guest image has no SSH host keys or authorized keys (`sanitize-image.sh`). |
| 14 | Git tree clean and recoverable | PASS | All work is committed and pushed to `zezortdx/Pegoles-Agent` (`main`, `release/v0.1.0`, nothing else); local backup refs (including the pre-purge history) are kept and never pushed. |
| 15 | README matches reality | PASS | Rewritten against the verified state; claims limited to what was run on this Mac. |

## Security regression gates

| Gate | Status | Evidence |
|---|---|---|
| Policy hostile-model regression | PASS | `crates/pegoles-policy/tests` escape matrix + property tests; `crates/pegoles-agent/tests/security_matrix.rs` (prompt-injected planner through the real executor and policy) |
| Model output parser fails closed | PASS | `local/parse.rs` tests + property tests (never panics, bounded) |
| Guest malformed/flooding IPC | PASS | `pegoles-computer` tests incl. the 20k-hello flood on a 2 MiB stack (it crashed the app before the fix); bounded helper lines and bytes; handshake per connection |
| MLX worker isolation | PASS | `worker::tests::sandbox_blocks_escapes`: 17 probes (fork, exec, TCP, DNS socket, home read/write, shared temp, LaunchServices, lsd, Apple Events, pasteboard, Keychain ×2, WindowServer, cfprefsd, signal, process inspection) succeed unsandboxed and fail sandboxed with the real runtime |
| Model hash verification | PASS | `pegoles-inference` store tests; re-verification before every (re)load |
| Guest image hash verification | PASS | gate 4 |
| Webview network containment | PASS | `examples/webview_egress_probe.rs` against the real page and release CSP: uncontained control reaches local listeners (preconnect TCP, navigations, WebRTC UDP); contained reaches nothing (0 TCP, 0 UDP) while IPC works; the packaged release app starts with it in force |
| CSP / XSS / IPC surface | PASS | `apps/desktop/src/security/*.test.*` (inert rendering of hostile strings, no HTML sinks, product calls only `api`), `src-tauri/src/ipc_acl.rs` (capability equals the invoked set through Tauri's ACL) |
| Native consent for cloud planner and key | PASS (logic) / NOT TESTED (visual) | `consent.rs` tests with an injected decider and the decline backoff; the AppKit alert itself was not clicked through in this phase |
| cargo audit / cargo deny / pnpm audit | PASS | gate 6 |
| CodeQL | PASS (GitHub, private mode) | `codeql.yml` with `security-extended` on PR #1: actions, JavaScript/TypeScript, Python, Rust (no build) and Swift (traced build of the helper) analyzed; 0 results in every language. While private, results cannot be uploaded to code scanning: the SARIF is kept as a run artifact and `.github/scripts/codeql-gate.py` fails on high/critical or error-level alerts |
| Long-run capture soak | PASS (with one transient error) | `capture_soak`: 2212 captures, guest memory flat (1256 → 1292 MiB); one capture timed out after 5 s in the guest compositor (reported, not a leak) |
| Lifecycle / resource soak | PASS | `release_soak`: 12 VM lifecycles, 1200 observations, 60 inferences with 2 forced worker kills; nothing left after each destroy; descriptors flat; host footprint falling |
| Crash recovery | PASS | guest runtime killed → ready in 2.9 s (`agent_e2e`); worker killed mid-task → task completed (`local_e2e`); packaged app quit → no helper/VM/worker left |

## Supply chain and provenance

| Item | Status | Evidence |
|---|---|---|
| Model pins | PASS | `crates/pegoles-inference/catalog/models.json`: MAI-UI-2B 6-bit `mlx-community/MAI-UI-2B-6bit-v2` at revision `cb57cf2f…`, SHA-256 per file |
| Guest image pins | PASS | `catalog/images.json`: archive SHA-256 `a938365c…` (561,846,260 bytes), disk SHA-512 `766188e5…` (3 GiB); package manifest `scripts/build-guest-image/manifests/pegoles-base-0.3.packages.txt` (511 packages) |
| Runtime provenance | PASS | `runtime-manifest.json` in the bundle (interpreter release + digest, lock digest, package list, tree digest) |
| Actions pinned to commit SHAs | PASS | every `uses:` in `.github/workflows/*` (hygiene script enforces); the repository also requires full-SHA pins and allows only GitHub-created actions plus `dtolnay/rust-toolchain` and `pnpm/action-setup` |
| Release secrets only in the protected sign job | PASS (by construction) / BLOCKED (plan: environment reviewers) | `release.yml`: `build` has no secrets; `sign` (environment `release`) runs only first-party scripts and Apple tools, takes only named files, checks the worker/lock/manifest against the checkout; `publish` uploads and attests an exact asset list; PR workflows have none ; on GitHub the `release` environment exists with a single `v*` tag rule, no administrator bypass and no secrets; required reviewers are refused on a private GitHub Free repository, so no secret may be added before the switch to public |
| SBOM | PASS (script) | CycloneDX via syft (816 components) |
| Build provenance attestation | BLOCKED (plan) | `actions/attest-build-provenance` in `release.yml`; artifact attestations need a public repository on GitHub Free |
| Third-party licenses | PASS | `THIRD_PARTY_NOTICES.md` in the bundle (Python runtime incl. OpenSSL 3.5.8, SQLite, mpdecimal, bzip2, liblzma, libffi; frontend; 240 Rust crates) |
| Updater | N/A | No auto-updater ships in 0.1 (no signing key needed) |

## Hardware actually tested

One Mac: Apple M4 Pro, 24 GB, macOS 26.5.2 (build 25F84). Nothing was run
on 8 GB or 16 GB machines, on other chips, or on older macOS; the app
declares macOS 14.0 as its minimum without having been tested there.

## Blockers and the exact actions they need

1. **Developer ID.** Join the Apple Developer Program (if not already),
   create a "Developer ID Application" certificate, and either install it
   in this Mac's login keychain (local signing) or export it as .p12 for
   the `release` environment secrets. Create an App Store Connect API key
   for notarization. Steps: `docs/GITHUB_RELEASE_CHECKLIST.md` §4.
2. **GitHub repository.** `zezortdx/Pegoles-Agent` exists (private) with
   every setting GitHub Free allows on a private repository (checklist §3).
   Still to do, all after the switch to public: the `main` and tag
   rulesets, required reviewers on the `release` environment, secret
   scanning with push protection, private vulnerability reporting, fork PR
   approval; then upload `pegoles-base-0.3-arm64.raw.gz` to a
   `guest-image-0.3` release and replace `UNPUBLISHED` in
   `crates/pegoles-computer/catalog/images.json`. (GitHub Pro would allow
   rulesets while private; required reviewers on a private repository's
   environment need GitHub Enterprise.)
3. **Release run.** Tag `v0.1.0-rc.1` on `main` after the PR merges;
   approve the `release` deployment; then run the draft's checks and a
   clean-machine install (checklist §7) before publishing.
