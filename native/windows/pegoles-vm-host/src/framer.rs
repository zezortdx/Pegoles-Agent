//! Byte stream → guest frames: split on `\n` (one `\r` stripped), 64 KiB
//! cap, UTF-8 only. Shared by the live socket pump and the tests.

use pegoles_guest_proto::MAX_FRAME_BYTES;

pub struct StreamFramer {
    pending: Vec<u8>,
}

impl StreamFramer {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Complete lines from `bytes`; the tail stays buffered. Any violation
    /// ends the stream (the caller disconnects).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, &'static str> {
        if self.pending.len() + bytes.len() > MAX_FRAME_BYTES + 4096 {
            self.pending.clear();
            return Err("frame_too_large");
        }
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > MAX_FRAME_BYTES {
                self.pending.clear();
                return Err("frame_too_large");
            }
            out.push(String::from_utf8(line).map_err(|_| "invalid_utf8")?);
        }
        if self.pending.len() > MAX_FRAME_BYTES {
            self.pending.clear();
            return Err("frame_too_large");
        }
        Ok(out)
    }
}

impl Default for StreamFramer {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse "00000352-facb-11e6-bd58-64006a7986d3" into GUID parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    pub fn parse(s: &str) -> Option<Self> {
        let p: Vec<&str> = s.split('-').collect();
        if p.len() != 5
            || p[0].len() != 8
            || p[1].len() != 4
            || p[2].len() != 4
            || p[3].len() != 4
            || p[4].len() != 12
        {
            return None;
        }
        let mut data4 = [0u8; 8];
        let tail = format!("{}{}", p[3], p[4]);
        for (i, chunk) in tail.as_bytes().chunks(2).enumerate() {
            data4[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
        }
        Some(Self {
            data1: u32::from_str_radix(p[0], 16).ok()?,
            data2: u16::from_str_radix(p[1], 16).ok()?,
            data3: u16::from_str_radix(p[2], 16).ok()?,
            data4,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_partials_batches_and_caps() {
        let mut f = StreamFramer::new();
        assert!(f.push(b"{\"a\":").unwrap().is_empty());
        assert_eq!(
            f.push(b"1}\n{\"b\":2}\n").unwrap(),
            vec!["{\"a\":1}", "{\"b\":2}"]
        );
        let mut f = StreamFramer::new();
        assert_eq!(f.push(b"x\r\ny\n").unwrap(), vec!["x", "y"]);
        let mut f = StreamFramer::new();
        assert!(f.push(&vec![b'z'; MAX_FRAME_BYTES + 1]).is_err());
        let mut f = StreamFramer::new();
        assert!(f.push(b"\xff\xfe\n").is_err());
    }

    #[test]
    fn guids_parse_and_match_the_listen_port() {
        let g = Guid::parse(&pegoles_computer::pegoles_hyperv_service_guid()).unwrap();
        assert_eq!(g.data1, pegoles_guest_proto::PEGOLES_GUEST_LISTEN_PORT);
        assert_eq!(g.data2, 0xfacb);
        assert_eq!(g.data4, [0xbd, 0x58, 0x64, 0x00, 0x6a, 0x79, 0x86, 0xd3]);
        assert!(Guid::parse("not-a-guid").is_none());
        assert!(Guid::parse("0f8fad5b-d9cb-469f-a165-70867728950e").is_some());
    }
}
