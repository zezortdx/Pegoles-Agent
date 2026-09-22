# Guest Runtime v0.1 (`pegoles-guest-runtime`)

Tiny Linux service (guest side of `GUEST_PROTOCOL.md`). ~700 KB static-ish
binary, three deps (`serde`, `serde_json`, `libc`). Deliberately absent:
GUI, browser, LLM, database, HTTP server, shell, filesystem access,
process execution, network use.

## Behavior

1. Dial host CID 2, port 4050 (`connector::resolve_host_endpoint()` —
   hypervisor-blind; identical on virtio, Hyper-V, vhost).
2. Send `GuestHello` first, always. Expect `HostHello` with matching
   version (mismatch → report `incompatible`, drop, backoff).
3. Send `Ready`. Answer `Ping`→`Pong` (nonce echoed) and
   `GetSystemInfo`→`SystemInfo` (allowlist facts only).
4. Any EOF / malformed frame / host `Error` → drop and reconnect with
   capped backoff (1,2,4,…30 s; reset after a 60 s+ healthy connection).
5. SIGTERM (systemd stop) just ends the process; stateless reconnect
   makes that safe. No state files, no local DB.

## systemd

Unit: `guest/runtime/pegoles-guest-runtime.service`. `User=pegoles`
(system user, nologin), `Restart=on-failure`, `RestartSec=5s`.
Hardening reviewed flag by flag (all compatible with AF_VSOCK + read-only
OS facts; `RestrictAddressFamilies` deliberately NOT set — AF_UNIX and
AF_VSOCK must both work, and older systemd lacks AF_VSOCK in that
directive; re-scope if a future systemd supports it):

`NoNewPrivileges`, `PrivateTmp`, `ProtectHome`, `ProtectSystem=strict`,
`ProtectKernelTunables/Modules`, `ProtectControlGroups`,
`LockPersonality`, `RestrictSUIDSGID`, `RestrictRealtime`,
`CapabilityBoundingSet=` (empty), `ReadOnlyPaths=/etc/os-release /etc/hostname`.

Starts `After=systemd-user-sessions.service`; needs no network
(`After=` has no network target on purpose — Ready with zero functional
interfaces is an architecture proof).

## Build (cross)

Documented in `scripts/build-guest-image/build-runtime.sh`: Docker
`rust:bookworm` (native arm64 on Apple Silicon — no cross toolchain on
the Mac), `cargo build --target aarch64-unknown-linux-gnu --release`.
CI checks the same target via `cargo check` (no linker needed). The macOS
app never builds this; it boots the derived image containing it.

## `--version`

`pegoles-guest-runtime --version` prints runtime + protocol versions
(used by the image builder to verify the staged binary).
