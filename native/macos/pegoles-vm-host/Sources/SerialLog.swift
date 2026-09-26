/// Bounded guest serial console capture.
///
/// The console is guest output, so it is untrusted: a guest writing to
/// hvc0 without end must not fill the host disk. Output flows through a
/// pipe into this reader, which appends to `serial.log` and rotates it to
/// `serial.log.1` past `serialLogMaxBytes` (at most two files on disk).

import Foundation
import Virtualization

let serialLogMaxBytes: UInt64 = 8 * 1024 * 1024

final class SerialLog {
    let attachment: VZFileHandleSerialPortAttachment
    private let pipe = Pipe()
    private let path: String

    init(path: String) throws {
        self.path = path
        guard FileManager.default.createFile(atPath: path, contents: nil,
                                             attributes: [.posixPermissions: 0o600]) else {
            throw VmHostError.failure(code: "invalid_params",
                                      message: "cannot open serial log at \(path)")
        }
        attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: nil,
            fileHandleForWriting: pipe.fileHandleForWriting)
        let reader = pipe.fileHandleForReading
        let thread = Thread { [path] in SerialLog.drain(reader, to: path) }
        thread.name = "pegoles.serial"
        thread.start()
    }

    /// Close the write end so the reader sees EOF and exits (VM removed).
    func close() {
        try? pipe.fileHandleForWriting.close()
    }

    private static func drain(_ reader: FileHandle, to path: String) {
        var out = FileHandle(forWritingAtPath: path)
        var size: UInt64 = 0
        while true {
            let data = reader.availableData
            if data.isEmpty { break } // VM gone: write end closed
            if size + UInt64(data.count) > serialLogMaxBytes {
                try? out?.close()
                let rotated = path + ".1"
                try? FileManager.default.removeItem(atPath: rotated)
                try? FileManager.default.moveItem(atPath: path, toPath: rotated)
                FileManager.default.createFile(atPath: path, contents: nil,
                                               attributes: [.posixPermissions: 0o600])
                out = FileHandle(forWritingAtPath: path)
                size = 0
            }
            out?.write(data)
            size += UInt64(data.count)
        }
        try? out?.close()
    }
}
