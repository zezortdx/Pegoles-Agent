# Guest Runtime (`pegoles-guest-runtime`, 0.2.0 in image v0.3)

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

Unit: `scripts/build-guest-image/seed/units/pegoles-guest-runtime.service`
(the image's copy; `guest/runtime/pegoles-guest-runtime.service` is kept
byte-identical and a guest-proto test fails if they drift). `User=pegoles`,
`Restart=always`, `RestartSec=2s`, `StartLimitIntervalSec=0` (a guest app
killing the runtime in a loop never leaves the unit failed).

Its only capability is `CAP_NET_BIND_SERVICE` (ambient and bounding set):
it dials the host from a reserved vsock source port (<= 1023), and the
helper accepts no other peer, so no other guest process can impersonate
it. Hardening: `NoNewPrivileges`, `PrivateTmp`, `ProtectHome`,
`ProtectSystem=strict`, `ProtectKernelTunables/Modules/Logs`,
`ProtectControlGroups`, `ProtectClock`, `ProtectHostname`,
`LockPersonality`, `RestrictSUIDSGID`, `RestrictRealtime`,
`RestrictNamespaces`, `RestrictAddressFamilies=AF_VSOCK AF_UNIX`,
`SystemCallArchitectures=native`, `MemoryDenyWriteExecute`, `UMask=0077`,
`ReadOnlyPaths=/etc/os-release /etc/hostname`. The process also sets
`PR_SET_DUMPABLE 0` itself.

Starts after `pegoles-weston.service` (it captures the screen and injects
input through Weston and uinput); needs no network (the VM has none).

## Build (cross)

Documented in `scripts/build-guest-image/build-runtime.sh`: Docker
`rust:bookworm` (native arm64 on Apple Silicon — no cross toolchain on
the Mac), `cargo build --target aarch64-unknown-linux-gnu --release`.
CI checks the same target via `cargo check` (no linker needed). The macOS
app never builds this; it boots the derived image containing it.

## `--version`

`pegoles-guest-runtime --version` prints runtime + protocol versions
(used by the image builder to verify the staged binary).
