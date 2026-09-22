# native/macos — Apple Virtualization boundary

Swift lives **only** here. Rust stays the core. The two sides talk over a
narrow, auditable JSON Lines IPC (stdin/stdout of a child process), never a
wide FFI surface. Contract: `docs/VM_HOST_PROTOCOL.md`.

## Layout

```text
native/macos/
  README.md                # this file
  pegoles-vm-host/         # SwiftPM executable (the only Swift in the repo)
    Package.swift          # swift-tools 5.9, macOS 13+, Swift 5 language mode
    Sources/
      Host.swift           # @main JSONL command loop
      Protocol.swift       # Codable mirror of vmhost_proto.rs
      VmManager.swift      # VZVirtualMachine lifecycle + state mapping
      GuestSocket.swift    # vsock listener, per-guest pumps (transport only)
```

## Build

```bash
cd native/macos/pegoles-vm-host && swift build -c release
# .build/release/pegoles-vm-host
```

Rust locates the helper via `PEGOLES_VM_HOST`, else the dev build tree,
else the Tauri bundle `Resources/` dir. The helper process itself needs
`com.apple.security.virtualization` in its signature — see
`scripts/codesign-dev.sh` and `apps/desktop/src-tauri/entitlements/macos.plist`.

## Threading (hard-won, verified with crash logs)

`VZVirtualMachine` asserts **main-queue** usage (`dispatch_assert_queue`,
not just the main thread). Three findings from real debugging on macOS 26:

1. A Swift `Task { @MainActor in }` runs on the main *thread* but outside
   a main-queue block — the assert still fires (`BUG IN CLIENT OF
   LIBDISPATCH`). `Thread.isMainThread == true` is NOT sufficient.
2. `dispatchMain()` drains main-queue blocks *off* the main thread on
   macOS 26 (verified in C); `CFRunLoopRun()`/`RunLoop.main.run()` keeps
   them on it. The helper's main thread therefore runs the runloop.
3. Consequently: stdin loop on a background thread, every Vz call inside
   a genuine `DispatchQueue.main.{sync,async}` block. Async Vz APIs go
   through their `__` completion-handler variants (the refined async names
   can't run on the queue) bridged with a semaphore; waiting happens on
   the background thread, never on main. (`stop` has no `__` variant; its
   completion API is directly visible.)

## Rules

1. Lifecycle + vsock transport only (`version/validate/create/start/pause/
   resume/stop/state/destroy` + `guest_send/guest_status/guest_disconnect`).
2. No host shell, no host files, no network proxy, no arbitrary native calls.
3. Devices: EFI boot + per-computer EFI vars, virtio block (instance disk),
   entropy, serial console to file, virtio socket with host listener on the
   Pegoles port. No network, no shared directories, no clipboard, no graphics.
4. All validation failures propagate as typed errors; nothing is swallowed.
5. `VZVirtioSocketListener.delegate` MUST be assigned (weak ref): without it
   the framework has nobody to call, guest dials go nowhere, and nothing
   logs an error. Verified the hard way in Phase 3.5 (guest_connected
   absent for weeks of symptom-free silence until this one line landed).
