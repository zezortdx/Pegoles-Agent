# Guest Protocol v1 (Core ↔ Guest Runtime)

Versioned, framed, capability-free control plane between Pegoles Core and
`pegoles-guest-runtime` over a vsock transport. This is a DIFFERENT
protocol from host↔VM-helper (`VM_HOST_PROTOCOL.md`): that one drives the
hypervisor; this one talks to the OS inside.

```text
┌─────────────────────────────┐
│ Pegoles Core                │
└──────────────┬──────────────┘
│
Rust ↔ Swift IPC (VM_HOST_PROTOCOL.md)
│
┌──────────────▼──────────────┐
│ pegoles-vm-host             │
│ VZVirtioSocketDevice        │
└──────────────┬──────────────┘
│
VSOCK (AF_VSOCK, port 4050)
│
┌──────────────▼──────────────┐
│ Debian 13                   │
│ pegoles-guest-runtime       │
└─────────────────────────────┘
```

## Transport

- Guest dials host (CID 2, port **4050**, `PEGOLES_VSOCK_PORT` — single
  source of truth in `pegoles-guest-proto`; no magic numbers).
- Host listens (`VZVirtioSocketListener` today; AF_HYPERV + service GUID
  on Windows tomorrow — same port, see `WINDOWS_BACKEND.md`).
- Framing: JSON Lines, one UTF-8 line per message, **64 KiB cap**
  (`MAX_FRAME_BYTES`). Partial reads accumulate; batched reads split;
  oversized/invalid frames close the channel; disconnect mid-frame emits
  nothing. Implemented once in `framing.rs`, used by both ends.
- Serial console is diagnostics only — never the control plane.

## Messages (v0.1, `protocol_version: 1`)

Guest → Host: `guest_hello {protocol_version, runtime_version, os,
os_version, arch}` (MUST be first on every connection), `ready`,
`pong {nonce}`, `system_info {...}`, `error {code, message}`.

Host → Guest: `host_hello {protocol_version}`, `ping {nonce}`,
`get_system_info`, `error {code, message}`.

There is intentionally no shell, filesystem, process, browser, install,
or any other capability in v0.1. A guard test (`no_capability_creep_in_v01`)
fails the build if one appears. Unknown `type` values are ignored
(forward compatibility); malformed JSON kills the session, never the host.

## Handshake (readiness signal)

```text
guest connects → GuestHello → version check → HostHello → Ready
```

`Ready` requires ALL of: transport connected + valid GuestHello first +
`protocol_version == 1` + `Ready` seen, within 60 s of VM start
(`GUEST_READY_TIMEOUT`). Anything else is Waiting/Connecting/
Disconnected/Incompatible/Error — tracked in `GuestRuntimeState`,
separate from `ComputerState::Running`.

## Heartbeat, timeout, reconnect

- Host pings every 10 s (`HEARTBEAT_INTERVAL`); guest echoes the nonce.
- 3 missed windows (30 s silence) → `Disconnected`. The VM may keep
  running; the states are independent by design.
- Guest reconnects with capped backoff (1…30 s); Core re-handshakes to
  Ready without rebooting the VM. Pause/resume may kill the channel on
  some hypervisors — the contract tolerates it; the protocol never
  assumes transport survival across lifecycle transitions.

## SystemInfo (allowlist only)

`os, os_version, kernel, arch, hostname, runtime_version,
protocol_version`, optional `uptime_s, cpu_count, mem_total_mb`. Never:
home contents, environment, credentials, tokens, command lines,
personal data (guard-tested on both ends).

## Failure containment

The guest is untrusted input: field lengths bounded (256 B hello
identity, 4 KiB info fields), unknown types ignored, malformed frames
kick + Error, version mismatch → `Incompatible` (never silent). A
compromised guest gets: no host files, no credentials, no host
processes, no host config — the channel is a protocol interface, not a
privilege passage.

## Phase 5: input + frame capture (protocol version stays 1)

Capability advertisement (backward compatible both ways):

- `GuestHello` gains optional `capabilities: ["input", "frame"]`
  (bounded: ≤8 entries, ≤32 B each). v0.1 hellos parse unchanged;
  old hosts ignore the unknown field.
- The host requires `Ready` + the matching capability before sending;
  otherwise it fails fast with `UnsupportedOperation` (never a blind
  5 s timeout against a v0.1 guest).

Input (`HostMessage::Input{request_id, op, display?}` → `InputAck`):

- Ops carry GUEST pixels (host converts agent 0–1 space) and canonical
  key names; every field is bound-checked on BOTH ends (host policy +
  `GuestInputOp::is_bounded` + guest `validate_op`).
- The guest answers every input with `InputAck{request_id, ok, error?}`;
  unknown keys/chars/geometry are acked `ok:false`, never executed.
- Drags interpolate guest-side over `duration_ms` (≤10 s cap); scroll
  uses logical units (+y = content down); chords hold ≤4 keys.

Frames (`HostMessage::GetFrame{request_id}` → `FrameBegin` + chunks):

- The guest captures its OWN framebuffer (`weston_capture_v1` client
  against weston 14; the retired `weston_screenshooter` protocol is gone),
  answers `FrameBegin{width, height, total_chunks}` then base64 raw-RGBA
  chunks (≤32 KiB raw each, ≤1024 chunks, 64 MiB total cap).
- Capture failures ride `InputAck{ok:false}` on the same id so the host
  fails fast instead of timing out a 30 s transfer.
- Host reassembles, stride-checks (`w×h×4`), dedups (`FrameCache`),
  and emits `FrameObserved` metadata (pixels stay out-of-band).
