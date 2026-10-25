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
| Mitigation | `pegoles-policy::evaluate` is an exhaustive match: finite unit-square coordinates, drag/scroll/wait/text caps, key vocabulary and chord validation, no control or Unicode format/bidi/zero-width characters in typed text, a key-material tripwire. `RequireApproval` suspends an exact request for finite native human confirmation; absent confirmation it fails closed. `pegoles-core/src/input.rs` applies rate limit → policy → cancellation → control ownership (checked under the dispatch lock). Waits are policy-checked and audited. Keys are dispatched in the normalized form the policy judged. |
| Verification | `pegoles-policy` escape matrix + property tests (never panics, never allows out-of-range or non-finite input), `pegoles-core` executor tests, `core_computer::scripted_run_goes_through_the_real_executor_and_policy` |
| Residual | The key-material tripwire is a heuristic, not a boundary (the VM has no network, so typed secrets cannot leave it). |

### Local model worker

| | |
|---|---|
| Surface | A Python process that parses model files and runs MLX/transformers |
| Mitigation | `pegoles-inference/src/worker.rs`: interpreter only from the signed bundle (`Contents/Resources/runtime`), `python -I`, cleared environment (no API key), cwd `/`, and a mandatory `sandbox-exec` profile: no network, no exec except its own interpreter, no Apple Events, Mach lookups only for `com.apple.MTLCompilerService` (no LaunchServices, pasteboard, Keychain, WindowServer), no signals to or inspection of other processes, IOKit limited to GPU/IOSurface clients, no file contents under `$HOME` except runtime/models/worker, writes only to a private 0700 temp dir. Supervisor bounds every reply (1 MiB), stderr, request writes and time; kills on garbage, hangs or stalls. Output is only ever parsed into typed actions. |
| Verification | `worker::tests::sandbox_blocks_escapes` (16 escape probes, unsandboxed control first), supervisor tests (crash, garbage, oversized, hang, closed stdout, stalled stdin), real inference under the profile |
| Residual | `sandbox-exec` is deprecated by Apple (still present on macOS 26). A compromised worker can read system files outside `$HOME` and file metadata under `$HOME`, and can lie to the planner, which is already untrusted. |

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
9. Windows/Linux host backends are not part of this release and unverified.
10. The release `sign` job trusts the unsigned bundle built by the `build` job of the same commit: a dependency compromised at its locked version could alter what gets signed (it cannot use the identity; the sign job checks the worker, lock and manifest against the checkout, and notarization scans the result).
11. Swift helper: a closed-then-reused descriptor race around superseded guest connections remains possible in a nanosecond window (same VM only today).
12. After Stop, releasing a held mouse button waits up to about 5 s on a guest that withholds acknowledgements.
13. The published image keeps cloud-init's provisioning logs and state and one `machine-id` shared by every copy (no secrets; the VM has no network).
14. Same-user host software can change an installed image between the once-per-session hash and a clone, or modify a resumed computer's disk (it already owns the account).
15. The 2B local model is imperfect: it can fail or stop tasks (budgets and brakes bound what it can do).
16. One guest kernel panic was observed in the first installed-app run, with the host under severe memory pressure (swap nearly full, about 2 GB of disk free): code pages of the guest's ext4 module read as zeros in guest memory. The image and both copies of the module were verified intact, the memory balloon negotiates no free-page reporting and gets no target, and about 19 minutes of targeted stress (file-heavy work, captures, concurrent model inference) did not reproduce it. Root cause unknown. The app now reports a guest that stops responding (after 60 s) instead of showing "Starting", and Stop or Reset recovers the computer.
17. A source install runs the project's and its dependencies' build steps (for example Rust `build.rs`) on the user's Mac, like any build from source; the inputs are the checkout plus digest- or lockfile-pinned downloads.

### Local appearance preferences (Agent 2.0, offline verification)

| | |
|---|---|
| Surface | The untrusted webview's `get_preferences`/`set_preferences` IPC payloads and local preference bytes. A compromised page can change appearance and language through its existing main-window authority. |
| Mitigation | `src-tauri/src/preferences.rs` validates schema version, closed enum sets and bounded Unicode names; serde refuses extra fields. Fixed app-data filename, 4096-byte read cap, regular-file checks, symlink refusal, exclusive private temporary file and synced atomic replacement. Dedicated blocking store mutex; no file/plugin permissions or path/style/markup parameters. A failed read/save remains an error in the UI. |
| Verification | Preference store tests cover round-trip/reopen, normalization, invalid enum/version/name/locale, unknown fields, corruption, oversize, disk errors, symlink targets and Unix file/directory permissions. Existing real Tauri ACL tests include the new commands and deny remote origins/other windows; the frontend checks exact invoke shapes. Dialog tests cover save rejection, load retry, disabled pending controls, focus containment/restoration, static reduced-motion preview and locale persistence. |
| Residual | Same-user host mutation races remain outside scope. The stored name and locale are local metadata, excluded from planner context; the chosen name can contain markup-shaped text and is always rendered inert. macOS hardware visual/accessibility acceptance and Windows file ACL/reparse-point verification were not run for this change. |

Localization uses explicit static catalog keys and plain interpolation, without
HTML sinks or DOM text rewriting. Protocol IDs, task IDs, objectives, model
narration and guest text retain their original values. Core state precedence,
network containment, sandbox, policy and planner authority are unchanged.


### Local task history and summary artifacts (Agent 2.0, offline verification)

| | |
|---|---|
| Surface | Existing user objectives, untrusted final planner Summary text and local JSON bytes; fixed `get_history_status` / `get_task_history(taskId)` IPC reads. |
| Mitigation | `pegoles-core/src/history.rs`: strict versioned/nested schema, duplicate-ID/association/title/time validation, bounded 512 KiB reads; 100 tasks, 30 days from update, 32 status rows/task, 500-character objective and one 4,000-character text/table result per task. Tables cap at 8 columns × 32 rows, 256 characters/cell, 4,000 total cell characters. No raw events/requests/actions/keys/typed text/screenshots/diagnostics on disk. Explicit projection only. Sensitive typing or secret-pattern objectives/results permanently taint that task's result; event-bus lag discards retained results conservatively. |
| Storage and recovery | Fixed app-data path; existing directory components and leaf reject symlinks/nonregular files; leaf O_NOFOLLOW/nonblocking opens on Unix; one fixed exclusively created 0600 staging file, 0700 directory, fsync + atomic replacement + directory sync. Permanent empty 0600 `.tasks.lock` with nonblocking OS locking serializes reads/writes/recovery and is never unlinked. Canonical snapshot ≤512 KiB plus at most one ≤512 KiB staging snapshot gives a finite 1 MiB transient aggregate. Before use, a ≤256-entry metadata scan discards only regular reserved fixed/legacy UUID stages without following symlinks, reading their contents, recursing or promoting them. Cleanup/lock/scan failures become read-only. Existing canonical file must validate before replacement. Startup prunes expired bytes and saves normalized recovery. Invalid/unwritable stores become read-only with a sidebar warning; corrupt data is preserved. Pending/running/waiting records recover cancelled without live events, execution or approval. |
| Rendering / authority | Typed, associated text/table summaries only, labelled as agent results. React text nodes and table cells, no HTML/Markdown interpretation, no path/URL/file-read command, no new executor/protocol authority. Previous tasks/results remain navigable after restart; locale changes preserve mounted task/draft data. |
| Verification | Core history tests: actual round-trip/reopen, recovery with no bus events, strict schema/association, exact count/age/read-byte limits and serialized-byte retention, corrupt/oversize/symlink/disk/private-mode failures, objective/input/output taint. Crash fixtures verify full private stages disappear on reopen, repeated fixed-slot interruptions remain bounded, active writer locks prevent cleanup/competing writes, exact scan limits and unrelated/corrupt canonical preservation. AppState tests: isolated injection, physical startup retention, disk read-only warning, lost-event output suppression. Artifact/App tests: inert hostile text/table cells, empty/load/error states, task navigation, previous tasks and visible warning. IPC shape/ACL and existing HTML-sink tests remain required. |
| Residual | Local objectives and allowed summaries are private, unencrypted user data; secret-pattern detection is conservative but incomplete. Same-user host mutation races remain outside scope. Corruption is preserved instead of automatically repaired; storage failures can delay physical expiry. Retention runs on startup/commands, not while the app is closed. Windows ACL/reparse-point and installed-app/hardware restart behavior remain unverified. |

### Human approval authority (Agent 2.0, offline verification)

The optional **Ask before agent input** setting adds approval only to otherwise
allowed modifying VM actions. Default remains off; both changes require native
confirmation. The UI explicitly labels it session-only and restart resets it.
Existing policy Deny rules, sealed/offline VM and private-key tripwires remain.

Core stores at most eight exact pending requests (one per task), bound to the
original action/task/computer IDs, Agent ownership and control generation. A
120-second monotonic deadline fails closed. The webview gets only IDs, a safe
verb summary and a wall-clock display deadline. Approval IPC offers only
`approve_once` and `deny`; no replacement action, payload, path or blanket grant.
Approve once must also pass native ConsentGate. It uses a Core-derived safe
summary outside Core locks, then revalidates the live task/request/deadline and
ownership after the human responds. Return/Escape do not approve natively;
existing one-dialog-at-a-time and declining backoff patterns apply. Native
refusal or unavailable confirmation denies. Guest/model approval tags fail parsing.

The runner waits outside Core locks and honors its cancellation/deadline token.
Denial/expiry ends the run rather than becoming a model-retry loop. Long typing
is approved as one complete payload and sliced under short locks; full-payload
policy and ownership are checked again for every slice. Failed/cancelled slices
report the previously completed prefix. Clearing raw pending payloads preserves
at most eight content-free stop records for that accounting. Typed stop reasons
separate resumable control intervention from terminal explicit denial/expiry. No grant or pending payload is persisted or recovered.

Residuals: a compromised webview can deny work, request native prompts within
the existing backoff, and misrepresent its own display. It cannot answer the
native confirmation. Safe summaries intentionally omit typed contents, so the
human approves the bounded input action in context rather than reviewing its
text in the dialog. Expiry uses monotonic Core time; the webview countdown can
be inaccurate after clock changes. Existing VM/guest round-trip cancellation
latency remains. Native visual/VoiceOver and actual hardware execution were not
run; injected deciders prove authority/lock/race behavior offline. This is a
session guard, not durable security policy or a new host capability.

### Native-confirmed snapshot human control

The webview remains untrusted: it can request a native prompt, but cannot grant
itself User ownership. The trusted native confirmation binds one exact current
computer/frame/generation; post-dialog checks reject changes or expired captures.
Once the person grants that session, compromised page code can issue the same
bounded guest inputs as the person until return/stop/failure revokes it. This
session authority is intentional and visible; it never grants host input, host
capture, file access, networking or arbitrary protocol dispatch.

Core owns snapshot capability and a monotonic successful-capture instant separate
from deduplicated pixel identity. Every input checks exact owner, epoch, computer,
frame and dimensions under the dispatch lock. Unknown fields, nonfinite/outside
coordinates, noncanonical keys, text >256 scalars and >60 events/s fail closed.
One desktop IPC is in flight; no Core input queue. UI drops its ≤16 event queue
on blur/hide/error/return; no held modifiers exist because key/chord operations
release atomically. Pointer buttons release on all exits. IME is unsupported.

Human intervention pauses the original task without resetting cancellation or
replaying typed prefixes. A task submitted while the person already controls the
guest waits outside the Core lock (bounded by Stop and its preparation/run
deadlines), re-observes if ownership changes before planning starts, and is
privacy-tainted from its first running status. Generation checks invalidate even a planner that
returns after the person took and returned control. Outstanding tool IDs receive
explicit intervention results before fresh observation/replanning. Deny/expiry
stay terminal. Human takeover conservatively taints active-task artifacts even
without TypeText; history chronological status audit handles delayed event drains.
Tests use injected native deciders, fake planners/channels and the real executor
with Mock. Native embed, actual keyboard/VoiceOver and Apple silicon VM behavior
remain hardware acceptance; periodic snapshots are not a native live display.

### Experimental opt-in team planner

Team mode is a cloud-only planner that runs captain plan, researcher,
executor, critic and captain synthesis in sequence behind the unchanged
`Planner` trait. It adds no host capability and no authority.

- Authority: only the executor (the wrapped Anthropic planner) returns
  `PlannerTurn::Calls`, and the team forwards them untouched, so every
  action is still parsed into a typed `ComputerAction`, evaluated by
  `pegoles-policy::evaluate`, dispatched by the Core executor and subject to
  approvals (PR 4) and snapshot takeover (PR 5). The other roles are
  text-only requests with no `tools`, no thinking and a 1024-token reply
  cap; a reply containing a tool call is a protocol error, and role text is
  never parsed into actions. A role therefore holds no permission, cannot
  approve and cannot widen what the executor may do. Role and executor text is
  hostile like any model text: it can be wrong or injected from the screen,
  and the protection is the policy, not the prompts. The executor receives
  the team's plan and notes after the user's objective, labelled as
  untrusted guidance; prompt-level influence on the executor is accepted
  and bounded by policy.
- Opt-in and consent: off by default; enabling needs a cloud provider and a
  native macOS confirmation, recorded with the disclosure version; going back
  to Pegoles Local turns it off; a hand-edited file cannot run the team on
  the local model, and `TeamPlanner::new` independently refuses a planner or
  role model that is not cloud. The webview cannot supply the consent.
- Budgets and cancellation: 180 s per model request, 25 min for the team,
  60 executor turns, each in addition to the runner's own limits. A watchdog
  trips the step's token on timeout or on Stop, so a hung request ends there;
  a timeout or error ends the run at that step (later steps reported as
  skipped), and Stop is reported as cancelled, not failed.
- Display: the UI shows only role names, states and the cleaned, bounded
  (400 characters) visible reply of each role, and the final answer, as
  plain text. Invisible formatting and control characters are removed in
  Rust; no hidden reasoning is requested or shown (thinking blocks are
  discarded). The final summary is published after the last team event and
  before the task closes.
- Takeover: if a human changed the screen after the team concluded, the
  runner drops that answer and the team returns to the executor (critic and
  captain run again); stale step states are cleared in the UI.
- Residual: the team multiplies cloud requests and screenshots per task
  (privacy and cost, disclosed at consent); a role could write misleading
  text that the person reads; the critic's verdict is advisory and does not
  gate completion. The cloud path was not exercised live: all tests use
  fakes.
