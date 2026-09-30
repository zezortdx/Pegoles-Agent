//! The Chromium managed-policy file that carries the session CA.
//!
//! The directory is root-owned, so the file can only be written IN PLACE:
//! open the existing file (no `O_CREAT`, no rename), truncate, write the
//! complete JSON once, fsync. It is reset, never deleted.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

pub const EMPTY_POLICY: &str = "{\"CACertificates\": []}";

/// Policy JSON trusting exactly one CA (base64 DER, as Chromium expects).
pub fn policy_json(ca_der: &[u8]) -> String {
    format!("{{\"CACertificates\": [\"{}\"]}}", STANDARD.encode(ca_der))
}

pub fn install(path: &Path, ca_der: &[u8]) -> io::Result<()> {
    write_in_place(path, &policy_json(ca_der))
}

pub fn reset(path: &Path) -> io::Result<()> {
    write_in_place(path, EMPTY_POLICY)
}

fn write_in_place(path: &Path, json: &str) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).truncate(true).open(path)?;
    f.write_all(json.as_bytes())?;
    f.sync_all()
}
