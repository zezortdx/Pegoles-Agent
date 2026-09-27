# Windows architecture (decision record, 2026-09-27)

Status: **decided, implemented up to the guest image, not verified on
consumer hardware.** Supersedes the Phase 3.6 assumptions in `WINDOWS_BACKEND.md`
(Windows Pro only, the Hyper-V feature, per-user Hyper-V Administrators
membership, an HKLM socket-service registration). Research sources are
listed at the end; claims marked *(verify)* must be proven on hardware.

## The principle does not change on Windows

The model is an untrusted planner. It proposes typed actions; Pegoles
Policy decides; Core's executor applies them to **Pegoles' own
computer**, a Linux VM with no network, no shared folders, no clipboard.
Nothing on Windows gives the model, the guest or the worker a path to the
host's files, processes, shell, registry or network.

## Decision

| Concern | Choice | Why |
|---|---|---|
| Hypervisor API | **Host Compute System (HCS)**, `computecore.dll` | The API WSL2, Windows Sandbox and Multipass use; full VM lifecycle; HvSocket built in |
| Windows feature | **Virtual Machine Platform** (VMP) only | HCS is present with VMP on every edition, Home included (Multipass 1.17 "works on all editions… including Windows Home"; WSL2 needs only VMP). The full Hyper-V role is not needed |
| Privilege | **A small broker service** (`PegolesVmBroker`, LocalSystem, demand-start) | HCS only accepts Administrators or Hyper-V Administrators (`HCS_E_ACCESS_DENIED`). WSL (`wslservice`), Multipass (`multipassd`) and Claude Cowork use the same pattern. The app itself never runs elevated |
| Guest channel | **AF_HYPERV from the unprivileged helper** straight to the guest | Guest bytes never reach the SYSTEM service. No registry key: the HCS document's `HvSocketConfig` grants bind/connect to SYSTEM and the signed-in user's SID only |
| Guest authentication | **The guest runtime listens** on privileged vsock port 850; the host connects | Linux lets only `CAP_NET_BIND_SERVICE` bind ports ≤ 1023, so only the runtime can own that listener. (macOS authenticates the runtime's privileged *source* port instead; on Hyper-V the peer's source port is not reliably visible to the host.) |
| Boot | **UEFI from the computer's VHDX**, boot loader on the removable-media path (`\EFI\BOOT\BOOTX64.EFI`), no saved NVRAM (`BootThis` the SCSI disk) | One sealed artifact (the disk), exactly as on macOS: no separate kernel/initrd files to pin and keep in step. LinuxKernelDirect was the first choice; it is what hcsshim uses for container utility VMs, while UEFI + SCSI VHDX is Microsoft's documented HCS path for full VMs. Secure Boot stays off (no template); the image is verified by digest before it boots |
| Disk | VHDX, per-computer copy under the user's `%LOCALAPPDATA%\Pegoles\computers` | Same layout as macOS; reset = fresh copy of the sealed image |
| Display / input | **Inside the guest** (Weston capture, uinput), over the control channel | Identical to macOS. The VM gets Hyper-V's synthetic video only because Weston needs a DRM device for its input seat (Debian's kernel has no virtual KMS driver); Pegoles never shows it on the host, and no keyboard or mouse device is attached |
| Network | **None** (no `NetworkAdapters`) | The guest stays offline, as on macOS |
| Serial log | COM1 → a named pipe the helper creates → bounded `logs/serial.log` | Boot diagnostics without shell access |
| Local AI | **llama.cpp** (libllama + libmtmd) in a Rust worker (`pegoles-llm-worker`), **Vulkan or CPU** | Only mature runtime for a Qwen3-VL-class model on Windows across NVIDIA/AMD/Intel; MLX is Apple-only. CUDA is not shipped (its runtime adds gigabytes; Vulkan covers NVIDIA) |
| Worker isolation | **AppContainer** (no capabilities ⇒ no network) + **Job Object** (kill-on-close, one process, memory cap) | Equivalent of the macOS `sandbox-exec` profile |
| Installer | NSIS, **per-machine** (Program Files) | A SYSTEM service's binary must not live in a user-writable folder (that would be a user → SYSTEM escalation). One UAC prompt at install, expected by everyone |
| Enabling VMP | From onboarding, after explaining why, via an elevated `pegoles-broker.exe enable-virtualization` (fixed verb, DISM) | Explain before UAC; never silently; restart only when the person presses "Restart now" |

### Rejected

- **WSL2 as the backend.** Microsoft: WSL "is not a security sandbox";
  distros share one utility VM; WSL serves the host file system into the
  guest over Plan 9 regardless of `wsl.conf`; interop can launch Windows
  executables; NAT networking is on by default.
- **QEMU + Windows Hypervisor Platform.** Works on Home, but a large
  device model runs in a user process, WHPX is still maturing (2026 bug
  series), host↔guest would be virtio-serial over a named pipe, and
  shipping QEMU brings GPL-2 source obligations. Kept only as a fallback
  idea.
- **Windows Sandbox.** Pro/Enterprise/Education only, Windows guests,
  networking on by default.
- **Requiring the Hyper-V role / Pro.** Unnecessary with VMP; would
  exclude Home.
- **Putting the user in Hyper-V Administrators.** Near-admin rights for
  every process the user runs; the group's existence on Home is unproven.

## Process topology

```text
Pegoles Agent.exe            (user; Tauri, Core, policy, planners)
 ├─ stdio JSONL ─► pegoles-vm-host.exe      (user; same command set as the
 │                  │                        macOS helper; guest socket,
 │                  │                        serial pipe; no HCS rights)
 │                  ├─ AF_HYPERV ─────────► guest runtime (listens :850)
 │                  └─ \\.\pipe\pegoles-vm-broker (tiny typed protocol)
 │                         ▼
 │                  PegolesVmBroker service (LocalSystem; HCS only)
 │                         ▼
 │                  HCS ─► Hyper-V partition: Debian 13 amd64, no network
 └─ stdio JSONL ─► pegoles-llm-worker.exe   (AppContainer, Job Object;
                                             llama.cpp; text out only)
```

## The broker's protocol (the only privileged surface)

One JSON object per line, ≤ 16 KiB, over a local named pipe:

- `hello {version}` → broker version (protocol mismatch fails).
- `create {computer_id, disk, vcpus, memory_mb}` → `{runtime_id}`. The
  broker builds the HCS document itself from validated values
  (`pegoles-broker-proto::hcs_document`); the client never sends JSON for
  HCS. The serial pipe name is derived from the id, not sent.
- `start`, `pause`, `resume`, `shutdown`, `terminate`, `state` by
  `computer_id`.

Checks, all fail-closed:

1. **Client.** The pipe rejects remote clients; its DACL allows SYSTEM and
   interactive users. The broker reads the client's process image path
   (`GetNamedPipeClientProcessId` → `QueryFullProcessImageNameW`) and
   requires `pegoles-vm-host.exe` in the broker's own (admin-only)
   install folder. The client's user SID comes from impersonating the
   pipe client.
2. **Paths.** `disk` must be `…\Pegoles\computers\<uuid>\disk.vhdx`
   inside **that user's** `%LOCALAPPDATA%`; no component may be a
   reparse point (junctions and symlinks are refused); files are opened
   while impersonating the user, so the broker never grants the VM access
   to anything the user could not already read and write.
3. **Values.** vCPUs 1–8, memory 1024–8192 MB, `computer_id` a UUID,
   the serial pipe name `\\.\pipe\pegoles-serial-<uuid>`.
4. **Lease.** VMs belong to the pipe connection that created them. When it
   closes (app quit, crash), the broker terminates them. The HCS document
   also sets `ShouldTerminateOnLastHandleClosed`, so a crashed broker
   takes its VMs down with it.
5. **Quotas.** At most two compute systems per user.

The broker has no verb that runs anything in the guest, touches other
files, opens the network or changes Windows settings. Enabling VMP is a
separate, elevated, one-shot command (`enable-virtualization`) that only
runs `%SystemRoot%\System32\dism.exe` with fixed arguments.

## Edition and hardware matrix

What the architecture targets. **Nothing here is verified on consumer
hardware yet** (`docs/PLATFORM_MATRIX.md` holds the verified state).

| | x64 | ARM64 |
|---|---|---|
| Windows 11 Home | Target (HCS + VMP) *(verify)* | Later (HCS UEFI boot only; needs an arm64 Windows image) |
| Windows 11 Pro / Enterprise / Education | Target | Later |
| Windows 10 | Not targeted (HCS exists, untested) | — |

Requirements on the PC: hardware virtualization enabled in firmware
(Intel VT-x / AMD-V; onboarding explains how), VMP turned on (onboarding
does it with one UAC prompt and one restart), 8 GB RAM minimum (16 GB
recommended), ~6 GB free for the model and the computer.

## Local AI on Windows

- Model: **MAI-UI-2B** (the macOS default), as GGUF: community conversion
  `mradermacher/MAI-UI-2B-GGUF` (Q8_0 + mmproj-f16, pinned by SHA-256),
  until Pegoles publishes its own conversion from the pinned Tongyi-MAI
  revision.
- Runtime: llama.cpp via the `llama-cpp-2` crate (0.1.157, `mtmd`),
  built from source by `scripts/package-windows.sh` and shipped with the
  app (never the upstream zips, which also carry an HTTP server). Backend
  modules (CPU variants, Vulkan) are loaded only from the install folder;
  the Visual C++ runtime they need is deployed app-locally; the Rust
  executables link the C runtime statically.
- Backend choice is automatic: onboarding predicts it from DXGI (the
  largest non-software adapter) and the presence of the Vulkan loader;
  the worker reports what llama.cpp actually initialized (`hello`), and
  runs on the CPU otherwise. `PEGOLES_LLM_CPU=1` forces the CPU.
- Same protocol (v1), bounds and supervision as the MLX worker; the
  prompt is the model's own chat template written out with special-token
  text neutralized; screenshots are resized to multiples of 32; at least
  1024 image tokens per screenshot.
- The same worker runs on macOS (Metal) for parity measurements
  (`local_bench` picks it for GGUF models); results are in
  `benchmarks/local-models/README.md`.

## What CI can and cannot prove

GitHub's `windows-2025` runners are Azure VMs with nested virtualization.
The `windows-installer` job builds the installer, installs it, starts the
broker service, boots a real HCS VM from an empty VHDX through the
unprivileged helper and the broker (firmware only: no guest image exists
for x64 yet), starts the real llama.cpp worker inside its AppContainer and
job object, and uninstalls. When the runner lacks the Virtual Machine
Platform the HCS step reports a warning instead. That is real Windows
virtualization, but **Windows Server on Azure, not a consumer PC**, and it
runs as an administrator. Consumer-hardware E2E (Home, a standard user,
DPI scaling, real GPUs, a restart during setup, a full task in a booted
guest) stays a separate gate (`docs/RELEASE_GATES.md`).

## Implementation status (2026-09-27, CI run 36346739833)

| Part | State |
|---|---|
| Broker service + protocol (`native/windows/pegoles-broker*`) | Written; linted and unit-tested on Windows in CI; hardened after an independent review (path race, admin-only folder) |
| VM helper (`native/windows/pegoles-vm-host`) | Written; linted and unit-tested on Windows in CI |
| Guest runtime listening on vsock 850 | Written (`--listen` / kernel command line); not yet in a sealed image |
| x64 guest image (Debian 13 amd64, UEFI, Hyper-V drivers) | Built and provisioned in CI (`guest-image-x64`); boots under QEMU/UEFI with its services up; **not published** (the blocker for a real Windows task) |
| llama.cpp worker + AppContainer/job sandbox | Written; worker runs on macOS against the real VM; Windows sandbox compiles; CI start test pending its first run |
| Desktop shell, onboarding, WebView2 containment | Linted and tested on Windows in CI; egress probe on Windows: 0 TCP / 0 UDP contained |
| Installer (NSIS, per machine), silent install/uninstall | Scripted and wired into CI (pending its first run); unsigned |
| Consumer hardware | **Never run** (no Windows PC available to the project yet) |

## Sources

- HCS on VMP / Home: Multipass 1.17 release notes and
  `src/platform/backends/hyperv_api` (canonical/multipass); WSL source
  `src/windows/service/exe/WslCoreVm.cpp`, `src/windows/common/hvsocket.cpp`
  (microsoft/WSL); Lima `vmType: hcs`.
- HCS privileges: `HCS_E_ACCESS_DENIED` in the Windows SDK `winerror.h`;
  `HcsCreateComputeSystem` docs (security descriptor reserved).
- HvSocket: HCS schema reference (`HvSocketConfig`,
  `DefaultBindSecurityDescriptor`); Linux
  `net/vmw_vsock/hyperv_transport.c`, `af_vsock.c` (privileged ports).
- Boot: Microsoft HCS tutorial and schema reference (UEFI `BootThis`,
  SCSI VHDX); hcsshim `internal/uvm/create_lcow.go` (LinuxKernelDirect,
  considered).
- WSL security: microsoft/WSL `doc/docs/technical-documentation/security.md`.
- QEMU WHPX: qemu.org WHPX docs; `qemu-options.hx` (`-chardev pipe`).
- llama.cpp: PR #16780 (Qwen3-VL), PR #25781 (grounding fix, b10085),
  `tools/mtmd/clip.cpp`; `llama-cpp-2` 0.1.157.
- Packaging: tauri-bundler `installer.nsi` (v2.10.0); Microsoft Learn on
  Run/RunOnce, SmartScreen, Smart App Control, Artifact Signing;
  microsoft/winget-pkgs policies.
