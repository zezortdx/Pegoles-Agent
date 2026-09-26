# Security

Pegoles runs a model-driven agent against a computer. The design goal is
that a fully prompt-injected model, a hostile web page or file inside
the guest, or a compromised guest OS cannot reach the user's Mac. This
document states what is enforced today, where, and what is not.

## Trust boundaries

```text
 user ─▶ webview UI ─(Tauri commands)─▶ Pegoles Core (Rust, host)
                                           │  typed ComputerAction only
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
| Model → host | untrusted | model output is parsed into typed actions only (`pegoles-agent/src/anthropic.rs::translate`); unknown tools, out-of-screen coordinates and unsupported members become error results, never actions |
| Action → execution | untrusted | `pegoles-policy::evaluate` (exhaustive, deterministic) on every action, then Core's executor (rate limit, control arbitration) |
| Guest → host | hostile | vsock frames ≤ 64 KiB, UTF-8, typed parse, bounded fields; frame reassembly caps; per-connection line-rate limit; bounded queues everywhere between guest and Core |
| Guest process → host channel | hostile | the helper accepts a vsock peer only from a reserved source port (≤ 1023), which only the runtime (CAP_NET_BIND_SERVICE via its unit) can bind |
| Webview → Core | semi-trusted UI | fixed command set, no path/URL/process arguments; strict CSP in release; Design Lab commands compiled only in debug builds |
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
10. **Secrets.** The Anthropic key is stored in the macOS Keychain
   (service `dev.pegoles.agent`) or read from `ANTHROPIC_API_KEY`; it is
   never returned to the webview, never logged (`Debug` redacts it),
   never placed in model context, and never sent to the guest. No
   credentials are baked into the guest image (root and `pegoles`
   accounts are locked; sshd and its vsock/unix socket activation are
   masked).
11. **Child processes** get a scrubbed environment (`env_clear`) and fixed
   arguments: the VM helper and `/usr/bin/tar` (image builder path).

## Prompt injection

Screen content is data. The system prompt says so, and Anthropic's
server-side classifiers flag injections in screenshots, but neither is
relied on: the policy and the missing network bound what a fully
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
- Windows (HCS) code has not been compiled or run in this environment;
  its security properties are unverified.

## Tests that pin these properties

`pegoles-policy` (vocabulary sweep, caps, control chars, tripwire),
`pegoles-protocol` (forbidden verbs never deserialize),
`pegoles-computer` (hostile frames, bounded helper lines, https-only,
symlinked base, fail-closed create, reset restores disk),
`pegoles-core` (policy short-circuit, stuck-input release, agent session
vs display), `pegoles-agent` (translation errors, batch halt, budgets,
cancellation, append-only history, real executor + policy with Mock),
and the hardware E2E `crates/pegoles-agent/examples/agent_e2e.rs`.
