# Release gates: 0.1.0-rc.1

Every gate for the first public release, with its status and the evidence
behind it. Status is one of **PASS**, **FAIL**, **BLOCKED** (cannot be run
without something external, named in the row), or **NOT TESTED**. Nothing
is marked PASS without evidence from this repository or a run on this
machine (M4 Pro, 24 GB, macOS 26.5.2, 2026-09-26/27). Commands are in
`CLAUDE.md` and the files named below.

**Verdict: not ready for a public release.** The engineering gates pass;
the distribution gates (Developer ID signing, notarization, Gatekeeper on
a quarantined download, a clean-machine install) are blocked on
credentials and a GitHub repository that do not exist yet. This tree is a
release candidate to be signed and published by the release workflow once
they do.

## P0 gates

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | Real Pegoles Local E2E (no API key, product path, real VM) | PASS (2 of 3 runs) | `apps/desktop/src-tauri/examples/local_e2e.rs`, release build, bundled-runtime layout: task completed and verified in the guest (38.7 s, 53.0 s); Stop 2.2 s; worker `kill -9` mid-task → recovered, task completed; teardown leaves no VM or worker; 0 internet sockets. Run 1 of 3: the 2B model returned 3 invalid replies on the first screen and the task stopped as designed (model quality, not a crash). |
| 2 | Real packaged app | PASS (ad-hoc) / NOT TESTED (UI flow) | `scripts/package-macos.sh` → `Pegoles Agent.app` + 170 MB DMG; `verify-artifact.sh` passes (62 Mach-O signed, exact entitlements, runtime, worker, notices, no developer paths); the packaged app launches and quits cleanly with no helper, VM or worker left. The UI was not driven end to end in the packaged app (the Mac was in use; a live visual pass is still to do). |
| 3 | Local MLX runtime bundled | PASS | `scripts/local-model/build-runtime.sh`: pinned CPython 3.12.14 (python-build-standalone 20260924, SHA-256) + 33 hash-locked wheels, no pip at run time; two builds into different directories give the same tree digest `304eab88…`; both offered models run real inference with it under the sandbox; the bundle carries it in `Contents/Resources/runtime`. |
| 4 | Guest image distribution verified | PASS (mechanism) / BLOCKED (hosting) | `crates/pegoles-computer/src/image_release.rs` + `catalog/images.json`; 14 unit tests (tampered, oversized, other-bytes, corrupt, resume, mirror fallback, cancel, tampered install, symlinked disk, unpublished URL) + pin enforcement test; the real 562 MB archive installs through the product installer in 11.2 s, byte-identical to the sealed disk. Download from a real host: blocked until the archive is uploaded to the GitHub repository (catalog URL still says UNPUBLISHED; the release workflow refuses to build until it is replaced). |
| 5 | No critical/high security defect unresolved | PASS | Round 1: 7 independent adversarial reviewers, every medium+ finding checked by two verifiers (trace, refute): 71 findings, 0 critical, the one high (a guest hello-flood crashing the app) fixed with a regression test, all verified mediums fixed. Round 2, after the fixes: 4 red-team attackers on the final tree (host/VM, AI/policy, web/Tauri, supply chain): 0 critical, 0 high, one confirmed medium (webview network egress via preconnect/WebRTC) fixed and proven with a probe; lows fixed or recorded with the accepted residuals in `docs/THREAT_MODEL.md`. |
| 6 | Clean dependency scans | PASS | `cargo audit`: 0 vulnerabilities (warnings: unmaintained `proc-macro-error` and `unic-*` via Tauri build tooling, unsound `glib` in the Linux-only GTK tree, none in the macOS app); `cargo deny check`: advisories, bans (wildcards denied), licenses, sources ok; `pnpm audit` and `pnpm audit --prod`: no known vulnerabilities. |
| 7 | CI green | NOT TESTED (no GitHub repository) | Workflows pass `actionlint` and the repository's hygiene script; every step they run is green locally (`bash scripts/check.sh`: 549 Rust tests, 253 frontend tests, production-bundle check; release-profile tests of policy, inference and computer); the Linux portability job's crates pass clippy and tests in a `rust:1.97.1` container; `cargo deny` and `cargo audit` pass. |
| 8 | Developer ID signing valid | BLOCKED | No Developer ID Application certificate on this Mac (`security find-identity -v -p codesigning` → 0 identities). The ad-hoc build is not distributable. |
| 9 | Hardened runtime valid | BLOCKED (partly verified) | App and helper are signed with the hardened runtime (flags `runtime`) even ad-hoc; the bundled interpreter can only use it with a Team ID (ad-hoc library validation refuses its own libraries: verified), so the full check needs the Developer ID build (`verify-artifact.sh --signed`). |
| 10 | Notarization accepted | BLOCKED | Needs the Developer ID build and App Store Connect notary credentials (`scripts/release/notarize.sh`). |
| 11 | Gatekeeper accepts the quarantined downloaded artifact | BLOCKED | `spctl --assess` rejects the ad-hoc build, as it must. Needs the notarized DMG downloaded through a browser. |
| 12 | Release artifact hashes generated | PASS (script) | `scripts/release/provenance.sh` on the ad-hoc DMG: `SHA256SUMS`, `build-manifest.json` (commit, toolchains, runtime tree digest, image pin, model revision), CycloneDX SBOM (816 components). The release workflow runs it in the build and sign jobs. |
| 13 | No secret exposure | PASS | `gitleaks git --log-opts=--all` over every ref: no leaks; working-tree scan: only third-party strings inside `target/` build caches; `.gitignore` covers keys, certificates, provisioning profiles, keychains, `.env`; the bundle verifier refuses keys, weights, venvs; the published guest image has no SSH host keys or authorized keys (`sanitize-image.sh`). |
| 14 | Git tree clean and recoverable | PASS | All work is committed on `release/v0.1.0` (and the pre-phase working tree as 4 commits on `main`); backup refs kept. |
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
| CodeQL | NOT TESTED | `codeql.yml` runs on GitHub (actions, JS/TS, Python, Rust, Swift) |
| Long-run capture soak | PASS (with one transient error) | `capture_soak`: 2212 captures, guest memory flat (1256 → 1292 MiB); one capture timed out after 5 s in the guest compositor (reported, not a leak) |
| Lifecycle / resource soak | PASS | `release_soak`: 12 VM lifecycles, 1200 observations, 60 inferences with 2 forced worker kills; nothing left after each destroy; descriptors flat; host footprint falling |
| Crash recovery | PASS | guest runtime killed → ready in 2.9 s (`agent_e2e`); worker killed mid-task → task completed (`local_e2e`); packaged app quit → no helper/VM/worker left |

## Supply chain and provenance

| Item | Status | Evidence |
|---|---|---|
| Model pins | PASS | `crates/pegoles-inference/catalog/models.json`: MAI-UI-2B 6-bit `mlx-community/MAI-UI-2B-6bit-v2` at revision `cb57cf2f…`, SHA-256 per file |
| Guest image pins | PASS | `catalog/images.json`: archive SHA-256 `a938365c…` (561,846,260 bytes), disk SHA-512 `766188e5…` (3 GiB); package manifest `scripts/build-guest-image/manifests/pegoles-base-0.3.packages.txt` (511 packages) |
| Runtime provenance | PASS | `runtime-manifest.json` in the bundle (interpreter release + digest, lock digest, package list, tree digest) |
| Actions pinned to commit SHAs | PASS | every `uses:` in `.github/workflows/*` (hygiene script enforces) |
| Release secrets only in the protected sign job | PASS (by construction) / NOT TESTED (on GitHub) | `release.yml`: `build` has no secrets; `sign` (environment `release`) runs only first-party scripts and Apple tools, takes only named files, checks the worker/lock/manifest against the checkout; `publish` uploads and attests an exact asset list; PR workflows have none |
| SBOM | PASS (script) | CycloneDX via syft (816 components) |
| Build provenance attestation | NOT TESTED | `actions/attest-build-provenance` in `release.yml` (needs a public GitHub repository) |
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
2. **GitHub repository.** Decide owner/name and visibility; create it;
   configure the settings in the checklist §3; upload
   `pegoles-base-0.3-arm64.raw.gz` to a `guest-image-0.3` release and
   replace `UNPUBLISHED` in `crates/pegoles-computer/catalog/images.json`.
3. **Release run.** Tag `v0.1.0-rc.1` on `main` after the PR merges;
   approve the `release` deployment; then run the draft's checks and a
   clean-machine install (checklist §7) before publishing.
