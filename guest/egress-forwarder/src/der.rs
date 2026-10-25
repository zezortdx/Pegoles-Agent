//! Minimal, strict validation of the CA certificate the host sends in HELLO.
//!
//! Not a certificate verifier: Chromium does the real parsing. This only
//! makes sure that what lands in the managed-policy file is one well-formed
//! DER X.509 v3 certificate whose basicConstraints says CA=TRUE, so a
//! malformed or non-CA blob is never handed to Chromium. Definite lengths
//! only, minimal length encodings, single-byte tags, no trailing bytes.

use std::fmt;

use pegoles_egress_proto::MAX_CA_DER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerError(pub &'static str);

impl fmt::Display for DerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DerError {}

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const BOOLEAN: u8 = 0x01;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const VERSION: u8 = 0xA0;
const ISSUER_UID: u8 = 0x81;
const SUBJECT_UID: u8 = 0x82;
const EXTENSIONS: u8 = 0xA3;
/// 2.5.29.19 basicConstraints.
const BASIC_CONSTRAINTS: [u8; 3] = [0x55, 0x1D, 0x13];

/// One TLV: (tag, value, bytes after it).
fn tlv(input: &[u8]) -> Result<(u8, &[u8], &[u8]), DerError> {
    let (&tag, rest) = input.split_first().ok_or(DerError("truncated"))?;
    if tag & 0x1F == 0x1F {
        return Err(DerError("multi-byte tag"));
    }
    let (&first, rest) = rest.split_first().ok_or(DerError("truncated"))?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7F);
        if n == 0 || n > 2 {
            return Err(DerError("unsupported length form"));
        }
        if rest.len() < n {
            return Err(DerError("truncated"));
        }
        let (bytes, rest) = rest.split_at(n);
        if bytes[0] == 0 {
            return Err(DerError("non-minimal length"));
        }
        let len = bytes.iter().fold(0usize, |a, &b| (a << 8) | usize::from(b));
        if len < 0x80 {
            return Err(DerError("non-minimal length"));
        }
        (len, rest)
    };
    if rest.len() < len {
        return Err(DerError("truncated"));
    }
    let (value, rest) = rest.split_at(len);
    Ok((tag, value, rest))
}

/// A TLV that must carry `tag`: (value, bytes after it).
fn expect(input: &[u8], tag: u8) -> Result<(&[u8], &[u8]), DerError> {
    let (t, value, rest) = tlv(input)?;
    if t != tag {
        return Err(DerError("unexpected tag"));
    }
    Ok((value, rest))
}

fn finished(rest: &[u8]) -> Result<(), DerError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(DerError("trailing bytes"))
    }
}

/// Checks that `der` is a single X.509 v3 certificate with
/// basicConstraints CA=TRUE.
pub fn validate_ca_certificate(der: &[u8]) -> Result<(), DerError> {
    if der.is_empty() || der.len() > MAX_CA_DER {
        return Err(DerError("certificate size out of range"));
    }
    let (cert, rest) = expect(der, SEQUENCE)?;
    finished(rest)?;
    let (tbs, rest) = expect(cert, SEQUENCE)?;
    let (_sig_alg, rest) = expect(rest, SEQUENCE)?;
    let (sig, rest) = expect(rest, BIT_STRING)?;
    finished(rest)?;
    if sig.is_empty() {
        return Err(DerError("empty signature"));
    }

    // TBSCertificate: v3 only (extensions do not exist before v3).
    let (version, t) = expect(tbs, VERSION)?;
    let (v, vrest) = expect(version, INTEGER)?;
    finished(vrest)?;
    if v != [2] {
        return Err(DerError("not an X.509 v3 certificate"));
    }
    let (serial, t) = expect(t, INTEGER)?;
    if serial.is_empty() {
        return Err(DerError("empty serial"));
    }
    // signature alg, issuer, validity, subject, subjectPublicKeyInfo
    let mut t = t;
    for _ in 0..5 {
        t = expect(t, SEQUENCE)?.1;
    }
    for uid in [ISSUER_UID, SUBJECT_UID] {
        if t.first() == Some(&uid) {
            t = expect(t, uid)?.1;
        }
    }
    let (wrapped, t) = expect(t, EXTENSIONS)?;
    finished(t)?;
    let (exts, rest) = expect(wrapped, SEQUENCE)?;
    finished(rest)?;
    check_extensions(exts)
}

fn check_extensions(mut exts: &[u8]) -> Result<(), DerError> {
    let mut seen = false;
    while !exts.is_empty() {
        let (ext, rest) = expect(exts, SEQUENCE)?;
        exts = rest;
        let (oid, mut body) = expect(ext, OID)?;
        if body.first() == Some(&BOOLEAN) {
            let (critical, rest) = expect(body, BOOLEAN)?;
            if critical.len() != 1 {
                return Err(DerError("bad critical flag"));
            }
            body = rest;
        }
        let (value, rest) = expect(body, OCTET_STRING)?;
        finished(rest)?;
        if oid == BASIC_CONSTRAINTS {
            if seen {
                return Err(DerError("duplicate basicConstraints"));
            }
            seen = true;
            check_basic_constraints(value)?;
        }
    }
    if seen {
        Ok(())
    } else {
        Err(DerError("no basicConstraints"))
    }
}

fn check_basic_constraints(value: &[u8]) -> Result<(), DerError> {
    let (seq, rest) = expect(value, SEQUENCE)?;
    finished(rest)?;
    // cA defaults to FALSE, so it must be present and TRUE (0xFF in DER).
    if seq.first() != Some(&BOOLEAN) {
        return Err(DerError("not a CA"));
    }
    let (ca, rest) = expect(seq, BOOLEAN)?;
    if ca != [0xFF] {
        return Err(DerError("not a CA"));
    }
    if !rest.is_empty() {
        let (_path_len, rest) = expect(rest, INTEGER)?;
        finished(rest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(is_ca: bool) -> Vec<u8> {
        let mut params = rcgen::CertificateParams::new(vec!["x.test".to_string()]).unwrap();
        if is_ca {
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        }
        let key = rcgen::KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().der().to_vec()
    }

    #[test]
    fn accepts_a_ca_certificate() {
        assert_eq!(validate_ca_certificate(&cert(true)), Ok(()));
    }

    #[test]
    fn rejects_a_non_ca_certificate() {
        assert!(validate_ca_certificate(&cert(false)).is_err());
    }

    #[test]
    fn rejects_trailing_bytes_and_every_truncation() {
        let der = cert(true);
        let mut extra = der.clone();
        extra.push(0);
        assert!(validate_ca_certificate(&extra).is_err());
        for n in 0..der.len() {
            assert!(validate_ca_certificate(&der[..n]).is_err(), "prefix {n}");
        }
    }

    #[test]
    fn rejects_garbage_and_oversize_without_panicking() {
        assert!(validate_ca_certificate(&[]).is_err());
        assert!(validate_ca_certificate(b"not a certificate").is_err());
        assert!(validate_ca_certificate(&[0x30, 0x80, 0, 0]).is_err());
        assert!(validate_ca_certificate(&[0x30, 0x82, 0xFF, 0xFF]).is_err());
        assert!(validate_ca_certificate(&vec![0x30; MAX_CA_DER + 1]).is_err());
    }

    #[test]
    fn single_byte_flips_never_panic() {
        let der = cert(true);
        for i in 0..der.len() {
            let mut m = der.clone();
            m[i] ^= 0xFF;
            let _ = validate_ca_certificate(&m);
        }
    }

    #[test]
    fn rejects_non_minimal_lengths() {
        // 0x81 0x05 encodes a short length the long way.
        assert!(tlv(&[0x30, 0x81, 0x05, 1, 2, 3, 4, 5]).is_err());
        assert!(tlv(&[0x30, 0x82, 0x00, 0x90]).is_err());
    }
}
