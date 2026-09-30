//! Message bodies: decoding of `Content-Length` / chunked / until-close
//! framing and re-encoding toward the other side. Chunk extensions and
//! trailers are dropped; every line is bounded.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use super::http::{Buffered, Framing, HttpError, MAX_HEAD, MAX_REQUEST_LINE, READ_CHUNK};

/// Longest chunk-size line (size plus extensions).
const MAX_CHUNK_LINE: usize = 4096;

#[derive(Clone, Copy, Debug)]
enum State {
    Length(u64),
    Close,
    ChunkSize,
    ChunkData(u64),
    ChunkEnd,
    Trailers(usize),
    Done,
}

/// Incremental decoder over a [`Buffered`] stream.
pub struct BodyReader {
    state: State,
}

impl BodyReader {
    pub fn new(framing: Framing) -> BodyReader {
        BodyReader {
            state: match framing {
                Framing::None | Framing::Length(0) => State::Done,
                Framing::Length(n) => State::Length(n),
                Framing::Chunked => State::ChunkSize,
                Framing::UntilClose => State::Close,
            },
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Next decoded piece (at most `READ_CHUNK` bytes), or `None` at the end
    /// of the body.
    pub async fn next<S: AsyncRead + Unpin>(
        &mut self,
        src: &mut Buffered<S>,
        idle: Duration,
    ) -> Result<Option<Vec<u8>>, HttpError> {
        loop {
            match self.state {
                State::Done => return Ok(None),
                State::Length(rem) => {
                    let piece = take(src, rem, idle).await?;
                    self.state = if rem == piece.len() as u64 {
                        State::Done
                    } else {
                        State::Length(rem - piece.len() as u64)
                    };
                    return Ok(Some(piece));
                }
                State::Close => {
                    if src.unread().is_empty() && src.fill(idle).await? == 0 {
                        self.state = State::Done;
                        return Ok(None);
                    }
                    return Ok(Some(take(src, u64::MAX, idle).await?));
                }
                State::ChunkData(rem) => {
                    let piece = take(src, rem, idle).await?;
                    self.state = if rem == piece.len() as u64 {
                        State::ChunkEnd
                    } else {
                        State::ChunkData(rem - piece.len() as u64)
                    };
                    return Ok(Some(piece));
                }
                State::ChunkEnd => {
                    while src.unread().len() < 2 {
                        if src.fill(idle).await? == 0 {
                            return Err(HttpError::Eof);
                        }
                    }
                    if &src.unread()[..2] != b"\r\n" {
                        return Err(HttpError::Malformed);
                    }
                    src.consume(2);
                    self.state = State::ChunkSize;
                }
                State::ChunkSize => {
                    let line = read_line(src, MAX_CHUNK_LINE, idle).await?;
                    self.state = match parse_chunk_size(&line)? {
                        0 => State::Trailers(0),
                        n => State::ChunkData(n),
                    };
                }
                State::Trailers(total) => {
                    let line = read_line(src, MAX_REQUEST_LINE, idle).await?;
                    if line.is_empty() {
                        self.state = State::Done;
                    } else {
                        let total = total + line.len() + 2;
                        if total > MAX_HEAD {
                            return Err(HttpError::Malformed);
                        }
                        self.state = State::Trailers(total);
                    }
                }
            }
        }
    }
}

/// Takes up to `max` bytes (and one read chunk) from the buffer, reading
/// first if it is empty. EOF with bytes still owed is an error.
async fn take<S: AsyncRead + Unpin>(
    src: &mut Buffered<S>,
    max: u64,
    idle: Duration,
) -> Result<Vec<u8>, HttpError> {
    if src.unread().is_empty() && src.fill(idle).await? == 0 {
        return Err(HttpError::Eof);
    }
    let n = src
        .unread()
        .len()
        .min(READ_CHUNK)
        .min(usize::try_from(max).unwrap_or(usize::MAX));
    let piece = src.unread()[..n].to_vec();
    src.consume(n);
    Ok(piece)
}

/// One CRLF-terminated line (terminator removed). Bare LF is refused.
async fn read_line<S: AsyncRead + Unpin>(
    src: &mut Buffered<S>,
    max: usize,
    idle: Duration,
) -> Result<Vec<u8>, HttpError> {
    loop {
        let unread = src.unread();
        if let Some(i) = unread.iter().position(|b| *b == b'\n') {
            if i == 0 || unread[i - 1] != b'\r' || i > max {
                return Err(HttpError::Malformed);
            }
            let line = unread[..i - 1].to_vec();
            src.consume(i + 1);
            return Ok(line);
        }
        if unread.len() > max {
            return Err(HttpError::Malformed);
        }
        if src.fill(idle).await? == 0 {
            return Err(HttpError::Eof);
        }
    }
}

fn parse_chunk_size(line: &[u8]) -> Result<u64, HttpError> {
    let end = line.iter().position(|b| *b == b';').unwrap_or(line.len());
    let hex = &line[..end];
    if hex.is_empty() || hex.len() > 16 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return Err(HttpError::Malformed);
    }
    let s = std::str::from_utf8(hex).map_err(|_| HttpError::Malformed)?;
    u64::from_str_radix(s, 16).map_err(|_| HttpError::Malformed)
}

/// Re-encodes body pieces for the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// `Content-Length`, bytes pass through; or until-close.
    Plain,
    Chunked,
}

impl Encoding {
    pub fn for_framing(f: Framing) -> Encoding {
        if f == Framing::Chunked {
            Encoding::Chunked
        } else {
            Encoding::Plain
        }
    }

    pub async fn write<W: AsyncWrite + Unpin>(self, w: &mut W, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        match self {
            Encoding::Plain => w.write_all(data).await,
            Encoding::Chunked => {
                let mut out = Vec::with_capacity(data.len() + 12);
                out.extend_from_slice(format!("{:x}\r\n", data.len()).as_bytes());
                out.extend_from_slice(data);
                out.extend_from_slice(b"\r\n");
                w.write_all(&out).await
            }
        }
    }

    pub async fn finish<W: AsyncWrite + Unpin>(self, w: &mut W) -> io::Result<()> {
        match self {
            Encoding::Plain => Ok(()),
            Encoding::Chunked => w.write_all(b"0\r\n\r\n").await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(1);

    async fn read_all(framing: Framing, raw: &[u8]) -> Result<Vec<u8>, HttpError> {
        let mut src = Buffered::new(raw);
        let mut r = BodyReader::new(framing);
        let mut out = Vec::new();
        while let Some(p) = r.next(&mut src, IDLE).await? {
            assert!(p.len() <= READ_CHUNK);
            out.extend_from_slice(&p);
        }
        Ok(out)
    }

    #[tokio::test]
    async fn content_length_bodies() {
        assert_eq!(
            read_all(Framing::Length(5), b"helloEXTRA")
                .await
                .expect("ok"),
            b"hello"
        );
        assert_eq!(read_all(Framing::None, b"ignored").await.expect("ok"), b"");
        assert!(matches!(
            read_all(Framing::Length(10), b"short").await,
            Err(HttpError::Eof)
        ));
        let big = vec![7u8; 40_000];
        assert_eq!(
            read_all(Framing::Length(40_000), &big).await.expect("ok"),
            big
        );
    }

    #[tokio::test]
    async fn chunked_bodies_with_extensions_and_trailers() {
        let raw = b"5;ext=1\r\nhello\r\nA\r\n world!!!!\r\n0\r\nTrailer-X: y\r\n\r\nNEXT";
        assert_eq!(
            read_all(Framing::Chunked, raw).await.expect("ok"),
            b"hello world!!!!"
        );
        assert_eq!(
            read_all(Framing::Chunked, b"0\r\n\r\n").await.expect("ok"),
            b""
        );
        let mut src = Buffered::new(&raw[..]);
        let mut r = BodyReader::new(Framing::Chunked);
        while r.next(&mut src, IDLE).await.expect("ok").is_some() {}
        assert!(r.is_done());
        assert_eq!(src.unread(), b"NEXT", "pipelined bytes stay in the buffer");
    }

    #[tokio::test]
    async fn chunked_violations_are_malformed() {
        for raw in [
            &b"zz\r\nhi\r\n0\r\n\r\n"[..],
            b"\r\nhi\r\n0\r\n\r\n",
            b"2\nhi\r\n0\r\n\r\n",
            b"2\r\nhiXX0\r\n\r\n",
            b"fffffffffffffffff\r\n",
            b"-1\r\nhi\r\n",
            b"2 \r\nhi\r\n0\r\n\r\n",
        ] {
            assert!(
                matches!(
                    read_all(Framing::Chunked, raw).await,
                    Err(HttpError::Malformed)
                ),
                "{:?}",
                String::from_utf8_lossy(raw)
            );
        }
        assert!(matches!(
            read_all(Framing::Chunked, b"5\r\nhel").await,
            Err(HttpError::Eof)
        ));
        assert!(matches!(
            read_all(Framing::Chunked, b"0\r\nX: y\r\n").await,
            Err(HttpError::Eof)
        ));
        let long_ext = format!("5;{}\r\nhello\r\n0\r\n\r\n", "e".repeat(5000));
        assert!(matches!(
            read_all(Framing::Chunked, long_ext.as_bytes()).await,
            Err(HttpError::Malformed)
        ));
        let mut trailers = String::from("0\r\n");
        for _ in 0..200 {
            trailers.push_str(&format!("X: {}\r\n", "t".repeat(300)));
        }
        trailers.push_str("\r\n");
        assert!(matches!(
            read_all(Framing::Chunked, trailers.as_bytes()).await,
            Err(HttpError::Malformed)
        ));
    }

    #[tokio::test]
    async fn until_close_reads_to_eof() {
        assert_eq!(
            read_all(Framing::UntilClose, b"abc def").await.expect("ok"),
            b"abc def"
        );
        assert_eq!(read_all(Framing::UntilClose, b"").await.expect("ok"), b"");
    }

    #[tokio::test]
    async fn chunked_encoding_roundtrips() {
        let mut out = Vec::new();
        let enc = Encoding::Chunked;
        enc.write(&mut out, b"hello").await.expect("w");
        enc.write(&mut out, b"").await.expect("w");
        enc.write(&mut out, &[b'x'; 300]).await.expect("w");
        enc.finish(&mut out).await.expect("w");
        let back = read_all(Framing::Chunked, &out).await.expect("decode");
        assert_eq!(&back[..5], b"hello");
        assert_eq!(back.len(), 305);
        let mut plain = Vec::new();
        Encoding::Plain.write(&mut plain, b"abc").await.expect("w");
        Encoding::Plain.finish(&mut plain).await.expect("w");
        assert_eq!(plain, b"abc");
    }
}
