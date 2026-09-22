/// Wire protocol mirror of `crates/pegoles-computer/src/vmhost_proto.rs`.
/// These types must stay field-compatible with the Rust side.
/// Full contract: `docs/VM_HOST_PROTOCOL.md`.
///
/// SECURITY: the command set is closed. There is intentionally no shell
/// execution, no host file operation, no network proxy, no arbitrary
/// native call. Unknown commands are rejected.

import Foundation

let hostVersion = "0.1.0"

struct CreateParams: Codable {
    let computer_id: String
    let disk_path: String
    let efi_vars_path: String
    let machine_id_path: String
    let serial_log_path: String
    let vcpus: Int
    let memory_mb: UInt64
    /// Build-time provisioning only (cloud-init seed ISO). Never set in
    /// the normal lifecycle; the guest boots from disk.img alone.
    let seed_iso_path: String?
}

struct IncomingRequest: Decodable {
    let id: UInt64
    let command: String
    let computer_id: String?
    let params: CreateParams?
    /// Frame payload for `guest_send` (one JSONL line, no `\n`).
    let payload: String?
}

struct HostError: Codable {
    let code: String
    let message: String
}

struct HostResponse: Codable {
    let id: UInt64
    let ok: Bool
    let state: String?
    let version: String?
    let connected: Bool?
    let error: HostError?

    static func ok(id: UInt64, state: String? = nil, version: String? = nil,
                   connected: Bool? = nil) -> HostResponse {
        HostResponse(id: id, ok: true, state: state, version: version,
                     connected: connected, error: nil)
    }

    static func fail(id: UInt64, code: String, message: String) -> HostResponse {
        HostResponse(id: id, ok: false, state: nil, version: nil, connected: nil,
                     error: HostError(code: code, message: message))
    }
}

struct HostEvent: Codable {
    let event: String
    let computer_id: String
    let state: String?
    let message: String?
    /// Guest frame content for `guest_frame`.
    let payload: String?
    /// Disconnect cause for `guest_disconnected`.
    let reason: String?
}

private let encoder: JSONEncoder = {
    let e = JSONEncoder()
    e.outputFormatting = [.sortedKeys]
    return e
}()

/// All stdout writes serialize here: responses are emitted from the main
/// command loop while delegate events may arrive on other threads.
private let emitQueue = DispatchQueue(label: "pegoles.emit")

/// Unbuffered line write to stdout (stdout to a pipe is block-buffered,
/// so `print` alone would delay responses indefinitely).
func emitLine(_ string: String) {
    emitQueue.sync {
        guard let data = (string + "\n").data(using: .utf8) else { return }
        FileHandle.standardOutput.write(data)
    }
}

func emit<T: Encodable>(_ value: T) {
    guard let data = try? encoder.encode(value),
          let string = String(data: data, encoding: .utf8) else { return }
    emitLine(string)
}

func log(_ message: String) {
    // Diagnostics only; Rust parses stdout, never stderr.
    if let data = "pegoles-vm-host: \(message)\n".data(using: .utf8) {
        FileHandle.standardError.write(data)
    }
}
