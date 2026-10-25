/// Egress transport: one raw byte stream between the guest's egress
/// listener (vsock port 4051) and a local Unix socket the app created.
///
/// Role discipline (same as GuestSocket.swift): transport only. No
/// parsing, no decisions, no logging of payload. The app (Rust,
/// `pegoles-egress`) speaks the mux protocol and owns every policy; this
/// file only pumps bytes and closes both sides when either closes.
/// The VM still has NO network device: this stream is the only way out of
/// the guest, and it exists only while the app asked for it
/// (`egress_open` .. `egress_close`, VM stop/destroy, helper exit).
///
/// Everything from the guest is untrusted. Memory is bounded: each
/// direction owns exactly one `egressBufferBytes` buffer and never reads
/// more until the previous chunk has been fully written to the other side.
///
/// Full contract: `docs/EGRESS.md` and `docs/VM_HOST_PROTOCOL.md`.

import Foundation
import Virtualization

/// Canonical port lives in Rust (`pegoles-egress-proto::EGRESS_VSOCK_PORT`,
/// mirrored by `pegoles-computer::egress::EGRESS_VSOCK_PORT`). Duplicated
/// here because Swift cannot import it; keep in sync.
let egressVsockPort: UInt32 = 4051

/// One pump buffer per direction.
let egressBufferBytes = 64 * 1024

/// How long the helper waits for the guest to accept the vsock stream.
let egressConnectTimeoutSeconds: TimeInterval = 10

/// `sockaddr_un.sun_path` capacity on macOS is 104 bytes including NUL.
let egressSocketPathMax = 103

/// Canonical rule lives in Rust (`pegoles-computer::egress`): the endpoint
/// is `<data root>/egress/<1..40 chars of [0-9a-f-]>/s`. Keep in sync.
let egressDirName = "egress"
let egressSocketName = "s"

/// Validate the endpoint the app asked for: an absolute path to a Unix
/// socket the app (same user) just created in a 0700 directory directly
/// under `<Pegoles data dir>/egress/`. The data dir is derived from the
/// registered computer's own folder (`<root>/computers/<id>`), never from
/// the request. Anything else (other directories, `..`, symlinks, sockets
/// owned by someone else, non-sockets) is rejected before any connect.
func validatedEgressEndpoint(_ endpoint: String, computerDir: String) throws -> String {
    func reject(_ why: String) -> VmHostError {
        VmHostError.failure(code: "invalid_params", message: "egress endpoint rejected: \(why)")
    }
    guard endpoint.hasPrefix("/"), !endpoint.unicodeScalars.contains("\0"),
          endpoint.utf8.count <= egressSocketPathMax else {
        throw reject("not an absolute path of bounded length")
    }
    let root = URL(fileURLWithPath: computerDir)
        .deletingLastPathComponent().deletingLastPathComponent().path
    let base = root + "/" + egressDirName
    guard endpoint.hasPrefix(base + "/") else { throw reject("outside the Pegoles data folder") }
    let rest = endpoint.dropFirst(base.count + 1).split(separator: "/", omittingEmptySubsequences: false)
    guard rest.count == 2, rest[1] == egressSocketName else { throw reject("unexpected layout") }
    let dirName = rest[0]
    let hex = Set("0123456789abcdef-".unicodeScalars)
    guard (1...40).contains(dirName.count), dirName.unicodeScalars.allSatisfy({ hex.contains($0) }) else {
        throw reject("unexpected directory name")
    }
    let me = getuid()
    func owned(_ path: String, _ kind: mode_t, exactPrivate: Bool) -> Bool {
        var st = stat()
        guard lstat(path, &st) == 0, (st.st_mode & S_IFMT) == kind, st.st_uid == me else { return false }
        return !exactPrivate || (st.st_mode & 0o077) == 0
    }
    let dir = base + "/" + dirName
    guard owned(base, S_IFDIR, exactPrivate: true), owned(dir, S_IFDIR, exactPrivate: true),
          owned(endpoint, S_IFSOCK, exactPrivate: false) else {
        throw reject("not a private socket owned by this user")
    }
    return endpoint
}

private func setNoSigpipe(_ fd: Int32) {
    var on: Int32 = 1
    _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
}

/// Connect to a Unix stream socket (already validated). Blocking; the app
/// listens before it sends `egress_open`, so this never waits on it.
func connectUnixSocket(path: String) throws -> Int32 {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else {
        throw VmHostError.failure(code: "egress_failed", message: "cannot create socket")
    }
    setNoSigpipe(fd)
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
        close(fd)
        throw VmHostError.failure(code: "invalid_params", message: "egress endpoint too long")
    }
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        raw.copyBytes(from: bytes)
        raw[bytes.count] = 0
    }
    addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
    let rc = withUnsafePointer(to: &addr) { ptr in
        ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Foundation.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
    }
    guard rc == 0 else {
        let e = errno
        close(fd)
        throw VmHostError.failure(code: "egress_failed",
                                  message: "cannot connect to the app's endpoint (errno \(e))")
    }
    return fd
}

/// One live egress stream. Owns both descriptors until both pump threads
/// are done (closing a descriptor another thread is reading would risk
/// fd reuse), then closes everything exactly once.
final class EgressBridge {
    let computerId: String
    private let vz: VZVirtioSocketConnection
    private let vzFd: Int32
    private let unixFd: Int32
    private let lock = NSLock()
    private var stopping = false
    private var requestedByApp = false
    private var reason = "closed"
    private var pumpsRunning = 2
    private let finished = DispatchSemaphore(value: 0)
    /// Called once, on a pump thread, when the stream ended by itself.
    var onEndedByItself: ((EgressBridge, String) -> Void)?

    init(computerId: String, vz: VZVirtioSocketConnection, unixFd: Int32) {
        self.computerId = computerId
        self.vz = vz
        self.vzFd = vz.fileDescriptor
        self.unixFd = unixFd
        setNoSigpipe(vzFd)
    }

    func start() {
        spawn(label: "egress-guest-to-app", from: vzFd, to: unixFd, closedBy: "guest_closed")
        spawn(label: "egress-app-to-guest", from: unixFd, to: vzFd, closedBy: "app_closed")
    }

    /// App asked to close (or the VM is going away): tear both sides down
    /// and wait (bounded) until the descriptors are released. No event.
    func close() {
        begin(reason: "closed", byApp: true)
        _ = finished.wait(timeout: .now() + 3)
    }

    private func begin(reason why: String, byApp: Bool) {
        lock.lock()
        if stopping { lock.unlock(); return }
        stopping = true
        requestedByApp = byApp
        reason = why
        lock.unlock()
        // Wake both blocked pumps; descriptors stay open until they exit.
        _ = shutdown(unixFd, SHUT_RDWR)
        _ = shutdown(vzFd, SHUT_RDWR)
    }

    private func spawn(label: String, from src: Int32, to dst: Int32, closedBy: String) {
        let thread = Thread { [self] in
            let why = pumpBytes(from: src, to: dst, eofReason: closedBy)
            begin(reason: why, byApp: false)
            pumpExited()
        }
        thread.name = "pegoles-\(label)"
        thread.stackSize = 256 * 1024
        thread.start()
    }

    /// Copy until EOF or error; one fixed buffer, no queue.
    private func pumpBytes(from src: Int32, to dst: Int32, eofReason: String) -> String {
        var buffer = [UInt8](repeating: 0, count: egressBufferBytes)
        while true {
            let n = buffer.withUnsafeMutableBytes { Foundation.read(src, $0.baseAddress!, $0.count) }
            if n < 0 && errno == EINTR { continue }
            if n == 0 { return eofReason }
            if n < 0 { return "read_error" }
            var written = 0
            while written < n {
                let w = buffer.withUnsafeBytes {
                    Foundation.write(dst, $0.baseAddress!.advanced(by: written), n - written)
                }
                if w < 0 && errno == EINTR { continue }
                if w <= 0 { return "write_error" }
                written += w
            }
        }
    }

    private func pumpExited() {
        lock.lock()
        pumpsRunning -= 1
        let last = pumpsRunning == 0
        let byApp = requestedByApp
        let why = reason
        lock.unlock()
        guard last else { return }
        Foundation.close(unixFd)
        closeConnectionOnMainQueue(vz)
        finished.signal()
        if !byApp { onEndedByItself?(self, why) }
    }
}
