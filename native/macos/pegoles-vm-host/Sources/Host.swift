/// Entry point: JSON Lines command loop on stdin/stdout.
/// One JSON object per line in, one JSON object per line out.
/// Anything that is not valid JSON gets an error response, never a crash.
///
/// Threading: Virtualization.framework asserts main-queue usage, so the
/// main thread pumps the main runloop and never blocks. Blocking stdin
/// reads happen on a background thread, which hops each command onto the
/// main thread and waits for the response.
///
/// Lifetime: the helper never outlives its parent. Stdin EOF or parent
/// exit stops every VM it owns and exits the process.
///
/// NOTE (macOS 26): `dispatchMain()` drains main-queue blocks off the main
/// thread; only `RunLoop.main.run()` keeps them on it. VZ requires the
/// latter, so we use the runloop here.

import Foundation

/// Longest command line accepted from the parent (a `guest_send` frame
/// is <= 64 KiB of payload plus JSON escaping).
let commandLineMax = 1024 * 1024

@main
struct PegolesVmHost {
    /// Buffered line reader over stdin. `nil` = EOF (parent went away);
    /// `.some(nil)` = a line that was oversized or not UTF-8 (skipped).
    final class LineReader {
        private var buffer = Data()
        private var chunk = [UInt8](repeating: 0, count: 64 * 1024)

        func next() -> String?? {
            while true {
                if let nl = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                    let line = buffer[buffer.startIndex..<nl]
                    buffer.removeSubrange(buffer.startIndex...nl)
                    return .some(String(data: Data(line), encoding: .utf8))
                }
                if buffer.count > commandLineMax {
                    // Oversized: drop through the next newline.
                    buffer.removeAll()
                    discardingOversized = true
                }
                let n = chunk.withUnsafeMutableBytes { ptr -> Int in
                    Foundation.read(0, ptr.baseAddress!, ptr.count)
                }
                if n < 0 && errno == EINTR { continue }
                if n <= 0 { return nil }
                if discardingOversized {
                    if let nl = chunk[..<n].firstIndex(of: UInt8(ascii: "\n")) {
                        discardingOversized = false
                        buffer.append(contentsOf: chunk[(nl + 1)..<n])
                        return .some(nil)
                    }
                    continue
                }
                buffer.append(contentsOf: chunk[..<n])
            }
        }

        private var discardingOversized = false
    }

    static func main() {
        // Tag the main queue for the reentrancy check in onMainQueueSync.
        DispatchQueue.main.setSpecific(key: mainQueueKey, value: 1)
        let manager = VmManager()
        let parent = getppid()
        if parent <= 1 {
            log("parent already gone; exiting")
            exit(0)
        }
        // Parent death without a clean EOF (e.g. SIGKILL of the app while a
        // grandchild still holds stdin) must still take the VMs down.
        let watch = DispatchSource.makeProcessSource(identifier: parent, eventMask: .exit,
                                                     queue: .global())
        watch.setEventHandler { shutdownAndExit(manager, reason: "parent exited") }
        watch.resume()
        Thread.detachNewThread {
            commandLoop(manager: manager)
            shutdownAndExit(manager, reason: "stdin closed")
        }
        RunLoop.main.run() // main-queue blocks drain on main
        _ = watch
    }

    private static let shutdownLock = NSLock()
    private static var shuttingDown = false
    /// Held while a command runs. `VmManager`'s tables belong to the
    /// command thread; the parent-exit handler (a global queue) takes this
    /// before touching them, so the two never race.
    private static let commandLock = NSLock()
    /// How long shutdown waits for an in-flight command before exiting
    /// without the graceful stop (the VMs die with this process anyway).
    private static let shutdownCommandWaitSeconds: TimeInterval = 20

    /// Stop every VM this helper owns, then exit. Idempotent.
    static func shutdownAndExit(_ manager: VmManager, reason: String) {
        shutdownLock.lock()
        if shuttingDown { shutdownLock.unlock(); return }
        shuttingDown = true
        shutdownLock.unlock()
        log("\(reason): stopping all VMs and exiting")
        // Never released: no command may start once shutdown owns the VMs.
        if commandLock.lock(before: Date().addingTimeInterval(shutdownCommandWaitSeconds)) {
            manager.shutdownAll()
        } else {
            log("a command is still running; exiting without graceful VM stop")
        }
        exit(0)
    }

    static func commandLoop(manager: VmManager) {
        let decoder = JSONDecoder()
        let reader = LineReader()
        while true {
            guard let lineOpt = reader.next() else { return } // EOF: parent went away
            guard let line = lineOpt, !line.isEmpty,
                  let json = line.data(using: .utf8) else { continue }
            guard let req = try? decoder.decode(IncomingRequest.self, from: json) else {
                // Malformed line: respond with an error envelope when an id exists.
                if let raw = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
                   let id = raw["id"] as? UInt64 {
                    emit(HostResponse.fail(id: id, code: "invalid_params",
                                           message: "malformed request"))
                } else {
                    log("dropping malformed line without id")
                }
                continue
            }
            // Vz calls run on the main queue (via VmManager's routing);
            // this background thread simply waits for the response.
            commandLock.lock()
            let response = handle(req, manager: manager)
            commandLock.unlock()
            emit(response)
        }
    }

    static func handle(_ req: IncomingRequest, manager: VmManager) -> HostResponse {
        do {
            switch req.command {
            case "version":
                return .ok(id: req.id, version: hostVersion)
            case "validate":
                guard let p = req.params else {
                    return .fail(id: req.id, code: "invalid_params", message: "validate needs params")
                }
                try manager.validate(params: p)
                return .ok(id: req.id, state: "stopped")
            case "create":
                guard let p = req.params else {
                    return .fail(id: req.id, code: "invalid_params", message: "create needs params")
                }
                let state = try manager.create(params: p)
                return .ok(id: req.id, state: state)
            case "start":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "start needs computer_id")
                }
                return .ok(id: req.id, state: try manager.start(computerId: id))
            case "pause":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "pause needs computer_id")
                }
                return .ok(id: req.id, state: try manager.pause(computerId: id))
            case "resume":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "resume needs computer_id")
                }
                return .ok(id: req.id, state: try manager.resume(computerId: id))
            case "stop":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "stop needs computer_id")
                }
                return .ok(id: req.id, state: try manager.stop(computerId: id))
            case "state":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "state needs computer_id")
                }
                return .ok(id: req.id, state: try manager.state(computerId: id))
            case "destroy":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "destroy needs computer_id")
                }
                try manager.destroy(computerId: id)
                return .ok(id: req.id, state: "stopped")
            case "guest_send":
                guard let id = req.computer_id, let payload = req.payload else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "guest_send needs computer_id + payload")
                }
                try manager.guestSend(computerId: id, payload: payload)
                return .ok(id: req.id)
            case "guest_status":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "guest_status needs computer_id")
                }
                return .ok(id: req.id, connected: try manager.guestStatus(computerId: id))
            case "guest_disconnect":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "guest_disconnect needs computer_id")
                }
                try manager.guestDisconnect(computerId: id)
                return .ok(id: req.id)
            case "egress_open":
                guard let id = req.computer_id, let endpoint = req.endpoint else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "egress_open needs computer_id + endpoint")
                }
                try manager.egressOpen(computerId: id, endpoint: endpoint)
                return .ok(id: req.id)
            case "egress_close":
                guard let id = req.computer_id else {
                    return .fail(id: req.id, code: "invalid_params",
                                 message: "egress_close needs computer_id")
                }
                try manager.egressClose(computerId: id)
                return .ok(id: req.id)
            default:
                // Closed command set: anything else is rejected, never executed.
                return .fail(id: req.id, code: "unknown_command",
                             message: "unsupported command: \(req.command)")
            }
        } catch let VmHostError.failure(code, message) {
            return .fail(id: req.id, code: code, message: message)
        } catch {
            return .fail(id: req.id, code: "internal", message: error.localizedDescription)
        }
    }
}
