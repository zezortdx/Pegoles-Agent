/// Guest vsock transport: host-side listener + connection pumps.
///
/// Role discipline: Swift owns ONLY transport — listen, authenticate the
/// peer port, accept, frame, forward, send, close. All protocol decisions
/// (handshake, versions, heartbeat) live in Rust
/// (`pegoles-computer/src/guest.rs`). No agent logic here, ever.
///
/// Framing mirrors `pegoles-guest-proto` (canonical): one UTF-8 line per
/// frame, `guestFrameMax` bytes cap. Oversize/non-UTF-8 frames close the
/// connection with a `guest_disconnected` reason; Rust decides policy.
///
/// Everything a guest sends is untrusted. The guest can never block the
/// helper's command thread (writes are queued and time out), never grow
/// host memory without bound (per-connection write backlog and line
/// rate caps), and an unprivileged guest process can never impersonate
/// the runtime (reserved source port, see `guestSourcePortMax`).

import Foundation
import Virtualization

/// Canonical port lives in Rust (`pegoles-guest-proto::PEGOLES_VSOCK_PORT`).
/// Duplicated here because Swift cannot import it; keep in sync.
let pegolesVsockPort: UInt32 = 4050

/// Canonical cap lives in Rust (`MAX_FRAME_BYTES`). Keep in sync.
let guestFrameMax = 64 * 1024

/// Peer authentication: Linux only lets a process holding
/// CAP_NET_BIND_SERVICE bind a vsock port <= 1023. The guest runtime
/// binds its source port in that range before dialing (systemd grants it
/// only that capability); any other guest process dials from an
/// ephemeral port > 1023 and is rejected here. Keep in sync with
/// `pegoles-guest-proto::GUEST_SOURCE_PORT_MAX`.
let guestSourcePortMax: UInt32 = 1023

/// Bytes queued toward the guest but not yet written. A guest that
/// stops reading is disconnected instead of growing host memory.
let guestWriteBacklogMax = 4 * 1024 * 1024

/// A single write may block this long before the guest is considered
/// wedged and dropped.
let guestSendTimeoutSeconds = 5

/// Inbound line budget per computer (token bucket). Kept by the link, not
/// the connection: reconnecting never refills it.
let guestLinesPerSecond = 10_000.0
let guestLineBurst = 20_000.0

/// Accepted connections per computer (token bucket). The runtime dials
/// once per boot and redials with backoff; every accept costs a thread,
/// a queue and a `guest_connected` event, so a guest cannot spin it.
let guestAcceptsPerSecond = 0.5
let guestAcceptBurst = 8.0

/// Close a Vz connection object on the main queue (Vz asserts queue usage
/// even for teardown).
func closeConnectionOnMainQueue(_ conn: VZVirtioSocketConnection) {
    if DispatchQueue.getSpecific(key: mainQueueKey) != nil {
        conn.close()
    } else {
        DispatchQueue.main.sync { conn.close() }
    }
}

/// One accepted guest connection. Owned by exactly one pump; detaching a
/// stale connection never touches its successor.
final class GuestConnection {
    let vz: VZVirtioSocketConnection
    let fd: Int32
    let writeQueue: DispatchQueue
    /// Guarded by the owning link's lock.
    var backlog = 0
    var closed = false

    init(_ vz: VZVirtioSocketConnection, label: String) {
        self.vz = vz
        self.fd = vz.fileDescriptor
        self.writeQueue = DispatchQueue(label: label)
        var timeout = timeval(tv_sec: guestSendTimeoutSeconds, tv_usec: 0)
        _ = setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout,
                       socklen_t(MemoryLayout<timeval>.size))
    }
}

/// Refill-on-read token bucket (callers hold the owning link's lock).
/// Monotonic time: a wall-clock step backwards must not drain the bucket.
struct TokenBucket {
    let rate: Double
    let burst: Double
    private var tokens: Double
    private var stamp = DispatchTime.now().uptimeNanoseconds

    init(rate: Double, burst: Double) {
        self.rate = rate
        self.burst = burst
        self.tokens = burst
    }

    mutating func take() -> Bool {
        let now = DispatchTime.now().uptimeNanoseconds
        let elapsed = now >= stamp ? Double(now - stamp) / 1_000_000_000 : 0
        tokens = min(burst, tokens + elapsed * rate)
        stamp = now
        if tokens < 1 { return false }
        tokens -= 1
        return true
    }
}

/// Per-computer guest channel: at most one live connection.
///
/// Event order on stdout matches connection order: `eventLock` is held
/// while a pump checks its connection is live and relays a frame, and
/// while `attach`/`detach` swap connections and emit their events. So no
/// frame of a superseded or detached connection can follow its
/// `guest_disconnected` (or its successor's `guest_connected`). Lock
/// order: `eventLock` before `lock`; never wait on the main queue while
/// holding `eventLock` (`attach` runs there).
final class GuestLink {
    let computerId: String
    private var current: GuestConnection?
    private let lock = NSLock()
    private let eventLock = NSLock()
    private var generation = 0
    /// Guarded by `lock`; shared by every connection of this computer.
    private var lineBudget = TokenBucket(rate: guestLinesPerSecond, burst: guestLineBurst)
    private var acceptBudget = TokenBucket(rate: guestAcceptsPerSecond, burst: guestAcceptBurst)

    init(computerId: String) {
        self.computerId = computerId
    }

    /// Adopt a new authenticated connection (guest runtime restart) and
    /// start its pump. Called on the main queue (Vz delegate). Returns
    /// false (Vz then refuses the connection) past the accept budget.
    func attach(_ vz: VZVirtioSocketConnection) -> Bool {
        eventLock.lock()
        lock.lock()
        guard acceptBudget.take() else {
            lock.unlock()
            eventLock.unlock()
            log("guest \(computerId): reconnecting too fast; refusing connection")
            return false
        }
        generation += 1
        let gen = generation
        let conn = GuestConnection(vz, label: "pegoles.guest.\(computerId).w\(gen)")
        let old = current
        current = conn
        old?.closed = true
        lock.unlock()
        if let old {
            // Same order as detach(): wake the old reader and fail its
            // pending writes before the fd number can be reused, then tell
            // Rust the old connection is gone BEFORE the new one exists.
            _ = Foundation.shutdown(old.fd, Int32(SHUT_RDWR))
            emit(HostEvent(event: "guest_disconnected", computer_id: computerId,
                           state: nil, message: nil, payload: nil, reason: "superseded"))
        }
        emit(HostEvent(event: "guest_connected", computer_id: computerId,
                       state: nil, message: nil, payload: nil, reason: nil))
        eventLock.unlock()
        if let old {
            closeConnectionOnMainQueue(old.vz)
        }
        let thread = Thread { [weak self] in self?.pump(conn) }
        thread.name = "pegoles.guest.\(computerId).r\(gen)"
        thread.start()
        return true
    }

    /// Drop `conn` (or whatever is current when nil). Emits exactly one
    /// `guest_disconnected` for the connection Rust knows about; a stale
    /// connection is closed silently.
    func detach(_ conn: GuestConnection? = nil, reason: String) {
        eventLock.lock()
        lock.lock()
        let target = conn ?? current
        guard let target, !target.closed else {
            lock.unlock()
            eventLock.unlock()
            return
        }
        target.closed = true
        let wasCurrent = target === current
        if wasCurrent { current = nil }
        lock.unlock()
        // Wake the reader and fail pending writes BEFORE the fd number is
        // released: a closed fd can be reused (reconnect, log rotation)
        // while the old pump or a queued write still holds the number.
        _ = Foundation.shutdown(target.fd, Int32(SHUT_RDWR))
        if wasCurrent {
            emit(HostEvent(event: "guest_disconnected", computer_id: computerId,
                           state: nil, message: nil, payload: nil, reason: reason))
        }
        eventLock.unlock()
        closeConnectionOnMainQueue(target.vz)
    }

    var isConnected: Bool {
        lock.lock(); defer { lock.unlock() }
        return current != nil
    }

    /// Queue one frame (payload must not contain `\n`; enforced by the
    /// caller). Never blocks the command thread.
    func send(payload: String) -> Bool {
        guard var data = payload.data(using: .utf8) else { return false }
        data.append(0x0A)
        lock.lock()
        guard let conn = current, !conn.closed else { lock.unlock(); return false }
        if conn.backlog + data.count > guestWriteBacklogMax {
            lock.unlock()
            detach(conn, reason: "send_backlog")
            return false
        }
        conn.backlog += data.count
        lock.unlock()
        conn.writeQueue.async { [weak self] in
            guard let self else { return }
            let ok = self.writeAll(conn, data: data)
            self.lock.lock()
            conn.backlog -= data.count
            self.lock.unlock()
            if !ok { self.detach(conn, reason: "send_timeout") }
        }
        return true
    }

    /// Write every byte unless the connection is closed meanwhile (then
    /// its fd number may already belong to something else).
    private func writeAll(_ conn: GuestConnection, data: Data) -> Bool {
        var written = 0
        while written < data.count {
            lock.lock()
            let closed = conn.closed
            lock.unlock()
            if closed { return false }
            let n = data.withUnsafeBytes { ptr -> Int in
                Foundation.write(conn.fd, ptr.baseAddress!.advanced(by: written),
                                 data.count - written)
            }
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { return false } // EAGAIN after SO_SNDTIMEO, EPIPE, …
            written += n
        }
        return true
    }

    private func pump(_ conn: GuestConnection) {
        var pending = Data()
        var chunk = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
            lock.lock()
            let gone = conn.closed
            lock.unlock()
            if gone { return }
            let n = chunk.withUnsafeMutableBytes { ptr -> Int in
                Foundation.read(conn.fd, ptr.baseAddress!, ptr.count)
            }
            if n < 0 && errno == EINTR { continue }
            if n <= 0 {
                detach(conn, reason: "eof")
                return
            }
            pending.append(contentsOf: chunk[..<n])
            // Split at a moving index and drop the consumed prefix once
            // per read: removing each line from the front was quadratic
            // under a flood of tiny lines.
            var consumed = pending.startIndex
            while let nl = pending[consumed...].firstIndex(of: UInt8(ascii: "\n")) {
                var line = pending[consumed..<nl]
                consumed = nl + 1
                if line.last == UInt8(ascii: "\r") { line = line.dropLast() }
                let text = String(data: Data(line), encoding: .utf8)
                let tooLarge = line.count > guestFrameMax
                // Valid JSON Lines never carry raw control bytes (JSON
                // escapes them); refusing them also bounds how much the
                // relayed JSON can grow when re-escaped for the host.
                let hasControl = line.contains { $0 < 0x20 && $0 != 0x09 }
                if tooLarge {
                    detach(conn, reason: "frame_too_large")
                    return
                }
                if hasControl {
                    detach(conn, reason: "invalid_frame")
                    return
                }
                guard let text else {
                    detach(conn, reason: "invalid_utf8")
                    return
                }
                lock.lock()
                let withinBudget = lineBudget.take()
                lock.unlock()
                if !withinBudget {
                    detach(conn, reason: "flood")
                    return
                }
                eventLock.lock()
                lock.lock()
                let live = !conn.closed
                lock.unlock()
                if live {
                    emit(HostEvent(event: "guest_frame", computer_id: computerId,
                                   state: nil, message: nil, payload: text, reason: nil))
                }
                eventLock.unlock()
                if !live { return }
            }
            pending.removeSubrange(pending.startIndex..<consumed)
            if pending.count > guestFrameMax {
                detach(conn, reason: "frame_too_large")
                return
            }
        }
    }
}

/// Routes listener callbacks to per-computer links.
final class GuestSocketDelegate: NSObject, VZVirtioSocketListenerDelegate {
    /// Adopts the connection; false refuses it (accept budget spent).
    var onAccept: ((String, VZVirtioSocketConnection) -> Bool)?
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
        guard connection.sourcePort <= guestSourcePortMax else {
            log("vsock connection from unprivileged guest port \(connection.sourcePort); rejecting")
            return false
        }
        return onAccept?(computerId, connection) ?? false
    }
}
