# Security

These invariants are architectural, not aspirational. Violating any of them is a bug.

## 1. Model-generated commands never execute directly on the host

There is no code path from a model (or an action) to `std::process::Command` on the host. `Shell` means "inside Pegoles Computer". The`ComputerBackend` trait exposes lifecycle operations only (`start/stop/pause/…`), never host execution.

## 2. All agent actions pass through structured actions

The future LLM may only emit `ComputerAction` (`crates/pegoles-protocol/src/actions.rs`). Raw shell strings from a model are data until classified — never commands. Any PR adding `HostShell` / `ExecuteOnHost` / `RawHostCommand` must be rejected.

## 3. Every structured action passes through Pegoles Policy

`pegoles-policy::evaluate()` is deterministic code, not AI. `Allow` / `Deny` / `RequireApproval` is computed from the action + `PolicyContext` before anything runs.

## 4. Pegoles Computer is treated as untrusted

Guest output is data, never trusted instructions. Future guest-runtime messages must be validated against `pegoles-protocol` schemas and capability-checked.

## 5. Host filesystem is inaccessible unless an explicit Host Bridge capability is granted

No `HostBridge` exists in Phase 1. `VirtualPath` (guest path) and host paths are different concepts; `is_host_path()` denies `/Users/`, `/etc/`, `~`, `C:\`, `..` escapes, and `.ssh` access. A future bridge would be an explicit, auditable, per-grant capability — not a default.

## 6. Secrets must never be placed directly in LLM context

Policy denies typing or reading credential patterns (`AWS_SECRET*`, `PRIVATE KEY`, `*.pem`, `*_TOKEN`). Future work: a secret vault + redaction at the Core boundary.

## 7. Network access will be policy-controlled

Phase 1: `OpenUrl` and network-adjacent shell (`curl`, `wget`, `apt`, `pip`) yield `RequireApproval`. Default-deny networking at the VM boundary arrives with the real backend.

## 8. The VM boundary is part of the security model, not merely a convenience

Isolation is enforced by the hypervisor (`Virtualization.framework` on macOS), not by good intentions in userspace. The Mock backend exists for development only and provides zero isolation — it must never be mistaken for a security boundary.

## Where each invariant lives in code

| Invariant | Enforcement point |
|---|---|
| 1, 2 | `pegoles-protocol/src/actions.rs` (enum shape + guard test); `vmhost_proto.rs::no_shell_command_exists` (wire protocol has no host execution) |
| 3 | `pegoles-policy/src/engine.rs` (`evaluate`) |
| 4 | `pegoles-computer/src/traits.rs` (trait docs), guest output treated as data |
| 5 | `VirtualPath` type + `is_host_path()` in policy; Phase 2: no shared directories, no clipboard, VM data confined to Application Support |
| 6 | `looks_like_secret()` in policy |
| 7 | `classify_shell()` + `OpenUrl` rule; Phase 2: VM has no network device attached |
| 8 | `MacOSVirtualizationBackend` + `native/macos/pegoles-vm-host` (hypervisor isolation; Mock is dev-only) |

## Phase 2 notes

- `pegoles-vm-host` is a lifecycle-only helper: its command set is
  `version/validate/create/start/pause/resume/stop/state/destroy`. Any other
  command is rejected. The model cannot reach this channel.
- Spawning `/usr/bin/tar` (verified-archive extraction) and the `curl`-free
  `ureq` download in `image.rs` are first-party installer behavior with
  fixed arguments — not model-generated host execution (invariant 1 covers
  model actions; no model exists yet and no path will be added).
- Helper crash/disconnect forces backend state to `Error`
  (`BackendDisconnected`); Core never shows a stale Running.
- No sudo is required at any point.

## Phase 3.5 notes: Windows threat model

The same invariants hold on every host; only the mechanism names change:

- **Hyper-V VM = security boundary** (same role as the
  Virtualization.framework VM on macOS). Guest escape is out of scope;
  hypervisor isolation is the assumption, not userspace goodwill.
- **Hyper-V socket = untrusted guest input boundary** (same role as the
  virtio socket). Every frame is length-bounded, version-checked, and
  parsed as data. A compromised guest gains NO access to: host
  filesystem, Windows credentials, registry, user profile, DPAPI, SSH
  keys, browser profile, or arbitrary host process execution — the
  socket is a protocol interface, not a privilege passage.
- **No HCS process execution, ever.** HCS can spawn processes inside
  compute systems; Pegoles must not use that as an LLM shortcut. Flow
  stays Model → Structured Action → Policy → Guest Protocol → Guest
  Runtime → guest action. Never Model → host process (any OS).
- **Privileged setup ≠ privileged runtime.** Socket-service registration
  and group membership are one-time installer steps; the runtime only
  verifies. Pegoles never runs as Administrator for daily use and never
  silently changes group membership.
- Socket-service GUIDs are derived deterministically from the logical
  port (`hyperv_service_guid_for_port`, tested) — never scattered
  hardcoded GUIDs, never registry writes from the runtime.

## Phase 5 notes: Eyes & Hands (structured guest input + observation)

Computer input targets the guest only:

- The ONLY input path is `ComputerAction` → Policy → `ComputerInputBackend`
  → vsock `HostMessage::Input`/`GetFrame` → guest runtime → compositor
  devices (uinput touchscreen/keyboard, Wayland screenshooter client).
- No host input API exists: no global event synthesis, no host process
  control, no clipboard bridge (typing uses the guest input path;
  `input.rs::no_host_input_or_capture_apis` fails the build on
  `CGEvent`/`SendInput`/clipboard tokens).
- No host screen capture API exists: `ObserveScreen` captures the VM
  framebuffer inside the guest; pixels never include host windows
  (`stride_consistent` + dimension caps enforced on receipt).

Observation captures the guest only:

- Frames carry metadata (`ObservedFrameMeta`: id, timestamp, dimensions,
  encoding) in events; pixels travel out-of-band and are cached with
  dedup (`FrameCache`), never streamed at 60 fps.

Control ownership prevents human/agent conflicts:

- `ControlArbiter`: agent acts only under `None`/`Agent`; a human
  `take_control` from `Agent` cancels the agent sequence and releases
  pressed state BEFORE granting `User` (never simultaneous).
- Cancellation (takeover, pause, stop, destroy, shutdown) always
  releases held buttons/modifiers on both host (`PressedState`) and
  guest (runtime `release_all` on disconnect/drop).

Actions are structured and pass policy:

- New pointer/keyboard/observation actions are allow-listed in
  `pegoles-policy` with static caps (unit square, drag/scroll/text/wait
  limits, key vocabulary); out-of-range or unknown input is `Deny`.
- The guest re-validates every op (`validate_op`) and advertises
  capabilities (`input`/`frame` in `GuestHello`); old guests get an
  honest `UnsupportedOperation`, never a silent pretend.
- The guest wire stays free of shell/file/process verbs
  (`no_capability_creep_in_v01` guard, Phase 5 allowlist).

## Phase 3.6 review (Windows real code + performance)

Re-reviewed with the HCS backend, Hyper-V transport, setup tool, and
governor in place. No new host capabilities were introduced:

- The HCS backend exposes lifecycle + vsock only. `HcsCreateProcess`
  exists in the API surface but is NEVER called — no code path exists
  from model/action/policy to guest process creation, on either OS.
  (`direction_tests` forbids shell-out patterns in `windows.rs`.)
- The setup tool's registry write is confined to one subkey
  (ElementName under our own service GUID), requires elevation, explains
  before acting (`explain`), and verifies after writing. It cannot run
  arbitrary commands by construction (fixed command set, no args).
- Hyper-V socket frames are untrusted guest input with the same bounds
  as virtio (64 KiB, UTF-8, version-checked, unknown types ignored).
- `ResourceGovernor`, `IdlePolicy`, balloon policy, and `bench.sh` read
  host facts (sysctl/disk/process table) and never change host state.
- Builder tooling (`qemu-img`, `debugfs` read-only, Docker) runs on the
  build machine only; the Windows runtime needs none of it.
- VHDX non-determinism note: container bytes embed creation metadata,
  so verification is convert+gate+boot, never cross-build byte compare.
  Hashes always describe the bytes on disk, never an expectation.
