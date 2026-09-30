//! Client-side helpers shared by the test doubles and the tests.

use tokio::io::AsyncRead;

use super::body::BodyReader;
use super::http::{request_framing, response_framing};
pub(crate) use super::http::{Buffered, RequestHead, ResponseHead};

const IDLE: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) struct Response {
    pub head: ResponseHead,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<String> {
        self.head
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| String::from_utf8_lossy(&h.value).to_string())
    }
}

/// Reads one complete response (head and decoded body).
pub(crate) async fn read_response<S: AsyncRead + Unpin>(
    b: &mut Buffered<S>,
    head_request: bool,
) -> Response {
    let head = b.read_response_head(IDLE).await.expect("response head");
    let framing = response_framing(head_request, &head).expect("response framing");
    let mut reader = BodyReader::new(framing);
    let mut body = Vec::new();
    while let Some(p) = reader.next(b, IDLE).await.expect("response body") {
        body.extend_from_slice(&p);
    }
    Response { head, body }
}

/// Reads a request body (server side of the test upstream).
pub(crate) async fn read_request_body<S: AsyncRead + Unpin>(
    b: &mut Buffered<S>,
    req: &RequestHead,
) -> Vec<u8> {
    let framing = request_framing(req, u64::MAX).expect("request framing");
    let mut reader = BodyReader::new(framing);
    let mut body = Vec::new();
    while let Ok(Some(p)) = reader.next(b, IDLE).await {
        body.extend_from_slice(&p);
    }
    body
}
