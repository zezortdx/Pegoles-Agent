/// Entry point: JSON Lines command loop on stdin/stdout.
/// One JSON object per line in, one JSON object per line out.
/// Anything that is not valid JSON gets an error response, never a crash.
///
/// Threading: Virtualization.framework asserts main-queue usage, so the
/// main thread pumps the main runloop and never blocks. Blocking stdin
/// reads happen on a background thread, which hops each command onto the
/// main thread and waits for the response.
///
/// NOTE (macOS 26): `dispatchMain()` drains main-queue blocks off the main
/// thread; only `RunLoop.main.run()` keeps them on it. VZ requires the
/// latter, so we use the runloop here.

import Foundation

@main
struct PegolesVmHost {
    /// Blocking line reader. NOTE: `read(upToCount:)` with a large count
    /// waits for that many bytes or EOF, which deadlocks a control channel
    /// whose stdin stays open. Single-byte reads return as soon as input
    /// is available; `nil` (empty) means EOF.
    static func readLine() -> String?? {
        let stdin = FileHandle.standardInput
        var line = Data()
        while true {
            let chunk: Data?
            do {
                chunk = try stdin.read(upToCount: 1)
            } catch {
                log("stdin read failed: \(error.localizedDescription)")
                return line.isEmpty ? nil : .some(nil)
            }
            guard let byte = chunk, !byte.isEmpty else {
                return line.isEmpty ? nil : .some(nil) // EOF
            }
            if byte[0] == UInt8(ascii: "\n") {
                return .some(String(data: line, encoding: .utf8))
            }
            line.append(byte)
        }
    }

    static func main() {
        // Tag the main queue for the reentrancy check in onMainQueueSync.
        DispatchQueue.main.setSpecific(key: mainQueueKey, value: 1)
        let manager = VmManager()
        Thread.detachNewThread {
            commandLoop(manager: manager)
        }
        RunLoop.main.run() // never returns; main-queue blocks drain on main
    }

    static func commandLoop(manager: VmManager) {
        let decoder = JSONDecoder()
        while true {
            let maybeLine = readLine()
            guard let lineOpt = maybeLine else { break } // EOF: parent went away
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
            let response = handle(req, manager: manager)
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
