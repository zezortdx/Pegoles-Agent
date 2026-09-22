/// Guest vsock transport: host-side listener + connection pumps.
///
/// Role discipline: Swift owns ONLY transport — listen, accept, frame,
/// forward, send, close. All protocol decisions (handshake, versions,
/// heartbeat) live in Rust (`pegoles-computer/src/guest.rs`). No agent
/// logic here, ever.
///
/// Framing mirrors `pegoles-guest-proto` (canonical): one UTF-8 line per
/// frame, `guestFrameMax` bytes cap. Oversize/non-UTF-8 frames close the
/// connection with a `guest_disconnected` reason; Rust decides policy.

import Foundation
import Virtualization

/// Canonical port lives in Rust (`pegoles-guest-proto::PEGOLES_VSOCK_PORT`).
/// Duplicated here because Swift cannot import it; keep in sync.
let pegolesVsockPort: UInt32 = 4050

/// Canonical cap lives in Rust (`MAX_FRAME_BYTES`). Keep in sync.
let guestFrameMax = 64 * 1024

/// Close a Vz connection object on the main queue (Vz asserts queue usage
/// even for teardown).
func closeConnectionOnMainQueue(_ conn: VZVirtioSocketConnection) {
    if DispatchQueue.getSpecific(key: mainQueueKey) != nil {
        conn.close()
    } else {
        DispatchQueue.main.sync { conn.close() }
    }
}

/// One guest connection: fd pump (background) + locked writes.
final class GuestLink {
    let computerId: String
    private var connection: VZVirtioSocketConnection?
    private var pumping = false
    private let lock = NSLock()
    private let pumpQueue: DispatchQueue

    init(computerId: String) {
        self.computerId = computerId
        self.pumpQueue = DispatchQueue(label: "pegoles.guest.\(computerId)")
    }

    /// Replace any existing connection (guest restart) and start pumping.
    /// Must be called on the main queue (touches Vz objects).
    func attach(_ conn: VZVirtioSocketConnection) {
        lock.lock()
        if let old = connection {
            lock.unlock()
            old.close()
            lock.lock()
        }
        connection = conn
        let already = pumping
        pumping = true
        lock.unlock()
        emit(HostEvent(event: "guest_connected", computer_id: computerId,
                       state: nil, message: nil, payload: nil, reason: nil))
        if !already {
            pumpQueue.async { [weak self] in self?.pump() }
        }
    }

    func detach(reason: String) {
        lock.lock()
        let conn = connection
        connection = nil
        pumping = false
        lock.unlock()
        if let conn {
            closeConnectionOnMainQueue(conn)
        }
        emit(HostEvent(event: "guest_disconnected", computer_id: computerId,
                       state: nil, message: nil, payload: nil, reason: reason))
    }

    var isConnected: Bool {
        lock.lock(); defer { lock.unlock() }
        return connection != nil
    }

    /// Write one frame (payload must not contain `\n`; enforced by Rust).
    func send(payload: String) -> Bool {
        lock.lock()
        guard let conn = connection else { lock.unlock(); return false }
        let fd = conn.fileDescriptor
        lock.unlock()
        guard var data = payload.data(using: .utf8) else { return false }
        data.append(0x0A)
        var written = 0
        while written < data.count {
            let n = data.withUnsafeBytes { ptr -> Int in
                Foundation.write(fd, ptr.baseAddress!.advanced(by: written), data.count - written)
            }
            if n <= 0 { return false }
            written += n
        }
        return true
    }

    private func pump() {
        while true {
            lock.lock()
            guard let conn = connection else { lock.unlock(); return }
            let fd = conn.fileDescriptor
            lock.unlock()
            // Blocking single-byte reads would be slow; read in chunks and
            // split locally. Idle guest => blocked here, harmless.
            var chunk = [UInt8](repeating: 0, count: 4096)
            let n = chunk.withUnsafeMutableBytes { ptr -> Int in
                Foundation.read(fd, ptr.baseAddress!, ptr.count)
            }
            if n <= 0 {
                detach(reason: "eof")
                return
            }
            if !ingest(bytes: chunk[..<n]) {
                return // ingest detached on violation
            }
        }
    }

    private var pending = Data()

    /// Returns false when the connection was dropped on violation.
    private func ingest(bytes: ArraySlice<UInt8>) -> Bool {
        pending.append(contentsOf: bytes)
        while let nl = pending.firstIndex(of: UInt8(ascii: "\n")) {
            var line = pending[..<nl]
            pending.removeSubrange(...nl)
            if line.last == UInt8(ascii: "\r") { line = line.dropLast() }
            if line.count > guestFrameMax {
                detach(reason: "frame_too_large")
                return false
            }
            guard let text = String(data: Data(line), encoding: .utf8) else {
                detach(reason: "invalid_utf8")
                return false
            }
            emit(HostEvent(event: "guest_frame", computer_id: computerId,
                           state: nil, message: nil, payload: text, reason: nil))
        }
        if pending.count > guestFrameMax {
            detach(reason: "frame_too_large")
            return false
        }
        return true
    }
}

/// Routes listener callbacks to per-computer links.
final class GuestSocketDelegate: NSObject, VZVirtioSocketListenerDelegate {
    var onAccept: ((String, VZVirtioSocketConnection) -> Void)?
    private var devices: [ObjectIdentifier: String] = [:]
    private let lock = NSLock()

    func register(device: VZSocketDevice, computerId: String) {
        lock.lock(); defer { lock.unlock() }
        devices[ObjectIdentifier(device)] = computerId
    }

    func unregister(computerId: String) {
        lock.lock(); defer { lock.unlock() }
        devices = devices.filter { $0.value != computerId }
    }

    /// NOTE: the ObjC selector MUST stay
    /// `listener:shouldAcceptNewConnection:fromSocketDevice:` — that is what
    /// Virtualization.framework dispatches. The `from:` Swift label is only
    /// surface syntax; without the explicit @objc the delegate silently
    /// never fires (no guest connections accepted, no error anywhere).
    @objc(listener:shouldAcceptNewConnection:fromSocketDevice:)
    func listener(
        _ listener: VZVirtioSocketListener,
        shouldAcceptNewConnection connection: VZVirtioSocketConnection,
        from socketDevice: VZVirtioSocketDevice
    ) -> Bool {
        lock.lock()
        let id = devices[ObjectIdentifier(socketDevice)]
        lock.unlock()
        guard let computerId = id else {
            log("vsock connection for unknown device; rejecting")
            return false
        }
        onAccept?(computerId, connection)
        return true
    }
}
