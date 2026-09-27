//! Shared, platform-free pieces of the local-model workers' protocol v1:
//! bounded line readers, the ordered request writer, the stderr drain and
//! the chat wire format. The MLX and llama.cpp supervisors both use them.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::backend::{ChatMessage, Part, Role};

pub const PROTOCOL_VERSION: u64 = 1;
/// Largest reply line accepted from the worker (text is capped at 32K
/// chars worker-side; this bounds a misbehaving or compromised worker).
pub(crate) const MAX_REPLY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_STDERR_LINES: usize = 40;
pub(crate) const MAX_STDERR_LINE_CHARS: usize = 400;
/// Bytes kept of one stderr line (room for `MAX_STDERR_LINE_CHARS` of any
/// UTF-8); the rest of a longer line is read and dropped.
pub(crate) const MAX_STDERR_LINE_BYTES: usize = MAX_STDERR_LINE_CHARS * 4;

pub(crate) enum Line {
    Reply(Value),
    Oversized,
    Invalid,
    Eof,
}

/// Writes request lines in order; stops at the first failed write (the
/// worker is gone or closed stdin, which its reply reader reports).
pub(crate) fn write_requests(mut stdin: impl Write, lines: Receiver<Vec<u8>>) {
    for line in lines {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
            return;
        }
    }
}

/// Keeps the worker's last `MAX_STDERR_LINES` stderr lines for error
/// messages. Reads raw chunks, so a line without a newline never grows
/// past `MAX_STDERR_LINE_BYTES`; decodes lossily; and drains until EOF
/// whatever the bytes are (a stopped drain would break the worker's
/// logging).
pub(crate) fn drain_stderr(mut pipe: impl Read, sink: &Mutex<VecDeque<String>>) {
    let push = |line: &[u8]| {
        let text = String::from_utf8_lossy(line)
            .chars()
            .take(MAX_STDERR_LINE_CHARS)
            .collect();
        let mut q = sink.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() == MAX_STDERR_LINES {
            q.pop_front();
        }
        q.push_back(text);
    };
    let mut buf = [0u8; 8192];
    let mut line = Vec::with_capacity(MAX_STDERR_LINE_BYTES);
    loop {
        let n = match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for piece in buf[..n].split_inclusive(|&b| b == b'\n') {
            let (body, complete) = match piece.split_last() {
                Some((b'\n', body)) => (body, true),
                _ => (piece, false),
            };
            let room = MAX_STDERR_LINE_BYTES - line.len();
            line.extend_from_slice(&body[..body.len().min(room)]);
            if complete {
                push(&line);
                line.clear();
            }
        }
    }
    if !line.is_empty() {
        push(&line);
    }
}

/// Bounded line reader: a line longer than `MAX_REPLY_BYTES` is never
/// buffered whole; the reader reports it and stops.
pub(crate) fn read_replies(stdout: impl Read, tx: mpsc::Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut buf = Vec::new();
        let n = match (&mut reader)
            .take(MAX_REPLY_BYTES as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) => n,
            Err(_) => {
                let _ = tx.send(Line::Eof);
                return;
            }
        };
        if n == 0 {
            let _ = tx.send(Line::Eof);
            return;
        }
        if buf.len() > MAX_REPLY_BYTES {
            let _ = tx.send(Line::Oversized);
            return;
        }
        let line = match serde_json::from_slice::<Value>(&buf) {
            Ok(v)
                if v.is_object()
                    && v.get("v").and_then(Value::as_u64) == Some(PROTOCOL_VERSION) =>
            {
                Line::Reply(v)
            }
            _ => Line::Invalid,
        };
        let stop = matches!(line, Line::Invalid);
        if tx.send(line).is_err() || stop {
            return;
        }
    }
}

pub(crate) fn wire_messages(messages: &[ChatMessage]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let content: Vec<Value> = m
                    .parts
                    .iter()
                    .map(|p| match p {
                        Part::Text(t) => json!({"type": "text", "text": t}),
                        Part::Image => json!({"type": "image"}),
                    })
                    .collect();
                json!({"role": role, "content": content})
            })
            .collect(),
    )
}
