# Pegoles threat model (release 0.1)

Concrete threat model for the first public release. Every control below
names the code that enforces it and the test or check that verifies it.
`docs/SECURITY.md` explains the design; `docs/RELEASE_GATES.md` records
what was verified for this release and how.

Pegoles is a computer-use agent: a planner (a model) looks at a screen and
proposes input. We assume the planner can be fully malicious, the screen
can carry instructions written by an attacker, and everything inside the
VM can be hostile. Security comes from deterministic layers the model
cannot talk its way through, not from the model behaving well. No claim
here is "Pegoles is secure"; each claim is scoped to a boundary.

## Assets

| Asset | Why it matters |
|---|---|
| Host files, processes, network, input devices, screen | The agent must never reach them. This is the primary asset. |
| Anthropic API key (optional) | Billing and account abuse; stored in the macOS Keychain. |
| Content of the user's tasks (objective, VM screenshots) | Private by default; leaves the Mac only when the user chooses a cloud planner. |
| Integrity of the model weights | Weights are parsed by a large native stack; tampered weights are an exploit vector into the worker. |
| Integrity of the guest image | Pegoles' guest runtime and hardening live in it; a tampered image is attacker code from boot (still inside the VM). |
| Release signing identity (Developer ID), notarization credentials | Whoever holds them can ship "Pegoles" to users. |
| The published release artifacts | Users map a download to a commit only through them. |

## Trust boundaries

```text
User
 │  (types objectives, clicks Stop/Reset, may enter an API key)
Tauri webview (React, apps/desktop/src)            untrusted rendering surface
 │  Tauri IPC: fixed command set, no paths/URLs/process args
Tauri Rust backend (apps/desktop/src-tauri)        trusted
 │
Agent runtime (crates/pegoles-agent runner)        trusted, budgets and brakes
 │  Planner trait
Model provider ── local: MLX worker (sandboxed Python process) ── untrusted
               └─ cloud: Anthropic API over HTTPS ────────────── untrusted
 │  model text
Strict parser (local/parse.rs, anthropic.rs translate)   turns text into ≤1 typed action
 │  ComputerAction (observe / pointer / keyboard / wait only)
Pegoles Policy (crates/pegoles-policy)             deterministic, exhaustive
 │
Core executor (crates/pegoles-core/src/input.rs)   rate limit → policy → cancel → control
 │  JSONL, closed command set
VM helper (native/macos/pegoles-vm-host, Swift)    only process with Virtualization.framework
 │  Virtualization.framework: no NIC, no shared dirs, no clipboard
Guest runtime (guest/runtime, Linux, vsock)        untrusted peer (authenticated by reserved port)
 │
Guest OS and apps (Debian + Weston + foot)         hostile
```

Supply-chain boundaries, separate from the runtime chain:

```text
Hugging Face (pinned revision) ─HTTPS─▶ model store (SHA-256 per file) ─▶ worker
GitHub release asset ─HTTPS─▶ image installer (SHA-256 archive, SHA-512 disk) ─▶ VM
python-build-standalone + PyPI wheels ─build time─▶ bundled runtime (pinned, hash-locked) ─▶ signed app
crates.io / npm (lockfiles) ─build time─▶ app binaries
GitHub Actions (pinned SHAs) ─▶ CI; protected `release` environment ─▶ signing + notarization ─▶ draft release
```

## Threat actors

| Actor | Controls | Wants |
|---|---|---|
| Malicious model | Every token the planner emits (local or cloud, incl. a MITM'd or compromised provider) | Host execution, data exfiltration, persistence, denial of service |
| Prompt-injected screen | Any text or image inside the VM | Make an honest model act for the attacker |
| Malicious guest | Root in the VM, arbitrary vsock traffic, can kill/replace the runtime | Escape to the host, crash or wedge the app |
| Compromised download | Bytes served for a model, the guest image or the runtime inputs | Code execution in the worker/host, a backdoored guest |
| Malicious dependency | A crate, npm package, wheel or Action | Code execution at build time or at run time |
| Compromised CI action / malicious PR | Code that runs in GitHub Actions | Steal release secrets, tamper with artifacts |
| Stolen release credential | Developer ID certificate or notary key | Ship a signed "Pegoles" |
| Local same-user malware | Already runs as the user | Out of scope as an attacker against Pegoles (it already owns the account); Pegoles must not make it worse |

## Boundary by boundary

Each row: attack surface → mitigation (where) → verification → residual risk.

### Model output → host

| | |
|---|---|
| Surface | Free-form model text; Anthropic tool-use JSON |
| Mitigation | No host verb exists: `pegoles-protocol/src/actions.rs` has only observe/pointer/keyboard/wait variants and rejects unknown tags and fields. Local text is parsed by `pegoles-agent/src/local/parse.rs` (exactly one closed `<tool_call>`, fixed tool name, allow-listed keys, finite bounded coordinates, 16K output cap); cloud output by `anthropic.rs::translate` (only the computer toolset, bounded points, unknown members become errors). One action per step. |
| Verification | `pegoles-protocol` deserialization tests (forbidden verbs, extra fields, nested schemas), `pegoles-agent` parser tests and property tests (never panics, fails closed), the malicious-planner matrix in `pegoles-agent` tests, `local_bench --safety` on the real VM |
| Residual | A model can do anything a user could do *inside* the offline VM: type, click, look. That is the product. |

### Parser → Policy → executor

| | |
|---|---|
| Surface | Typed `ComputerAction` values chosen by the model |
| Mitigation | `pegoles-policy::evaluate` is an exhaustive match: finite unit-square coordinates, drag/scroll/wait/text caps, key vocabulary and chord validation, no control or Unicode format/bidi/zero-width characters in typed text, a key-material tripwire. `RequireApproval` is treated as deny. `pegoles-core/src/input.rs` applies rate limit → policy → cancellation → control ownership (checked under the dispatch lock). Waits are policy-checked and audited. Keys are dispatched in the normalized form the policy judged. |
| Verification | `pegoles-policy` escape matrix + property tests (never panics, never allows out-of-range or non-finite input), `pegoles-core` executor tests, `core_computer::scripted_run_goes_through_the_real_executor_and_policy` |
| Residual | The key-material tripwire is a heuristic, not a boundary (the VM has no network, so typed secrets cannot leave it). |

### Local model worker

| | |
|---|---|
| Surface | A Python process that parses model files and runs MLX/transformers |
| Mitigation | `pegoles-inference/src/worker.rs`: interpreter only from the signed bundle (`Contents/Resources/runtime`), `python -I`, cleared environment (no API key), cwd `/`, and a mandatory `sandbox-exec` profile: no network, no exec except its own interpreter, no Apple Events, Mach lookups only for `com.apple.MTLCompilerService` (no LaunchServices, pasteboard, Keychain, WindowServer), no signals to or inspection of other processes, IOKit limited to GPU/IOSurface clients, no file contents under `$HOME` except runtime/models/worker, writes only to a private 0700 temp dir. Supervisor bounds every reply (1 MiB), stderr, request writes and time; kills on garbage, hangs or stalls. Output is only ever parsed into typed actions. |
| Verification | `worker::tests::sandbox_blocks_escapes` (16 escape probes, unsandboxed control first), supervisor tests (crash, garbage, oversized, hang, closed stdout, stalled stdin), real inference under the profile |
| Residual | `sandbox-exec` is deprecated by Apple (still present on macOS 26). A compromised worker can read system files outside `$HOME` and file metadata under `$HOME`, and can lie to the planner, which is already untrusted. |

### Local model worker on Windows (llama.cpp)

Implemented, not yet run on a Windows PC (see `docs/WINDOWS_ARCHITECTURE.md`).

| | |
|---|---|
| Surface | `pegoles-llm-worker.exe`: llama.cpp parsing GGUF files and screenshots |
| Mitigation | `pegoles-inference/src/sandbox_windows.rs`: an AppContainer with **no capabilities** (no network, no user files or registry beyond what AppContainers may read; read/execute granted only on the model store), a child-process-restricted policy, a job object (killed with Pegoles, one process, committed-memory cap, no desktop/clipboard/global atoms/system parameters, no error-reporting dialog), exactly the three pipe handles inherited, and an environment rebuilt from scratch (implicit Vulkan layers such as overlays disabled). Backend DLLs load only from the admin-only install folder. Same supervisor, protocol bounds, pre-load re-hash and text-only output as the MLX worker; the prompt is rebuilt from a strict subset with special-token text neutralized. |
| Verification | Unit tests (protocol, prompt template, prep bounds); the worker runs under `sandbox-exec` on macOS against the real VM (`local_bench`); on Windows CI, `a_confined_worker_cannot_read_files_reach_the_network_or_start_processes` (each probe succeeds unconfined and fails confined) |
| Residual | GPU drivers run inside the worker (a driver bug is reachable from model input). The Windows escape probes cover files, network and processes; registry and named-object probes are not written yet. |

### Windows VM broker (privileged)

Implemented, exercised only on CI runners (Windows Server, administrator).

| | |
|---|---|
| Surface | `PegolesVmBroker`, a LocalSystem service reachable over `\\.\pipe\pegoles-vm-broker` |
| Mitigation | Eight fixed verbs (`hello`, `create`, `start`, `pause`, `resume`, `shutdown`, `terminate`, `state`), one bounded JSON line each, typed with `deny_unknown_fields`. Pipe: local clients only, DACL SYSTEM/Administrators/interactive users; the client's image must be `pegoles-vm-host.exe` in the broker's own admin-only folder; the client is impersonated to learn its SID and to open its files. The disk path is checked as the user and again as the service: every folder from `Pegoles` down and the disk must be the same file object in both views (defeating per-user drive-letter remapping), none may be a reparse point, and the folders stay open without delete sharing for as long as the VM exists, so they cannot be swapped for junctions between the check and HCS's use. The broker and its install step refuse to run outside Program Files (their trust in the neighbouring helper and the SYSTEM service binary assumes an admin-only folder). The broker writes the HCS document itself from validated values (UUID ids, 1–8 vCPUs, 1–8 GB, the user's own `…\Pegoles\computers\<id>\disk.vhdx`, no reparse points): no network adapter, no shared folders, no keyboard or mouse (only Hyper-V's synthetic video, the guest's own screen, which the host never shows), HvSocket limited to SYSTEM and that user. VMs are leased to the pipe connection (terminated when it closes) and to the broker (`ShouldTerminateOnLastHandleClosed`); at most two per user. Demand-start, stops when idle. Installed per machine; the installer registers it, the uninstaller removes it. |
| Verification | `pegoles-broker-proto` tests (request parsing, path and value validation, the HCS document has no network/share/video/input devices); CI installs it, starts it, boots a VM through it and uninstalls it |
| Residual | A bug in the broker is a local privilege-escalation surface for any interactive user (the verbs are few and typed, but it is still SYSTEM code parsing input). The client image check is a speed bump against same-user malware, which already owns the account; a signature check on the peer comes with code signing. Whether HCS or the VM worker ever opens the disk with more than the VM's own identity is not documented; the access grant is made as the user, so a file the user cannot change cannot be granted. An independent review (2026-09-27) found the check-then-use race described above; fixed as described, not yet exercised by a hostile test on Windows. |

### Model weights (supply chain)

| | |
|---|---|
| Surface | Files downloaded from Hugging Face |
| Mitigation | Compiled-in catalog (`pegoles-inference/catalog/models.json`): full commit revision, SHA-256 and size of every file, allowed file types (safetensors/JSON/text/Jinja only), no `auto_map`/custom code; HTTPS on every hop; resumable download bounded to the pinned size; installed only after the whole layout verifies (then atomic rename); re-hashed before the first load of each session; the worker disables transformers' dynamic modules and remote code. The UI can only install catalog ids. |
| Verification | `pegoles-inference` store tests (resume, corruption, oversize, symlinks, extra files, remote-code JSON, stale staging), real download + verification of both offered models |
| Residual | Trust in the pinned upstream revision itself (Hugging Face accounts of the model publishers). |

### Guest image (supply chain)

| | |
|---|---|
| Surface | A 562 MB archive downloaded on first run; the installed disk in the data directory |
| Mitigation | `pegoles-computer/src/image_release.rs` + `catalog/images.json` compiled into the signed app: archive SHA-256 and size, disk SHA-512 and size. HTTPS-only, resumable, bounded to the pinned size; pure-Rust gzip bounded to the pinned disk size; manifest and marker written from the pin, never from the download; atomic install. Release builds refuse to boot an installed image whose manifest does not match the pin and re-hash the disk once per session before cloning it. Never a fallback to a generic Debian image. The published image is sanitized (no SSH host keys, no random seed, locked accounts, empty `authorized_keys`) by `scripts/build-guest-image/sanitize-image.sh`. |
| Verification | `image_release` tests (tampered archive, oversize stream, decompression to other bytes, corrupt gzip, resume, mirror fallback, cancel, tampered install, symlinked disk, unpublished catalog), installation of the real archive through the product installer |
| Residual | A tampered image is attacker code *inside the VM*, which the design already treats as hostile. Same-user malware can modify the installed disk between the session check and a clone (it already owns the account). |

### Guest → helper → Core

| | |
|---|---|
| Surface | vsock frames from the guest; helper JSONL; serial console |
| Mitigation | Swift helper (`GuestSocket.swift`): reserved-source-port peer authentication, 64 KiB frames, UTF-8 and control-byte checks, token bucket per guest link, bounded write backlog. Rust (`native_backend.rs`, `guest.rs`, `pegoles-guest-proto`): typed allow-listed messages with per-field caps, bounded line and byte budgets per pump, non-reentrant outbound flushing with a capped queue, overall deadlines on helper calls, a fresh handshake on every connection, bounded frame reassembly. The helper is killed on hangs or garbage (its VM dies with it) and exits with its parent. Serial log capped at 8 MiB; the UI gets a sanitized tail. |
| Verification | `pegoles-computer` hostile-frame tests incl. the hello-flood regression on a 2 MiB stack, bounded-line tests, handshake tests; hardware E2E (runtime killed inside the guest → recovered) |
| Residual | A guest can deny service to its own agent (kill or starve the runtime). Hypervisor escape is Apple's boundary. |

### Webview → Tauri backend

| | |
|---|---|
| Surface | Any script running in the webview (assume XSS) |
| Mitigation | Release CSP (`tauri.conf.json`): `script-src 'self'`, IPC-only `connect-src`, no frames, workers, manifests or media, no remote content; DNS prefetch off. Network containment (`src-tauri/src/webview_egress.rs`): the webview is created with a WebKit content rule list that blocks every http(s)/ws(s)/ftp(s)/file load (subresources, preconnects, navigations), compiled before the window exists (no list, no window); WebRTC constructors removed in every frame. Navigation away from the app and new windows are refused; an app ACL grants only the commands the release UI invokes (debug/lab commands are compiled out); no command takes a path, URL, process argument or endpoint; model ids must be catalog ids; switching to a cloud planner and storing an API key need a native confirmation the webview cannot answer (Cancel is the keyboard default; declines back off 30 s → 10 min → until restart); the key is typed into a native secure field and never returned to the webview. React renders model, guest and error text as text nodes only. |
| Verification | Rust capability/ACL tests; `examples/webview_egress_probe.rs` (the real page tries fetch, beacons, preconnect, dns-prefetch, prefetch, WebRTC incl. from an iframe, and navigation: uncontained control reaches the listeners over TCP and UDP, contained reaches nothing while IPC still works); frontend XSS regression tests; production-bundle lab-exclusion test on the real build |
| Residual | `style-src 'unsafe-inline'` (motion library inline styles). A script in the webview can still drive the UI's own commands (start/stop/reset the VM, start tasks, read task text and VM screenshots over IPC), but has no network to send them anywhere. It could open the native file picker (WKWebView implements it) and read a file the person picks, again with no way out. After a person has consented to the cloud planner, a compromised page could keep it selected while showing "Pegoles Local"; there is no native indicator of the active planner yet. |

### Cloud provider (optional)

| | |
|---|---|
| Surface | Anthropic API responses; the network path |
| Mitigation | Fixed HTTPS endpoint compiled in, no redirects followed (the key header never leaves the endpoint), response and conversation budgets, same parser → policy path as local. |
| Residual | Using the cloud planner sends the objective and VM screenshots to Anthropic (documented in the UI and `docs/PRIVACY.md`). The cloud path was not exercised live for this release (no key available). |

### Build, CI and release

| | |
|---|---|
| Surface | Dependencies, GitHub Actions, the release job, signing credentials, the source installer |
| Mitigation | Lockfiles with checksums/integrity (`Cargo.lock`, `pnpm-lock.yaml`, hash-locked wheels, pinned python-build-standalone); `cargo deny`, `cargo audit`, `pnpm audit`, dependency review, CodeQL; every Action pinned to a commit SHA; workflows default to read-only tokens; PR workflows have no secrets and never run on self-hosted runners; release runs only from a `v*` tag in the protected `release` environment (required reviewer), signs inside-out with a Developer ID (hardened runtime, timestamps, minimal entitlements), notarizes, staples, verifies (`scripts/release/verify-artifact.sh --distribution`), emits SHA256SUMS, an SBOM and build provenance attestations, and creates a *draft* release that a human publishes. That binary pipeline runs only once the repository variable `PEGOLES_BINARY_RELEASES` is set: 0.1.x is source-first, built on the user's Mac by `scripts/install.sh`, which downloads only digest-pinned toolchains (Rust, Node.js, pnpm; cargo-about by version from crates.io with `--locked`), installs JavaScript packages from the lockfile without lifecycle scripts, runs in a clean environment, refuses quarantined checkouts and cargo configurations another user could have planted above the checkout, and replaces an installed app only after the new one is staged and verifies. No auto-updater ships in 0.1. |
| Verification | CI runs on every PR; `verify-artifact.sh` on every bundle the installer builds (and on the exact DMG for binary releases); installer edge-case tests and an adversarial review (docs/RELEASE_GATES.md) |
| Residual | Trust in GitHub-hosted runners and Apple's notary service; a maintainer account compromise. |

## Residual risks (accepted for 0.1, tracked)

1. Anything a planner can do inside the offline VM (by design).
2. Guest denial of service against its own agent.
3. `sandbox-exec` deprecation; worker can read non-home system files.
4. Weston runs with `--debug` for `weston_capture_v1`: any guest client of the same user can capture the guest screen (the guest is hostile anyway).
5. GUI apps and the runtime share one guest user (mitigated by reserved-port auth and a non-dumpable runtime).
6. Hypervisor escape (Apple Virtualization.framework).
7. The image keeps `openssh-server` and `cloud-init` installed but masked/disabled; the VM has no network device.
8. Same-user host malware is out of scope as an attacker.
9. Windows/Linux host backends are not part of this release and unverified. (Windows, 2026-09-27: implemented on the `phase/windows-0.2` branch and exercised only in CI; see `docs/WINDOWS_ARCHITECTURE.md`.)
10. The release `sign` job trusts the unsigned bundle built by the `build` job of the same commit: a dependency compromised at its locked version could alter what gets signed (it cannot use the identity; the sign job checks the worker, lock and manifest against the checkout, and notarization scans the result).
11. Swift helper: a closed-then-reused descriptor race around superseded guest connections remains possible in a nanosecond window (same VM only today).
12. After Stop, releasing a held mouse button waits up to about 5 s on a guest that withholds acknowledgements.
13. The published image keeps cloud-init's provisioning logs and state and one `machine-id` shared by every copy (no secrets; the VM has no network).
14. Same-user host software can change an installed image between the once-per-session hash and a clone, or modify a resumed computer's disk (it already owns the account).
15. The 2B local model is imperfect: it can fail or stop tasks (budgets and brakes bound what it can do).
16. One guest kernel panic was observed in the first installed-app run, with the host under severe memory pressure (swap nearly full, about 2 GB of disk free): code pages of the guest's ext4 module read as zeros in guest memory. The image and both copies of the module were verified intact, the memory balloon negotiates no free-page reporting and gets no target, and about 19 minutes of targeted stress (file-heavy work, captures, concurrent model inference) did not reproduce it. Root cause unknown. The app now reports a guest that stops responding (after 60 s) instead of showing "Starting", and Stop or Reset recovers the computer. Seen a second time on 2026-09-27 in a `local_bench` run on the development Mac (about 20 GB of 24 GB in use when it started): the guest's `vsock` module code read as zeros (`Code: 00000000…`, "undefined instruction" in `__vsock_create`), after which the runtime could not open its socket. Three later runs on the same image (MLX and llama.cpp workers, 34 tasks in all) did not show it. Same signature, still unexplained.
17. A source install runs the project's and its dependencies' build steps (for example Rust `build.rs`) on the user's Mac, like any build from source; the inputs are the checkout plus digest- or lockfile-pinned downloads.
