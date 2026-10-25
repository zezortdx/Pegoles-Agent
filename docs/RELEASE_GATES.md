# Release gates: 0.1.0 (source-first)

Every gate for the first public release, with its status and the evidence
behind it. 0.1.0 is a **source-first** release: users build and install
with `./scripts/install.sh`; no binary is published. Status is one of
**PASS**, **FAIL** or **DEFERRED** (not a source-first requirement).
Nothing is marked PASS without evidence from this repository, GitHub, or a
run on the test Mac (Apple M4 Pro, 24 GB, macOS 26.5.2, 2026-09-26/27).
The final acceptance run used the release commit `58616be` (tag `v0.1.0`),
cloned anonymously from the public repository, with fresh app data.

## Source-first P0 gates

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | Secrets and history scan | PASS | Before the first push, and again after rewriting commit emails to the GitHub noreply address: gitleaks 8.30.1 over every commit of `main` and `release/v0.1.0` finds only four policy tripwire fixtures (truncated key headers in tests), listed by exact fingerprint in `.gitleaksignore`. A scan of every blob found stray `__pycache__` bytecode with an absolute home path, purged with `git filter-repo`, and nothing else (no tokens, keys, personal paths or private artifacts). The `secrets` CI job rescans the full history on every run. |
| 2 | License | PASS | MIT (`LICENSE`, every crate's `license = "MIT"`); the built app carries `THIRD_PARTY_NOTICES.md` for everything it redistributes. |
| 3 | Clean Git state | PASS | Everything is committed and pushed. `v0.1.0` is an annotated tag on `58616be`, the head of `main` when it was created; `guest-image-0.3` holds the image. Besides `main`, GitHub has only `release/v0.1.0` (already contained in `main`) and Dependabot's branches. |
| 4 | CI | PASS | Public repository, GitHub-hosted runners only: every `ci.yml` job green on the release commit (PR #10 and its push to `main`): Rust on macOS arm64 (fmt, clippy `-D warnings`, tests), release profile, Swift helper, frontend, guest aarch64, portability on Ubuntu and Windows, supply chain, hygiene, full-history gitleaks; `dependency-review.yml` green on the PR. The `main` ruleset requires all 16 checks. |
| 5 | CodeQL | PASS | `security-extended`, 5 languages (actions, JS/TS, Python, Rust, Swift), uploaded to code scanning for the release commit: 0 open alerts. The `main` ruleset requires CodeQL results (high or higher). Secret scanning with push protection: 0 alerts. |
| 6 | Dependency audits | PASS | `cargo deny` (advisories, bans, licenses, sources), `cargo audit` (0 vulnerabilities), `pnpm audit --prod` (0), lockfile and wheel-hash rules, in CI. One Dependabot alert (glib 0.18 unsoundness) is dismissed as not used: it is only in Tauri's Linux GTK tree, absent from the macOS dependency graph (`cargo tree -i glib --target aarch64-apple-darwin` is empty; same reasoning as `deny.toml`). Dependabot's major-version upgrade PRs are deferred past 0.1.0. |
| 7 | `install.sh` security | PASS | Adversarial review by an independent reviewer; valid findings fixed (cargo applies `.cargo/config.toml` from every directory above the build, so configurations another user could have planted are refused; the clean-environment marker is no longer trusted alone; atomic stale-lock takeover; glob characters in paths refused; packaging refuses to strip quarantine). The review also found verification pipelines that could pass on a match under `pipefail`; fixed everywhere, and the shipped app and published image re-verified. Tested: paths with spaces, a fresh HOME, a symlinked invocation from a hostile working directory with a hostile environment (PATH shims, `RUSTFLAGS`, `CARGO_TARGET_DIR`, `NODE_OPTIONS`, `.npmrc`, a cargo `rustc-wrapper`: none took effect), SIGINT and SIGTERM mid-build (lock released, previous app intact, no orphans), a tampered checksum and a failed download (refused, no partial files), reruns, replacing an installation, refusing a running app, a foreign `Pegoles.app`, a symlinked destination and a writable `~/Applications`, a symlinked `~/Applications`, a quarantined checkout, injection-prone checkout paths, a Rosetta shell, too little disk space. |
| 8 | Clean clone | PASS | A fresh `git clone` from GitHub (no `node_modules`, no `target`, no venv, no model, no image) into a path with a space: `./scripts/install.sh` → `~/Applications/Pegoles.app` in 4 min 38 s on the first run (tools, build, runtime, signing, verification). Final run: an anonymous clone of `58616be` (no `target`, no `node_modules`), installed as documented in 5 min 13 s, every `verify-artifact.sh` check passing. |
| 9 | Local build | PASS | `verify-artifact.sh` on every installer build: strict ad-hoc signature, 62 Mach-O files signed, the helper carries only the virtualization entitlement, the app and interpreter none, runtime/worker/lock match the checkout, no weights, keys, venvs or build paths. |
| 10 | Installed app independent of the repo | PASS | With the clone moved away, the installed app launched, set up Pegoles Local and ran a task; the helper and worker ran from inside the bundle; no string in the binaries or bundle references the checkout, `target/`, `node_modules`, a venv, `/tmp` or the build user's home. |
| 11 | Model real download + verification | PASS | From the installed app, keyless: MAI-UI-2B 6-bit from `huggingface.co/mlx-community/MAI-UI-2B-6bit-v2` at the pinned revision `cb57cf2f…`; quitting mid-download kept the partial file at 939,524,096 bytes and the relaunched app resumed from there; installed atomically; all 13 files match their pinned SHA-256 and size, nothing extra. Final run: installed in 82 s, 13/13 files match. A model file then corrupted in place (same size): the app refused to load it and the task failed; after Remove model and a new setup (80 s, 13/13 files match) the same task ran. Corruption, tampering, oversize and resume are also unit-tested (`pegoles-inference` store tests). |
| 12 | Guest image real download + verification | PASS | The image is the immutable release asset `guest-image-0.3`; GitHub reports SHA-256 `a938365c…` for it, equal to the pin, and it downloads anonymously (redirect, then range requests). Final run: the installed app downloaded, verified and installed it from the public release in 37 s; the installed disk's SHA-512 equals the pin. The download path (HTTPS only, range resume, archive SHA-256, bounded decompression with disk SHA-512, atomic install) is also unit-tested. |
| 13 | VM boot | PASS | From the installed app: ready in 3.3–3.6 s, including after a guest crash and after Reset; 3.5 s in the final run. |
| 14 | Keyless local-model E2E | PASS | Installed app, no API key, Pegoles Local: "create hello.txt containing pegoles local, then cat it" completed and the VM screen shows `cat hello.txt` → `pegoles local` (twice in the private phase). Final run, from the clean clone of `58616be` with the image and model downloaded by the app: Done in 26 actions, same verified output. |
| 15 | Stop | PASS | Final run, healthy guest, through the GUI: a task waiting on `sleep 300 && echo finished` ended 2.0–2.5 s after the Stop press (screen captured every 0.5 s); Reset and a new task followed on the same computer. Earlier: 0.5 s with a crashed guest; `local_e2e` measured 2.2–2.5 s. |
| 16 | Reset | PASS | The Reset button (with an in-app confirmation) replaced the computer's disk with a fresh clone of the sealed image (new inode, the image's timestamp); the computer then started normally. Repeated in the final run with the same result. |
| 17 | Quit / relaunch | PASS | Quit mid-download, and quit with a crashed guest: the app, VM helper and model worker all exit. Relaunch: the model is still installed and Ready, the computer Off, and a partial download resumes. Final run: quit left 0 processes; the relaunched app ran a new task (after the model repair in gate 11). |
| 18 | No orphan processes | PASS | After every quit: no `pegoles-desktop`, `pegoles-vm-host` or worker process left. |
| 19 | Screenshot soak | PASS | `capture_soak`: 2212 captures, guest memory flat; `guest_memory_repro`: about 19 minutes of file-heavy guest work with a capture every 5 s, with and without concurrent model inference, no guest fault. |
| 20 | Hostile model, prompt-injection and IPC abuse tests | PASS | CI: `crates/pegoles-policy/tests` (escape matrix, property tests), `crates/pegoles-agent/tests/security_matrix.rs` (prompt-injected planners through the real executor and policy), local parser fail-closed tests, the Tauri ACL test (`ipc_acl.rs`: the capability equals the invoked command set). |
| 21 | Frontend / XSS / CSP | PASS | CI: `apps/desktop/src/security/*.test.*` (hostile strings render inert, no HTML sinks, only the `api` layer calls the backend); `webview_egress_probe` (the webview reaches no network: 0 TCP, 0 UDP) |
| 22 | Docs accuracy | PASS | README, SECURITY, PRIVACY, ARCHITECTURE, THREAT_MODEL, CONTRIBUTING, CHANGELOG checked against the code and these results; claims limited to what was run. |

### Found by the installed-app runs, and fixed

- A guest that stopped responding left the app on "Starting…" forever;
  the session now becomes an error after 60 s without a reconnect
  (unit-tested), and Stop or Reset recovers the computer.
- Settings showed "Runs on mac_o_s" (serde naming); image setup errors
  ran the raw backend message into the size note.
- The installer's pnpm store broke license notices in a developer
  checkout, silently; its disk-space requirement (15 GB) was a guess (the
  build peaks at about 5 GB; it now asks for 8 GB).
- Found in the final run, not fixed in 0.1.0: Settings keeps showing
  Pegoles Local as "Ready" when an installed model file is corrupted,
  because that status checks the file layout only. Loading verifies every
  file, so a corrupted model is never used: the task fails, and Remove
  model followed by a new setup repairs it.
- One guest kernel panic (ext4 code pages read as zeros in guest memory)
  under severe host memory pressure; not reproduced; image verified
  intact. Recorded as residual risk 16 in `docs/THREAT_MODEL.md`.

## Deferred: official signed binary distribution

Not source-first requirements. The binary release workflow stays in the
tree and runs only when `PEGOLES_BINARY_RELEASES` is enabled.

| Gate | Status | What it needs |
|---|---|---|
| Developer ID signing | DEFERRED | A "Developer ID Application" certificate (Apple Developer Program) in the `release` environment (`docs/GITHUB_RELEASE_CHECKLIST.md` §4). |
| Notarization | DEFERRED | An App Store Connect API key for `scripts/release/notarize.sh`. |
| Stapling | DEFERRED | Follows notarization. |
| Official notarized DMG | DEFERRED | The three above; then Gatekeeper on a quarantined download and a clean-machine install. |
| Build provenance attestation | DEFERRED | Part of the binary release workflow (`actions/attest-build-provenance`). |

## Security regression gates (unchanged, in CI)

| Gate | Status | Evidence |
|---|---|---|
| Model output parser fails closed | PASS | `local/parse.rs` tests + property tests (never panics, bounded) |
| Guest malformed/flooding IPC | PASS | `pegoles-computer` tests incl. the 20k-hello flood; bounded helper lines and bytes; handshake per connection |
| MLX worker isolation | PASS | `worker::tests::sandbox_blocks_escapes`: 17 probes succeed unsandboxed and fail sandboxed |
| Model and image hash verification | PASS | store and image tests; re-verification before every (re)load and boot |
| Webview network containment | PASS | `webview_egress_probe` (gate 21) |
| Native consent for cloud planner and key | PASS (logic) | `consent.rs` tests with an injected decider and the decline backoff |
| Lifecycle / resource soak | PASS | `release_soak`: 12 VM lifecycles, 1200 observations, 60 inferences with 2 forced worker kills; nothing left after each destroy |

## Supply chain and provenance

| Item | Status | Evidence |
|---|---|---|
| Model pins | PASS | `crates/pegoles-inference/catalog/models.json`: MAI-UI-2B 6-bit at revision `cb57cf2f…`, SHA-256 per file |
| Guest image pins | PASS | `catalog/images.json`: archive SHA-256 `a938365c…` (561,846,260 bytes), disk SHA-512 `766188e5…`; URL the immutable release asset `guest-image-0.3`; package manifest (511 packages) |
| Installer toolchain pins | PASS | `scripts/install.sh`: Rust 1.97.1 components, Node.js 24.21.0 and pnpm 9.15.9 by SHA-256/SHA-512; python-build-standalone by SHA-256; wheels by hash |
| Actions pinned to commit SHAs | PASS | hygiene script; the repository requires full-SHA pins and allows only GitHub-created actions plus two named ones |
| Release artifacts | PASS | Release `v0.1.0` (immutable): `build-manifest.json` (commit `58616be`, not dirty, the installer's pinned toolchains), `pegoles-0.1.0.cdx.json` (CycloneDX 1.7, 816 components) and `SHA256SUMS` (both files plus the guest image archive), from the clean-clone build of the tag; the digests GitHub reports match. No DMG. |
| Third-party licenses | PASS | `THIRD_PARTY_NOTICES.md` in the built app |
| Updater | N/A | No auto-updater in 0.1 |

## Hardware actually tested

One Mac: Apple M4 Pro, 24 GB, macOS 26.5.2 (build 25F84), Command Line
Tools only (no Xcode). Nothing was run on 8 GB or 16 GB machines, on other
chips, or on older macOS; the app declares macOS 14.0 as its minimum
without having been tested there.
