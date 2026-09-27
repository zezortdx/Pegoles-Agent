# Security

Pegoles runs a model-driven agent against a computer. The design goal is
that a fully prompt-injected model, a hostile web page or file inside
the guest, or a compromised guest OS cannot reach the user's Mac. This
document states what is enforced today, where, and what is not.

## Trust boundaries

```text
 user ─▶ webview UI ─(Tauri commands)─▶ Pegoles Core (Rust, host)
                                           │  typed ComputerAction only
   local VLM (MLX worker, sandboxed) ◀─pipes─┐
          model (Anthropic API) ◀─HTTPS─ agent runner ─▶ Policy ─▶ executor
                                           │
                                  pegoles-vm-host (Swift, child process)
                                           │  Virtualization.framework
                                 ┌─────────┴──────────────── hypervisor ──┐
                                 │ guest: Debian + weston + foot + runtime │
                                 └─────────────────────────────────────────┘
```

| Boundary | Trust | Enforcement |
|---|---|---|
| Model → host | untrusted (local and cloud alike) | model output is parsed into typed actions only — cloud: `pegoles-agent/src/anthropic.rs::translate`; local: `pegoles-agent/src/local/parse.rs` (exactly one tool call, allow-listed keys, bounded finite coordinates, no control/bidi/zero-width text); unknown tools, out-of-screen coordinates and unsupported members become error results, never actions |
| Local model worker → host | untrusted process | the interpreter shipped in the signed bundle only (`Contents/Resources/runtime`); `workers/mlx/pegoles_mlx_worker.py` runs under `sandbox-exec` (mandatory; missing → Pegoles Local refuses to run): no network (TCP, UDP, DNS, unix sockets), no fork, no exec but its own interpreter, no Apple Events, Mach lookups only for `com.apple.MTLCompilerService` (no LaunchServices, pasteboard, Keychain, WindowServer), no signals to or inspection of other processes, IOKit only GPU/IOSurface clients, no file contents under `$HOME` except the runtime, the model store and the worker script, writes only to a private 0700 temp dir (emptied on spawn); cleared environment (no API key), `python -I`, own process group (killed as a group); protocol on a private stdout duplicate; replies bounded (1 MiB/line), stderr drained in bounded chunks, stalled writes, garbage, oversize, timeouts and unhonored cancels → killed. Pinned by `worker::tests::sandbox_blocks_escapes` (17 probes against the real runtime, with an unsandboxed control) |
| Model files → worker | untrusted bytes | compiled-in catalog pins repository commit and SHA-256 of every file; only safetensors/JSON/text/Jinja files; any model JSON with `auto_map`/`custom_pipelines` refused; staged files always re-hashed, moved into place only after the whole tree verified; full re-hash before every (re)load; transformers' dynamic-module loader disabled in the worker, `trust_remote_code=False` |
| Guest image → VM | untrusted bytes | compiled-in pin (`pegoles-computer/catalog/images.json`): archive SHA-256 and size, disk SHA-512 and size; HTTPS-only resumable download bounded to the pin; pure-Rust gzip bounded to the pinned disk size; manifest written from the pin; atomic install. Release builds report any installed image that does not match the pin as invalid and re-hash the disk once per session before cloning it; never a generic Debian fallback |
| Action → execution | untrusted | `pegoles-policy::evaluate` (exhaustive, deterministic) on every action, then Core's executor (rate limit, control arbitration) |
| Guest → host | hostile | vsock frames ≤ 64 KiB, UTF-8, typed parse, bounded fields; frame reassembly caps; per-connection line-rate limit; bounded queues everywhere between guest and Core |
| Guest process → host channel | hostile | the helper accepts a vsock peer only from a reserved source port (≤ 1023), which only the runtime (CAP_NET_BIND_SERVICE via its unit) can bind |
| Webview → Core | semi-trusted UI | app ACL (`build.rs` + `capabilities/default.json`): only the commands the release UI invokes, event listen/unlisten, window dragging; no path/URL/process arguments; navigation away from the app origin refused; strict CSP in release; no network egress at all from the webview (WebKit content rule list blocking every network URL, attached before the window exists; WebRTC removed; `webview_egress.rs`, proven by `examples/webview_egress_probe.rs`); Design Lab and diagnostic commands compiled and granted only in debug builds; switching to a cloud planner and storing an API key need a native macOS alert the page cannot answer (Cancel is the keyboard default; 30 s cooldown after a decline), and the key is typed into the alert's secure field, never the webview |
| Host → model provider | external service | HTTPS only; sends the objective, screenshots of the VM and the model's own history; never host files, host screen, or the key in content |

## Invariants (enforced)

1. **No model-to-host execution path.** There is no host shell, file,
   process, or URL action anywhere in the protocol. The action
   vocabulary is observe / pointer / keyboard / wait inside the VM
   (`pegoles-protocol/src/actions.rs`); unknown `type` tags fail to
   deserialize. Removed in 2026-09: `shell`, `read_file`, `write_file`,
   `open_url` (they were never implemented in the guest and used to
   report false success).
2. **Every action passes policy.** `evaluate` is an exhaustive match
   (a new variant does not compile without a rule): unit-square
   coordinates, drag/scroll/wait/text caps, key vocabulary, no control
   characters in typed text, a key-material tripwire. Waits pass
   policy and audit too (`begin_wait`/`end_wait`). `RequireApproval`
   is reserved and treated as a denial (fail closed): no current action
   produces it.
3. **The VM has no network device, no shared folders, no clipboard, no
   host input devices** (`VmManager.swift::buildConfiguration`). With no
   network, nothing the agent does inside the VM can have an external
   side effect; that is why in-VM actions need no approval prompts.
4. **The guest is untrusted.** All guest frames are parsed as data with
   bounds; malformed or oversized input drops the connection, never the
   host (a char-boundary panic in error truncation was fixed and
   regression-tested). Frames larger than 64 MiB or with inconsistent
   stride are rejected. A guest that stops reading, floods, or hangs is
   disconnected; a helper that stops answering is killed (and its VM
   with it) instead of wedging the app.
5. **Only the runtime can speak for the guest.** Reserved-port peer
   authentication (above) plus a non-dumpable runtime process
   (`PR_SET_DUMPABLE 0`) and Yama `ptrace_scope=2` mean an app running
   inside the guest cannot impersonate the runtime, forge
   acknowledgements, tamper with the compositor, or read typed input.
   The agent's terminal additionally has no vsock sockets and no device
   nodes beyond the basics (no `/dev/uinput`). An app can still kill the
   runtime (same user): the host sees the disconnect, systemd restarts
   it without a start limit, measured recovery ≈ 3 s.
6. **Control ownership is checked atomically.** Agent input reaches the
   guest only while the agent owns control, checked under the same lock
   as the dispatch; any path that takes control from the agent (pause,
   stop, failure, human takeover) cancels its run. Typed text is sent in
   64-character slices so Stop interrupts a long paste within ~1 s, and
   Stop/Pause/Take Control cancel the run before waiting for the state.
7. **Reset is a real reset**: fresh disk clone and a fresh EFI variable
   store (a guest root cannot persist boot entries through NVRAM).
8. **No orphan VMs.** The helper exits on stdin EOF and on parent death
   (kqueue process watch), stopping every VM it owns first.
9. **Host filesystem.** VM state lives only under
   `~/Library/Application Support/Pegoles` (0700 dirs, 0600 disks).
   The sealed image is read-only; each computer gets an APFS clone.
   Image downloads are HTTPS-only, archive members must be regular
   files (no symlink following), extraction ignores archive ownership
   and modes. A normal create boots only a sealed Pegoles image; booting
   plain Debian is a debug-build image-builder path.
10. **Secrets.** The Anthropic key is typed into a native secure field
   (never the webview's DOM), stored in the macOS Keychain (service
   `dev.pegoles.agent`) or read from `ANTHROPIC_API_KEY`; it is never
   returned to the webview, never logged (`Debug` redacts it), never
   placed in model context, never sent to the guest or the local worker,
   and the Anthropic client follows no redirects (the key header only
   ever goes to the compiled-in endpoint). No
   credentials are baked into the guest image (root and `pegoles`
   accounts are locked; sshd and its vsock/unix socket activation are
   masked).
11. **Child processes** get a scrubbed environment (`env_clear`) and fixed
   arguments: the VM helper, the MLX worker and `/usr/bin/tar` (image
   builder path).
12. **Local models are planners, not principals.** Pegoles Local goes
   through the same `Planner` → runner → Core executor → Policy path as
   the cloud planner; a local model is not trusted because it is local.
   Its worker returns text only; it never receives VM, helper, policy,
   filesystem or secret access. Deterministic brakes live outside the
   model: one action per step, turn/action/time budgets, a loop brake
   (the same action on an already-seen screen three times stops the
   task, including A-B-A-B toggling), bounded invalid-output retries.
   The only thing a fully malicious local model can do is what any
   planner can: type, click and look inside the disposable offline VM.

## Prompt injection

Screen content is data. The system prompts (cloud and local) say so,
and Anthropic's server-side classifiers flag injections in
screenshots, but none of that is relied on (small local models follow
instructions on screen more readily; see the `injection` task in
`benchmarks/local-models`): the policy and the missing network bound what a fully
injected model can do to "type, click and look inside a disposable,
offline VM". Reset returns the VM to the sealed image.

## Residual risks (known, accepted for now)

- A compromised guest can kill or starve the runtime (denial of service
  of the agent, not host access).
- Weston runs with `--debug` (required for `weston_capture_v1`); any
  guest client of the same user can capture the guest screen.
- The GUI apps and the runtime share one guest user; a dedicated runtime
  user (Wayland access through a group) is future work.
- The key-material tripwire in policy is a heuristic, not a boundary.
- Hypervisor escape is out of scope (Apple Virtualization.framework is
  the boundary).
- Windows (HCS) is implemented on the `phase/windows-0.2` branch and
  exercised only on CI runners (Windows Server, as an administrator),
  never on a consumer PC; its properties are designed and unit-tested,
  not hardware-verified. See "Windows" below and
  `docs/WINDOWS_ARCHITECTURE.md`.
- The MLX worker's sandbox uses `sandbox-exec`, which Apple marks
  deprecated; it can still read files outside `$HOME` (system
  libraries, `/opt`) and file metadata under `$HOME`, and can fill its
  private temp dir while it runs (it is emptied on the next spawn). A
  compromised worker (e.g. malicious weights exploiting a parser) could
  lie to the planner — which is already untrusted — but has no network,
  no other process and no app launch to exfiltrate through.
- The bundled Python runtime is signed with the app; its hardened
  runtime (library validation) can only be exercised with a Developer ID
  build (ad-hoc local builds cannot use it).
- The release `sign` job trusts the unsigned bundle produced by the
  release `build` job of the same commit (a dependency compromised at
  its locked version could alter what gets signed; it can no longer use
  the signing identity).

## Windows (implemented, not hardware-verified)

The same boundaries, with Windows mechanisms (`docs/WINDOWS_ARCHITECTURE.md`):

| Boundary | Enforcement on Windows |
|---|---|
| App → hypervisor | The app never runs elevated. Host Compute System calls need administrator rights, so they live in `PegolesVmBroker` (LocalSystem, demand-start, per-machine install): eight typed verbs over a local named pipe, clients limited to Pegoles' own `pegoles-vm-host.exe`, paths limited to the calling user's own computer folder (no reparse points, opened while impersonating the user), the HCS document built by the broker from validated values with no network adapter, shared folder, keyboard or mouse (the guest's own synthetic display only; never shown on the host, no input through it). VMs die with the helper's connection and with the broker. |
| Guest process → host channel | The guest runtime listens on privileged vsock port 850 (only it holds `CAP_NET_BIND_SERVICE`); the unprivileged helper connects over AF_HYPERV. HvSocket access is limited to SYSTEM and the signed-in user by the VM's own security descriptor (no registry registration). Guest bytes never reach the SYSTEM service. |
| Local model worker → host | AppContainer with no capabilities (no network, no user files, read-only access granted to the model store only), child processes forbidden, job object (dies with Pegoles, one process, memory cap, UI restrictions), explicit handle list, rebuilt environment. |
| Webview → network | WebView2 switches set before the webview exists: every proxied request goes to a dead proxy on the loopback discard port (loopback included), every host name resolves to nothing, WebRTC may not use UDP outside a proxy; WebRTC constructors removed; navigation guard as on macOS. |
| Cloud planner and key | Not available on Windows yet: switching to a cloud planner and storing a key need a native confirmation the page cannot answer, which exists only on macOS, so both fail closed. Pegoles Local is the only planner on Windows. |
| Installer | Per-machine NSIS installer (one UAC prompt). It registers the broker service and nothing else: no firewall rule, no Defender exclusion, no Windows feature. The Virtual Machine Platform is turned on later from onboarding, after an explanation, through a fixed elevated verb (`dism.exe` with fixed arguments); Pegoles restarts Windows only when the person presses "Restart now". Unsigned today: SmartScreen warns and Smart App Control blocks it; Pegoles never asks anyone to turn those off. |

## Tests that pin these properties

`pegoles-policy` (vocabulary sweep, caps, control chars, tripwire),
`pegoles-protocol` (forbidden verbs never deserialize),
`pegoles-computer` (hostile frames, bounded helper lines, https-only,
symlinked base, fail-closed create, reset restores disk),
`pegoles-core` (policy short-circuit, stuck-input release, agent session
vs display), `pegoles-agent` (translation errors, batch halt, budgets,
cancellation, append-only history, real executor + policy with Mock;
local parser: hostile coordinates, schema escapes, NaN/Infinity, bidi,
multiple calls; local planner: garbage output fails closed, loop and
oscillation brakes, crash restart, cancel during inference),
`pegoles-inference` (store: resume, corruption, oversize, symlinks,
extra files, remote-code JSON, staged files never trusted by size;
worker supervisor: crash, garbage, oversized reply, hang, unhonored
cancel, closed stdout, stalled stdin, stray prints; sandbox escapes
against the real runtime), `pegoles-computer` image distribution
(tampered/oversized/corrupt archives, decompression to other bytes,
resume, mirrors, cancel, pins enforced at boot),
`pegoles-agent/tests/security_matrix.rs` (a planner that follows prompt
injection through the real executor and policy), `apps/desktop` (ACL
equals the invoked command set, native consent, XSS regressions), the hardware E2E
`crates/pegoles-agent/examples/agent_e2e.rs`, the hostile-model run
`local_bench --safety` (real VM + policy) and the keyless local E2E
`apps/desktop/src-tauri/examples/local_e2e.rs`.
