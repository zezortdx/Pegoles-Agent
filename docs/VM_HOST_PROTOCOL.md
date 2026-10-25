# VM Host Protocol (Rust <-> pegoles-vm-host)

Narrow, auditable IPC between `MacOSVirtualizationBackend` (Rust) and
`native/macos/pegoles-vm-host` (Swift). Transport: **JSON Lines over the
child process stdin/stdout** — one JSON object per line in each direction.
Diagnostics go to stderr (never parsed).

Rust mirror: `crates/pegoles-computer/src/vmhost_proto.rs`.
Swift mirror: `native/macos/pegoles-vm-host/Sources/Protocol.swift`.
The two must stay field-compatible; `vmhost_proto.rs` tests lock the shape.

## Requests (Rust -> Swift)

`{ "id": <u64>, "command": "<name>", ... }`

| command | fields | reply |
|---|---|---|
| `version` | — | `{ok, version}` |
| `validate` | `params` | `{ok, state:"stopped"}` or `validation_failed` |
| `create` | `params` | `{ok, state:"stopped"}` |
| `start` | `computer_id` | `{ok, state:"running"}` |
| `pause` | `computer_id` | `{ok, state:"paused"}` |
| `resume` | `computer_id` | `{ok, state:"running"}` |
| `stop` | `computer_id` | `{ok, state:"stopped"}` (graceful `requestStop` first, forced `stop` fallback) |
| `state` | `computer_id` | `{ok, state}` |
| `destroy` | `computer_id` | `{ok, state:"stopped"}` |
| `egress_open` | `computer_id`, `endpoint` | `{ok}` or `egress_failed` / `unknown_computer` / `invalid_params` |
| `egress_close` | `computer_id` | `{ok}` (idempotent) or `unknown_computer` |

`params` (create/validate):

```json
{
  "computer_id": "…",
  "disk_path": "…/disk.img",
  "efi_vars_path": "…/efi-vars.bin",
  "machine_id_path": "…/machine-id",
  "serial_log_path": "…/logs/serial.log",
  "vcpus": 2,
  "memory_mb": 1536
}
```

## Responses (Swift -> Rust)

`{ "id": <u64>, "ok": true, "state": "<vm_state>" }`
`{ "id": <u64>, "ok": false, "error": { "code": "<code>", "message": "…" } }`

Error codes: `unknown_command`, `unknown_computer`, `already_exists`,
`invalid_params`, `validation_failed`, `start_failed`, `stop_failed`,
`pause_failed`, `resume_failed`, `not_entitled`, `guest_unavailable`,
`egress_failed`, `internal`.

## Async events (Swift -> Rust, no `id`)

- `{ "event": "vm_state_changed", "computer_id": "…", "state": "…" }`
- `{ "event": "vm_failed", "computer_id": "…", "message": "…" }`

Emitted from `VZVirtualMachineDelegate` (`guestDidStop`,
`didStopWithError`). Rust applies them to its cached state while waiting
for command responses.

- `{ "event": "egress_closed", "computer_id": "…", "reason": "…" }` — the
  egress stream ended by itself (`guest_closed`, `app_closed`,
  `read_error`, `write_error`). Never sent in reply to `egress_close`,
  stop or destroy (the app caused those). See below.

## Egress stream (`egress_open` / `egress_close`)

Transport only, for `docs/EGRESS.md`. The app creates a private local
endpoint FIRST, then sends `egress_open`; the helper connects the guest's
forwarder (vsock port 4051) and the endpoint, replies `{ok}`, and pumps
raw bytes both ways with one fixed buffer per direction (no queues, no
parsing, no payload logging). The app accepts exactly one connection
(10 s) and the endpoint disappears. Either side closing closes both.

- `endpoint` on macOS: absolute path `<Pegoles data dir>/egress/<id>/s`,
  a Unix socket in a fresh 0700 directory (not in the computer folder:
  `sun_path` is 104 bytes on macOS). The Swift helper derives the data dir
  from the computer's own folder, never from the request, and rejects
  anything else (other directories, `..`, symlinks, sockets or directories
  not owned by this user, directories not 0700, non-sockets).
- `endpoint` on Windows: exactly `\\.\pipe\pegoles-egress-<lowercase uuid>`,
  a named pipe the app created (current-user-only DACL,
  `PIPE_REJECT_REMOTE_CLIENTS`, first instance). The helper connects to
  the guest service `{00000fd3-facb-11e6-bd58-64006a7986d3}` (port 4051,
  Linux hv_sock pattern). The HCS document already allows it: its two
  default HvSocket descriptors cover every service id.
- At most one stream per computer (`egress_failed` otherwise); the
  computer must be running. `egress_failed` also covers a guest that is
  not listening or an endpoint the helper cannot reach.
- The stream also ends on `stop`, `destroy`, helper exit and VM loss. The
  VM keeps NO network device (asserted in `VmManager.buildConfiguration`;
  `egress::tests::the_vm_never_gets_a_network_device` guards the source).
- Port 4051 is duplicated in `EgressBridge.swift` and
  `pegoles-computer::egress::EGRESS_VSOCK_PORT`; canonical:
  `pegoles-egress-proto`.

## State mapping

Vz `starting/pausing/resuming` -> `"starting"`; `stopping` -> `"stopping"`;
`saving/restoring` (unreachable, no save/restore commands) -> `"starting"`;
the rest map 1:1. Documented in both mirrors.

## Closed command set

There is no shell execution, no host filesystem operation, no network
proxy, no arbitrary native call — and there must never be. Unknown
commands are rejected with `unknown_command`; malformed lines without an
`id` are dropped with a stderr note; malformed lines with an `id` get an
`invalid_params` error envelope. `vmhost_proto.rs::no_shell_command_exists`
is a guard test against protocol creep.

## Cross-platform parity (Phase 3.5)

The command SET above is hypervisor-independent by design and does not
change on Windows: the future `pegoles-vm-host.exe` answers the same
`version/validate/create/start/pause/resume/stop/state/destroy` plus
`guest_send/guest_status/guest_disconnect` and
`egress_open/egress_close`. Backend specifics travel in
capabilities and optional fields (e.g. `CreateParams.seed_iso_path`,
`HostResponse.connected`) — there will never be `windowsStart` vs
`macStart` forks. Guest channel events (`guest_connected/guest_frame/
guest_disconnected`) have the same shape on virtio and Hyper-V sockets;
only the transport underneath differs (`WINDOWS_BACKEND.md`).

## Guest connection rules (0.1 hardening)

- A computer has at most one live guest connection. A new connection from
  the reserved port range supersedes the old one: the helper shuts the old
  socket down (SHUT_RDWR) before closing it and emits
  `guest_disconnected` with reason `superseded` before `guest_connected`,
  in that order. Core then requires a fresh `GuestHello` handshake; the
  new connection inherits nothing from the old one.
- A `GuestHello` repeated on an already greeted connection is a protocol
  violation (kick + error), not a new handshake.
- Per guest link (not per connection): a line token bucket (10k lines/s,
  20k burst) on a monotonic clock, and a rate limit on accepts (burst 8,
  0.5/s), so reconnecting cannot refill either.
- Rust bounds each pump by lines (2048) and bytes (8 MiB), keeps at most
  64 queued host actions per connection (overflow: kick + error), never
  sends from inside another helper call, and gives every helper call one
  overall deadline that guest traffic cannot extend.

