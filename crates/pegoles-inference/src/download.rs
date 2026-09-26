//! HTTPS fetcher for pinned Hugging Face revisions, with byte-range
//! resume. Integrity is not this module's job: the store hashes every
//! byte against the compiled-in catalog, so a hostile mirror or redirect
//! can waste bandwidth but never install different weights.

use std::io::Read;
use std::time::Duration;

use crate::catalog::{ModelFile, ModelSource, ModelSpec};
use crate::store::{Fetcher, StoreError};

/// Bytes per ranged request. Each request has its own time bound, so a
/// stalled connection fails in bounded time and resumes at the chunk.
const CHUNK_BYTES: u64 = 64 * 1024 * 1024;

pub struct HttpsFetcher {
    agent: ureq::Agent,
    base: String,
}

impl Default for HttpsFetcher {
    fn default() -> Self {
        Self::new("https://huggingface.co")
    }
}

impl HttpsFetcher {
    pub fn new(base: &str) -> Self {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(8)
                .timeout_connect(Some(Duration::from_secs(20)))
                .timeout_recv_response(Some(Duration::from_secs(60)))
                // Whole-body bound for one CHUNK_BYTES range (>= ~0.4 MB/s).
                .timeout_recv_body(Some(Duration::from_secs(180)))
                .user_agent("Pegoles-ModelStore/0.1")
                .build(),
        );
        Self {
            agent,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn url(&self, spec: &ModelSpec, file: &ModelFile) -> Result<String, StoreError> {
        match &spec.source {
            ModelSource::Huggingface { repo, revision } => Ok(format!(
                "{}/{repo}/resolve/{revision}/{}",
                self.base, file.path
            )),
            _ => Err(StoreError::NotDownloadable(spec.id.clone())),
        }
    }
}

impl Fetcher for HttpsFetcher {
    fn fetch(
        &self,
        spec: &ModelSpec,
        file: &ModelFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), StoreError> {
        let url = self.url(spec, file)?;
        let mut at = offset;
        let mut buf = vec![0u8; 256 * 1024];
        while at < file.size {
            let end = (at + CHUNK_BYTES).min(file.size) - 1;
            let mut res = self
                .agent
                .get(&url)
                .header("Range", &format!("bytes={at}-{end}"))
                .call()
                .map_err(|e| StoreError::Network(short(&e.to_string())))?;
            let status = res.status().as_u16();
            // 206 = the range we asked for. A 200 carries the whole file,
            // acceptable only when that is what we asked for.
            if !(status == 206 || (status == 200 && at == 0 && end + 1 == file.size)) {
                return Err(StoreError::Network(format!(
                    "HTTP {status} for {} (range {at}-{end})",
                    file.path
                )));
            }
            let want = end + 1 - at;
            // ureq errors when a body reaches its limit; +1 lets exactly `want` pass.
            let mut body = res.body_mut().with_config().limit(want + 1).reader();
            let mut got: u64 = 0;
            loop {
                let n = body
                    .read(&mut buf)
                    .map_err(|e| StoreError::Network(short(&e.to_string())))?;
                if n == 0 {
                    break;
                }
                got += n as u64;
                if !sink(&buf[..n]) {
                    return Ok(());
                }
            }
            if got != want {
                return Err(StoreError::Network(format!(
                    "{} range ended early ({got} of {want} bytes)",
                    file.path
                )));
            }
            at = end + 1;
        }
        Ok(())
    }
}

fn short(s: &str) -> String {
    s.chars().take(300).collect()
}
