//! Magic-byte classification. Signatures are matched on the decoded body
//! head, never on the declared type.

/// Executable and archive formats that are blocked whatever the declared
/// type says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// MZ: DOS/PE executables, DLLs, installers.
    Pe,
    Elf,
    MachO,
    /// `CAFEBABE` / `BFBAFECA`: Mach-O universal binary (also Java class).
    MachOFat,
    /// `#!` script.
    Shebang,
    /// ZIP, JAR, APK, Office OOXML.
    Zip,
    SevenZip,
    Rar,
    Gzip,
    Xz,
    Bzip2,
    Zstd,
    Cab,
    /// OLE compound file: MSI and legacy Office.
    Ole,
    /// `ar` archive: deb, static libraries.
    Ar,
    /// xar: macOS pkg.
    Xar,
    /// Apple UDIF disk image (trailer `koly`).
    Udif,
    /// ISO 9660.
    Iso,
}

/// Formats that may pass as a download when the declared type agrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Safe {
    Pdf,
    Png,
    Jpeg,
    Gif,
    Webp,
}

/// Offset of the ISO 9660 primary volume descriptor identifier `CD001`.
const ISO_OFFSET: usize = 0x8001;
/// Bytes of the UDIF trailer.
const UDIF_TRAILER: usize = 512;

/// Checks the head for an executable or archive signature. `complete`: the
/// slice is the whole body, which enables the end-of-file UDIF check.
pub fn sniff_blocked(head: &[u8], complete: bool) -> Option<Blocked> {
    const PREFIXES: &[(&[u8], Blocked)] = &[
        (b"MZ", Blocked::Pe),
        (b"\x7fELF", Blocked::Elf),
        (b"\xfe\xed\xfa\xce", Blocked::MachO),
        (b"\xfe\xed\xfa\xcf", Blocked::MachO),
        (b"\xce\xfa\xed\xfe", Blocked::MachO),
        (b"\xcf\xfa\xed\xfe", Blocked::MachO),
        (b"\xca\xfe\xba\xbe", Blocked::MachOFat),
        (b"\xca\xfe\xba\xbf", Blocked::MachOFat),
        (b"\xbe\xba\xfe\xca", Blocked::MachOFat),
        (b"\xbf\xba\xfe\xca", Blocked::MachOFat),
        (b"#!", Blocked::Shebang),
        (b"PK\x03\x04", Blocked::Zip),
        (b"PK\x05\x06", Blocked::Zip),
        (b"PK\x07\x08", Blocked::Zip),
        (b"7z\xbc\xaf\x27\x1c", Blocked::SevenZip),
        (b"Rar!\x1a\x07", Blocked::Rar),
        (b"\x1f\x8b", Blocked::Gzip),
        (b"\xfd7zXZ\x00", Blocked::Xz),
        (b"BZh", Blocked::Bzip2),
        (b"\x28\xb5\x2f\xfd", Blocked::Zstd),
        (b"MSCF", Blocked::Cab),
        (b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", Blocked::Ole),
        (b"!<arch>\n", Blocked::Ar),
        (b"xar!", Blocked::Xar),
    ];
    for (sig, kind) in PREFIXES {
        if head.starts_with(sig) {
            return Some(*kind);
        }
    }
    if head.len() >= ISO_OFFSET + 5 && &head[ISO_OFFSET..ISO_OFFSET + 5] == b"CD001" {
        return Some(Blocked::Iso);
    }
    if complete
        && head.len() >= UDIF_TRAILER
        && head[head.len() - UDIF_TRAILER..].starts_with(b"koly")
    {
        return Some(Blocked::Udif);
    }
    None
}

/// Safe-download signatures at offset 0.
pub fn sniff_safe(head: &[u8]) -> Option<Safe> {
    if head.starts_with(b"%PDF-") {
        Some(Safe::Pdf)
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Safe::Png)
    } else if head.starts_with(b"\xff\xd8\xff") {
        Some(Safe::Jpeg)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some(Safe::Gif)
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some(Safe::Webp)
    } else {
        None
    }
}

/// Plain text: no NUL, almost no control bytes, UTF-8 (a character cut at
/// the end of the window is tolerated).
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return true;
    }
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        Err(e) if e.error_len().is_none() => {
            // truncated multi-byte sequence at the end
            match std::str::from_utf8(&head[..e.valid_up_to()]) {
                Ok(t) => t,
                Err(_) => return false,
            }
        }
        Err(_) => return false,
    };
    let bad = text
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{c}'))
        .count();
    bad == 0
}
