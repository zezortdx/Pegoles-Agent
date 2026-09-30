# Internet access for the agent's computer (egress)

Status: in development (branch `phase/windows-0.2`). This document is the
contract between the pieces; change it before changing the code.

## Goals and non-goals

- The agent's computer can reach the web **only** during a task the person
  started with internet enabled, and only through Pegoles' host-side proxy.
- The VM still has **no network device** (macOS: no `VZNetworkDeviceConfiguration`;
  Windows: no network adapter in the HCS document). There is no NAT, no DNS
  and no raw socket path out of the guest. The only way out is one
  byte stream over vsock/HvSocket that the host opens and closes.
- Every request is decided by the host, by rules compiled into the signed
  binary (the "immutable protocol" below). The guest, the model, the
  webview and settings files cannot change those rules.
- Non-goal: making the web safe for the person's own browsing. This is a
  containment boundary for an untrusted agent in a disposable VM.

## Modes (chosen per task, off by default)

| Mode | What passes | Consent |
|---|---|---|
| `off` (default) | nothing; the egress channel is never opened | none |
| `allowlist` | only hosts under the domains the person typed for this task (max 32 entries, registrable domains or subdomains, validated, no wildcards, no IP literals) | native confirmation listing the domains |
| `open_web` | any public host, minus the always-deny layer | native confirmation with an explicit warning; the webview cannot answer it (same pattern as the cloud-planner consent: Cancel is the default, back-off after declines) |

The mode lives with the task, not in a settings file. Stopping, pausing,
failing, human takeover, or the end of the task closes the egress channel
(kill switch). A new task starts at `off`.

## Architecture

```text
guest: Chromium ──HTTP proxy──▶ 127.0.0.1:3128 (pegoles-egress-forwarder, user pegoles-egress)
                                  │ mux (pegoles-egress-proto) over AF_VSOCK, guest LISTENS on port 4051
host:  pegoles-vm-host (Swift on macOS / Rust on Windows) connects to guest 4051 when told,
       bridges the raw bytes to a local endpoint owned by the app
       (macOS: Unix socket in a 0700 dir; Windows: named pipe, current-user DACL,
        PIPE_REJECT_REMOTE_CLIENTS, first instance)
app:   pegoles-egress (Rust, tokio) — mux, HTTP proxy, TLS interception,
       immutable policy, response inspection, audit
       ──▶ internet (rustls + webpki-roots upstream, TLS 1.2+)
```

The helpers are transport only (no decisions), like the guest control
channel. The host always initiates: no egress stream exists unless the app
asks the helper to connect, and dropping it cuts every guest connection.

### Mux protocol (`crates/pegoles-egress-proto`, no dependencies, shared by host and guest)

Frame: `stream_id: u32 BE | kind: u8 | len: u32 BE | payload[len]`,
`len <= 16384`. Kinds:

| kind | dir | payload |
|---|---|---|
| `0 HELLO` | host→guest, first frame, stream 0 | `version: u16 BE` + CA certificate DER (≤ 4096 bytes) |
| `1 HELLO_ACK` | guest→host, stream 0 | `version: u16 BE` |
| `2 OPEN` | guest→host | empty; guest allocates odd ids, strictly increasing |
| `3 DATA` | both | bytes |
| `4 CLOSE` | both | empty (half-close not supported; closes both ways) |
| `5 CREDIT` | both | `u32 BE` window increment |

Rules: protocol version 1; initial per-stream send window 262144 bytes,
sender must not exceed granted credit; at most 64 open streams; unknown
kind, oversize frame, frame for an unknown stream (other than CLOSE),
credit overflow, or anything before HELLO/HELLO_ACK → drop the whole
connection. Decoder is incremental and never allocates more than one
frame. Fuzz/proptest the decoder.

### TLS interception

A fresh CA key pair is generated **in memory** for each egress session
(rcgen, ECDSA P-256, 24 h validity, name constraints are not relied on).
Its certificate is sent in HELLO; the forwarder installs it for Chromium
(managed policy `CACertificates`, Chromium 132+, dynamic refresh, in
`/etc/chromium/policies/managed/pegoles-egress-ca.json`, the only file the
`pegoles-egress` user may write there; the directory is root-owned, so the
forwarder writes that file in place and, when the channel closes, resets it
to `{"CACertificates": []}` instead of deleting it).
The key never touches disk and dies with the session. Leaf certificates are
minted per host and cached per session. Toward the guest only
`http/1.1` is offered via ALPN. Upstream the host verifies certificates with
`webpki-roots` (no system trust store, no user overrides); a failed
verification is a deny, never a click-through.

## The immutable protocol (host policy, `crates/pegoles-egress/src/policy`)

Evaluated in this order; the first deny wins. It is an exhaustive `match`
over a closed enum of decisions; there is no runtime configuration except
the per-task mode and allowlist.

1. **Transport rules.** Only `CONNECT host:443` and absolute-form
   `http://host[:80]/…` requests. Other methods to the proxy, other ports,
   `userinfo@` in URLs, IP-literal hosts, hosts without a dot, `localhost`,
   `.local`, `.internal`, `.lan`, `.home.arpa`, `.onion`, `.arpa` → deny.
   Hostnames are lowercased, IDNA-normalized; mixed-script labels
   (homograph) → deny. Header block ≤ 32 KiB, request line ≤ 8 KiB.
2. **Resolution rules.** The host resolves names itself; every resolved
   address must be global unicast. Loopback, RFC 1918, CGNAT 100.64/10,
   link-local (incl. 169.254.169.254), multicast, broadcast, 0/8,
   documentation and benchmark ranges, ULA fc00::/7, fe80::/10, IPv4-mapped
   or -compatible forms of any of those → deny. The proxy connects to the
   checked address (no second lookup: no DNS rebinding).
3. **Always-deny layer (both modes, overrides the allowlist).** Compiled-in
   threat snapshot (malware/phishing domains and URLs from licence-compatible
   feeds; see `crates/pegoles-egress/data/README.md`), verified against a
   SHA-256 constant at startup (mismatch → egress refuses to start). In
   `open_web` also adult and gambling categories. Updated only by
   `scripts/egress/update-threat-feed.sh` + review + release.
4. **Mode rule.** `allowlist`: host must equal or be a subdomain of an
   entry. `open_web`: allowed.
5. **Response inspection (after TLS interception, every response).**
   - Executable or archive signatures (MZ/PE, ELF, Mach-O and fat, `#!`,
     ZIP/JAR/APK/Office-OOXML, 7z, RAR, gzip, xz, bzip2, zstd, CAB, MSI/OLE,
     `ar`/deb, xar/pkg, DMG/UDIF trailer, ISO) → blocked whatever the
     declared type.
   - A **download** (`Content-Disposition: attachment`, or a top-level
     type outside the web-page set) passes only if it is one of the safe
     types **and** its magic bytes agree: PDF, PNG, JPEG, GIF, WebP, plain
     text, CSV. Everything else → blocked.
   - Web-page resources (HTML, CSS, JavaScript, JSON, fonts, images,
     audio/video, WebAssembly) pass if not attachments and not caught above.
   - Downloads larger than 50 MiB → blocked.
   A blocked response is replaced with a small static 403 page.
6. **Limits.** 64 streams (a stream's host handler holds its slot until the
   handler is gone, and a guest CLOSE aborts it, so open/close loops cannot
   pile up tasks or sockets; at most 16 name lookups run at once, more are
   denied; inbound DATA is queued in full 16 KiB chunks, so queued memory
   tracks the window, not the frame count), request body ≤ 10 MiB, request body ≤ 10 MiB, idle timeout 60 s,
   connect timeout 10 s, bytes per task capped (1 GiB).

Every decision is an audit event: time, mode, host, path without query,
decision and reason code, bytes. No bodies, headers, cookies or query
strings are logged.

## Host wiring (Core, desktop app)

- **Task.** `create_task(title, internet?)` stores `{mode, domains}` on the
  task, validated by `pegoles_core::validate_internet` (the same rules as
  `Mode::allowlist`; `open_web` and `off` carry no domains). Invalid → a
  sentence, no task. The IPC carries strings only.
- **Start.** `run_task` builds the planner, then the run (`CoreComputer::prepare`)
  asks for the native confirmation *before* the computer boots (no app lock
  held, may block on the dialog), boots, takes agent control, and only then
  opens the session: the registry asks the backend for the stream
  (`open_egress`, bounded to 10 s), adopts it into tokio (Unix socket /
  overlapped named pipe) and starts `EgressSession` on a dedicated runtime
  owned by Core; the run waits for the guest's HELLO_ACK (15 s, cancellable)
  with Core unlocked. Declined, unanswerable (another dialog open, back-off,
  unsupported OS), backend refusal or handshake failure: the task runs
  offline and says so (`internet_unavailable` + a progress note). Never
  silently online.
- **Stop.** The session exists only while the agent controls a running
  computer. `cancel_agent_input` (Stop, pause, takeover, stop, reset,
  destroy), `end_agent_session` (task end, failure, cancel), a pump that
  finds control lost, registry drop and app exit all close it and then the
  backend stream. Stop, pause, takeover and exit also cut it through a
  lock-free handle so they never queue behind a guest call; the backend
  stream is closed by the next pump.
- **Audit.** Proxy events go through a bounded drop-oldest queue (256) to
  the UI as `EgressDecision` (host, allowed, reason code, bytes, time,
  `dropped_before`); the status carries the last 20 and the counters.
- **Planners.** While a session is open, both planners' prompts get one
  note: a browser can be opened from the top panel, and which sites work.

## Guest image

New sealed images (`pegoles-base-0.4` arm64, `pegoles-base-x64-0.2`):
Debian `chromium`; root-owned managed policies in
`/etc/chromium/policies/managed/pegoles.json` (full list and rationale:
`docs/DEBIAN_IMAGE.md`, "0.4 / x64-0.2: browser"): fixed proxy
`127.0.0.1:3128` (`ProxySettings.ProxyMode: fixed_servers`; the top-level
`ProxyMode` is deprecated), no QUIC, DoH off,
`DownloadRestrictions` = block dangerous, DevTools off, extensions blocked,
`file://` and `chrome://` blocked except the minimum, password manager and
autofill off, sync/sign-in off, background networking off, no
default-browser checks. `pegoles-egress-forwarder` runs as its own system
user under systemd hardening, listening on vsock 4051 and TCP 127.0.0.1:3128
only while the host holds the channel. Chromium can reach nothing else: the
VM has no network device.

## Residual risks (documented, accepted)

- In `open_web` a site not yet in the snapshot can be malicious; the
  snapshot ages until the next release.
- Text the agent types into pages can now leave the VM. The key-material
  tripwire is a heuristic; do not give the agent secrets.
- Allowed domains can host untrusted content (user uploads, redirects).
- Chromium bugs reachable from web content run inside the VM, which stays
  disposable and has no host access.
- Page resources are only signature-checked at the start of the body
  (downloads and pages alike). A malicious page can assemble bytes in
  JavaScript (`fetch`, `Blob`, `WebSocket` after a 101) and save them in the
  guest, where no signature check sees the final file. This stays inside the
  disposable VM, which has no host access.
