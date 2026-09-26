/// VM lifecycle owner: builds `VZVirtualMachineConfiguration`,
/// owns `VZVirtualMachine` instances, maps Vz states to wire states.
///
/// Devices attached:
/// - EFI boot loader + per-computer EFI variable store
/// - virtio block (instance disk, read-write; optional read-only seed ISO
///   for image builders only)
/// - virtio entropy, traditional memory balloon
/// - virtio console -> bounded logs/serial.log (guest output only)
/// - virtio socket: the authenticated Core <-> guest runtime channel
/// - virtio-gpu, one 1440x900 scanout (guest compositor output only)
/// Deliberately absent: network, shared directories, clipboard, audio,
/// Rosetta, host keyboard/pointing devices.

import Foundation
import Virtualization

let mainQueueKey = DispatchSpecificKey<UInt8>()

/// Guest display scanout (matches the Image v2 compositor mode).
let guestDisplayWidthPx = 1440
let guestDisplayHeightPx = 900

func mapState(_ state: VZVirtualMachine.State) -> String {
    switch state {
    case .stopped: return "stopped"
    case .starting, .pausing, .resuming: return "starting"
    case .running: return "running"
    case .paused: return "paused"
    case .stopping: return "stopping"
    // .saving/.restoring are unreachable via our command set (no
    // save/restore commands); treat as transitional if ever observed.
    case .saving, .restoring: return "starting"
    case .error: return "error"
    @unknown default: return "error"
    }
}

final class VmDelegate: NSObject, VZVirtualMachineDelegate {
    var onGuestStopped: ((String) -> Void)?
    var onFailed: ((String, String) -> Void)?
    /// computerId by VM identity; set at creation. Delegate callbacks may
    /// arrive off the main thread, so access is locked.
    private var ids: [ObjectIdentifier: String] = [:]
    private let lock = NSLock()

    func register(_ vm: VZVirtualMachine, computerId: String) {
        lock.lock(); defer { lock.unlock() }
        ids[ObjectIdentifier(vm)] = computerId
    }

    func unregister(_ vm: VZVirtualMachine) {
        lock.lock(); defer { lock.unlock() }
        ids.removeValue(forKey: ObjectIdentifier(vm))
    }

    private func id(for vm: VZVirtualMachine) -> String? {
        lock.lock(); defer { lock.unlock() }
        return ids[ObjectIdentifier(vm)]
    }

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        guard let id = id(for: virtualMachine) else { return }
        onGuestStopped?(id)
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        guard let id = id(for: virtualMachine) else { return }
        onFailed?(id, error.localizedDescription)
    }
}

enum VmHostError: Error {
    case failure(code: String, message: String)
}

final class VmManager {
    private var machines: [String: VZVirtualMachine] = [:]
    private let delegate = VmDelegate()
    private var guestLinks: [String: GuestLink] = [:]
    /// Guards `guestLinks` (touched from the command thread and from Vz
    /// delegate callbacks on the main queue).
    private let linksLock = NSLock()
    private var serialLogs: [String: SerialLog] = [:]
    private let socketDelegate = GuestSocketDelegate()
    private let socketListener = VZVirtioSocketListener()
    /// Serial capture built by the last `buildConfiguration` call; adopted
    /// by `create`, dropped by `validate`.
    private var pendingSerial: SerialLog?

    init() {
        // The listener holds its delegate weakly: without this line the
        // framework has nobody to call, guest dials go nowhere, and nothing
        // logs an error. (Phase 3 shipped exactly this bug.)
        socketListener.delegate = socketDelegate
        delegate.onGuestStopped = { id in
            emit(HostEvent(event: "vm_state_changed", computer_id: id, state: "stopped",
                           message: nil, payload: nil, reason: nil))
        }
        delegate.onFailed = { id, message in
            emit(HostEvent(event: "vm_failed", computer_id: id, state: nil,
                           message: message, payload: nil, reason: nil))
        }
        socketDelegate.onAccept = { [weak self] computerId, connection in
            if let strong = self {
                strong.guestLink(for: computerId).attach(connection)
            }
        }
    }

    // MARK: - configuration

    private func buildConfiguration(params: CreateParams) throws -> VZVirtualMachineConfiguration {
        guard params.vcpus >= 1, params.memory_mb >= 512 else {
            throw VmHostError.failure(code: "invalid_params",
                                      message: "vcpus and memory_mb out of range")
        }
        let config = VZVirtualMachineConfiguration()
        config.cpuCount = Int(params.vcpus)
        config.memorySize = UInt64(params.memory_mb) * 1024 * 1024

        // Platform + stable machine identity (persisted by Rust layout).
        let platform = VZGenericPlatformConfiguration()
        platform.machineIdentifier = try loadOrCreateMachineIdentifier(path: params.machine_id_path)
        config.platform = platform

        // EFI boot from the instance disk.
        let efi = VZEFIBootLoader()
        efi.variableStore = try loadOrCreateVariableStore(path: params.efi_vars_path)
        config.bootLoader = efi

        // Instance disk (writable copy owned by this computer).
        let diskURL = URL(fileURLWithPath: params.disk_path)
        guard FileManager.default.fileExists(atPath: params.disk_path) else {
            throw VmHostError.failure(code: "invalid_params",
                                      message: "disk image missing at \(params.disk_path)")
        }
        let attachment = try VZDiskImageStorageDeviceAttachment(url: diskURL, readOnly: false)
        var storage: [VZStorageDeviceConfiguration] = [
            VZVirtioBlockDeviceConfiguration(attachment: attachment),
        ]
        // Build-time provisioning only: an optional read-only seed ISO
        // (cloud-init NoCloud). Never used in the normal lifecycle.
        if let seed = params.seed_iso_path, !seed.isEmpty {
            guard FileManager.default.fileExists(atPath: seed) else {
                throw VmHostError.failure(code: "invalid_params",
                                          message: "seed ISO missing at \(seed)")
            }
            let seedAttachment = try VZDiskImageStorageDeviceAttachment(
                url: URL(fileURLWithPath: seed), readOnly: true)
            storage.append(VZVirtioBlockDeviceConfiguration(attachment: seedAttachment))
        }
        config.storageDevices = storage

        // Entropy for the guest.
        config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        // Guest display (Phase 5.1): one virtio-gpu scanout so the guest
        // compositor runs its DRM backend (a libinput seat for the agent's
        // in-guest uinput device) and output capture has a real
        // framebuffer. Output only: no host keyboard/pointing devices,
        // no clipboard. Headless images simply leave it unused.
        let graphics = VZVirtioGraphicsDeviceConfiguration()
        graphics.scanouts = [
            VZVirtioGraphicsScanoutConfiguration(
                widthInPixels: guestDisplayWidthPx, heightInPixels: guestDisplayHeightPx),
        ]
        config.graphicsDevices = [graphics]

        // Memory balloon (Phase 3.6 §42): lets the hypervisor reclaim
        // idle guest pages. There is deliberately NO host-driven target
        // API wired here — Vz exposes none publicly; the device enables
        // hypervisor-managed ballooning and the guest balloon driver.
        // Targets are computed by BalloonPolicy (Rust) for future
        // enforcement points (HCS dynamic memory on Windows).
        config.memoryBalloonDevices = [VZVirtioTraditionalMemoryBalloonDeviceConfiguration()]

        // Serial console -> bounded log file (guest output, diagnostics).
        let serial = try SerialLog(path: params.serial_log_path)
        let console = VZVirtioConsoleDeviceSerialPortConfiguration()
        console.attachment = serial.attachment
        config.serialPorts = [console]
        pendingSerial = serial

        // Virtio socket: the Core <-> guest runtime control channel
        // (authenticated by reserved source port, see GuestSocket.swift).
        config.socketDevices = [VZVirtioSocketDeviceConfiguration()]

        // No network, no shared directories, no clipboard, no host input.
        return config
    }

    private func loadOrCreateMachineIdentifier(path: String) throws -> VZGenericMachineIdentifier {
        let url = URL(fileURLWithPath: path)
        if let data = try? Data(contentsOf: url),
           let id = VZGenericMachineIdentifier(dataRepresentation: data) {
            return id
        }
        let id = VZGenericMachineIdentifier()
        do {
            try id.dataRepresentation.write(to: url, options: .atomic)
        } catch {
            throw VmHostError.failure(code: "internal",
                                      message: "cannot persist machine identifier: \(error.localizedDescription)")
        }
        return id
    }

    private func loadOrCreateVariableStore(path: String) throws -> VZEFIVariableStore {
        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            return VZEFIVariableStore(url: url)
        }
        do {
            return try VZEFIVariableStore(creatingVariableStoreAt: url, options: [])
        } catch {
            throw VmHostError.failure(code: "internal",
                                      message: "cannot create EFI variable store at \(path): \(error.localizedDescription)")
        }
    }

    // MARK: - main-queue routing
    //
    // VZVirtualMachine asserts main-QUEUE usage (dispatch_assert_queue, not
    // just the main thread). A Swift `Task { @MainActor in }` runs on the
    // main thread but outside a main-queue block, which still trips the
    // assert ("BUG IN CLIENT OF LIBDISPATCH"). Therefore every Vz call
    // happens inside a genuine main-queue GCD block. Async Vz APIs are
    // reached through their `__` completion-handler variants (the refined
    // async names cannot run on the queue) bridged with a semaphore; the
    // waiting always happens on our background thread, never on main.

    private func onMainQueueSync<T>(_ body: @escaping () -> T) -> T {
        if DispatchQueue.getSpecific(key: mainQueueKey) != nil { return body() }
        return DispatchQueue.main.sync(execute: body)
    }

    private func vmState(_ vm: VZVirtualMachine) -> VZVirtualMachine.State {
        onMainQueueSync { vm.state }
    }

    private func vmCan(_ vm: VZVirtualMachine, _ key: @escaping (VZVirtualMachine) -> Bool) -> Bool {
        onMainQueueSync { key(vm) }
    }

    /// Call a `__` completion-handler Vz API on the main queue and block
    /// (background thread only) until it completes or times out.
    private func awaitOnMain(timeout: TimeInterval,
                             _ call: @escaping (@escaping (Error?) -> Void) -> Void) -> Error? {
        var result: Error?
        let sema = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            call { err in
                result = err
                sema.signal()
            }
        }
        if sema.wait(timeout: .now() + timeout) == .timedOut {
            return VmHostError.failure(code: "internal", message: "native call timed out")
        }
        return result
    }

    // MARK: - commands (each returns the resulting wire state)

    func validate(params: CreateParams) throws {
        let config = try buildConfiguration(params: params)
        pendingSerial?.close()
        pendingSerial = nil
        try config.validate()
    }

    func create(params: CreateParams) throws -> String {
        if machines[params.computer_id] != nil {
            throw VmHostError.failure(code: "already_exists", message: "computer already registered")
        }
        let config = try buildConfiguration(params: params)
        let serial = pendingSerial
        pendingSerial = nil
        do {
            try config.validate()
        } catch {
            serial?.close()
            throw VmHostError.failure(code: "validation_failed",
                                      message: "VM configuration invalid: \(error.localizedDescription)")
        }
        serialLogs[params.computer_id] = serial
        let vm = onMainQueueSync { VZVirtualMachine(configuration: config) }
        vm.delegate = delegate
        delegate.register(vm, computerId: params.computer_id)
        machines[params.computer_id] = vm
        attachGuestListener(computerId: params.computer_id, vm: vm)
        return mapState(vmState(vm))
    }

    // MARK: - guest vsock transport

    private func guestLink(for computerId: String) -> GuestLink {
        linksLock.lock(); defer { linksLock.unlock() }
        if let link = guestLinks[computerId] { return link }
        let link = GuestLink(computerId: computerId)
        guestLinks[computerId] = link
        return link
    }

    /// Bind the shared listener on the Pegoles port for this VM's socket
    /// device. Called at creation; the guest connects after boot.
    private func attachGuestListener(computerId: String, vm: VZVirtualMachine) {
        onMainQueueSync {
            for device in vm.socketDevices {
                guard let virtio = device as? VZVirtioSocketDevice else { continue }
                virtio.setSocketListener(self.socketListener, forPort: pegolesVsockPort)
                self.socketDelegate.register(device: device, computerId: computerId)
            }
        }
        _ = guestLink(for: computerId)
    }

    /// Queue one frame for the guest. Payload must be a single JSONL line.
    func guestSend(computerId: String, payload: String) throws {
        guard machines[computerId] != nil else {
            throw VmHostError.failure(code: "unknown_computer", message: "no such computer")
        }
        if payload.contains("\n") || payload.utf8.count > guestFrameMax {
            throw VmHostError.failure(code: "invalid_params",
                                      message: "guest payload must be one frame")
        }
        guard guestLink(for: computerId).send(payload: payload) else {
            throw VmHostError.failure(code: "guest_unavailable",
                                      message: "no guest connection")
        }
    }

    func guestStatus(computerId: String) throws -> Bool {
        guard machines[computerId] != nil else {
            throw VmHostError.failure(code: "unknown_computer", message: "no such computer")
        }
        return guestLink(for: computerId).isConnected
    }

    /// Drop the guest connection (protocol violation, incompatibility).
    func guestDisconnect(computerId: String) throws {
        guard machines[computerId] != nil else {
            throw VmHostError.failure(code: "unknown_computer", message: "no such computer")
        }
        guestLink(for: computerId).detach(reason: "kicked")
    }

    private func dropGuestLink(computerId: String) {
        socketDelegate.unregister(computerId: computerId)
        linksLock.lock()
        let link = guestLinks.removeValue(forKey: computerId)
        linksLock.unlock()
        link?.detach(reason: "destroyed")
    }

    private func machine(_ id: String) throws -> VZVirtualMachine {
        guard let vm = machines[id] else {
            throw VmHostError.failure(code: "unknown_computer", message: "no such computer")
        }
        return vm
    }

    /// Poll until the VM reaches `wanted` or the timeout elapses.
    /// Runs on our background thread; state reads hop to the main queue.
    private func waitForState(_ vm: VZVirtualMachine, _ wanted: VZVirtualMachine.State,
                              timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if vmState(vm) == wanted { return }
            Thread.sleep(forTimeInterval: 0.2)
        }
    }

    func start(computerId: String) throws -> String {
        let vm = try machine(computerId)
        guard vmCan(vm, { $0.canStart }) else {
            throw VmHostError.failure(code: "start_failed",
                                      message: "VM cannot start from state \(mapState(vmState(vm)))")
        }
        if let err = awaitOnMain(timeout: 150, { vm.__start(completionHandler: $0) }) {
            throw mapNativeError(err, fallback: "start_failed")
        }
        return mapState(vmState(vm))
    }

    func pause(computerId: String) throws -> String {
        let vm = try machine(computerId)
        guard vmCan(vm, { $0.canPause }) else {
            throw VmHostError.failure(code: "pause_failed",
                                      message: "VM cannot pause from state \(mapState(vmState(vm)))")
        }
        if let err = awaitOnMain(timeout: 60, { vm.__pause(completionHandler: $0) }) {
            throw mapNativeError(err, fallback: "pause_failed")
        }
        return mapState(vmState(vm))
    }

    func resume(computerId: String) throws -> String {
        let vm = try machine(computerId)
        guard vmCan(vm, { $0.canResume }) else {
            throw VmHostError.failure(code: "resume_failed",
                                      message: "VM cannot resume from state \(mapState(vmState(vm)))")
        }
        if let err = awaitOnMain(timeout: 60, { vm.__resume(completionHandler: $0) }) {
            throw VmHostError.failure(code: "resume_failed", message: err.localizedDescription)
        }
        return mapState(vmState(vm))
    }

    func stop(computerId: String) throws -> String {
        let vm = try machine(computerId)
        // Prefer a graceful guest shutdown; fall back to a forced stop.
        if vmCan(vm, { $0.canRequestStop }) {
            onMainQueueSync { () in _ = try? vm.requestStop() }
            waitForState(vm, .stopped, timeout: 15)
        }
        if vmState(vm) != .stopped {
            // NOTE: unlike start/pause/resume, stop has no refined `__`
            // variant; the completion-handler API is directly visible.
            if let err = awaitOnMain(timeout: 60, { vm.stop(completionHandler: $0) }) {
                throw mapNativeError(err, fallback: "stop_failed")
            }
        }
        return mapState(vmState(vm))
    }

    func state(computerId: String) throws -> String {
        return mapState(vmState(try machine(computerId)))
    }

    func destroy(computerId: String) throws {
        let vm = try machine(computerId)
        let s = vmState(vm)
        if s == .running || s == .paused {
            _ = try? stop(computerId: computerId)
        }
        delegate.unregister(vm)
        machines.removeValue(forKey: computerId)
        dropGuestLink(computerId: computerId)
        serialLogs.removeValue(forKey: computerId)?.close()
    }

    /// Parent gone (stdin EOF or parent exit): stop every VM this helper
    /// owns so none outlives the app that could control it. Graceful for
    /// a few seconds, then forced. Runs on the command thread.
    func shutdownAll() {
        for id in Array(machines.keys) {
            guard let vm = machines[id] else { continue }
            let s = vmState(vm)
            if s == .running || s == .paused || s == .starting {
                if vmCan(vm, { $0.canRequestStop }) {
                    onMainQueueSync { () in _ = try? vm.requestStop() }
                    waitForState(vm, .stopped, timeout: 3)
                }
                if vmState(vm) != .stopped {
                    _ = awaitOnMain(timeout: 10, { vm.stop(completionHandler: $0) })
                }
            }
            dropGuestLink(computerId: id)
            serialLogs.removeValue(forKey: id)?.close()
        }
        machines.removeAll()
    }

    private func mapNativeError(_ err: Error, fallback: String) -> VmHostError {
        let desc = err.localizedDescription.lowercased()
        if desc.contains("entitle") {
            return .failure(code: "not_entitled",
                            message: "missing com.apple.security.virtualization entitlement: \(err.localizedDescription)")
        }
        return .failure(code: fallback, message: err.localizedDescription)
    }
}
