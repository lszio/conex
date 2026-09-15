//! One verified HTTP/1.1 connection. Requests never leave the installed path.
use async_trait::async_trait;
use bytes::Bytes;
use conex_core::{
    CallError, CallResult, Connection, OutboundRequest, OutboundResponse, VerifiedPeer,
};
use conex_proto::v1;
use http_body_util::Full;
use hyper::body::Incoming;
use tokio::time::{Instant, timeout_at};

pub struct HttpConnection {
    pub(crate) peer: VerifiedPeer,
    pub(crate) sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    pub(crate) max_response_bytes: usize,
    pub(crate) fixed_path: String,
}

#[async_trait]
impl Connection for HttpConnection {
    fn peer(&self) -> &VerifiedPeer {
        &self.peer
    }

    async fn request(
        &mut self,
        request: OutboundRequest,
        deadline: Instant,
    ) -> CallResult<OutboundResponse> {
        if request.path.starts_with("http://") || request.path.starts_with("https://") {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "absolute request URLs are not allowed",
            ));
        }
        if !request.path.starts_with(&self.fixed_path) {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "request path is outside the installed fixedPath",
            ));
        }
        let OutboundRequest {
            method,
            path,
            headers,
            body,
        } = request;
        let mut built = http::Request::builder()
            .method(method)
            .uri(&path)
            .body(Full::new(body))
            .map_err(|error| {
                CallError::new(v1::ErrorCode::BadRequest, format!("build request: {error}"))
            })?;
        *built.headers_mut() = headers;

        let response = timeout_at(deadline, self.sender.send_request(built))
            .await
            .map_err(|_| {
                CallError::new(v1::ErrorCode::Timeout, "http request exceeded the deadline")
            })?
            .map_err(|error| {
                CallError::new(
                    v1::ErrorCode::Unavailable,
                    format!("http request failed: {error}"),
                )
            })?;

        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(CallError::new(
                v1::ErrorCode::Unavailable,
                "redirects are not followed",
            ));
        }
        let headers = response.headers().clone();
        let body = timeout_at(
            deadline,
            read_bounded(response.into_body(), self.max_response_bytes),
        )
        .await
        .map_err(|_| {
            CallError::new(
                v1::ErrorCode::Timeout,
                "http response body exceeded the deadline",
            )
        })??;
        Ok(OutboundResponse {
            status,
            headers,
            body,
        })
    }
}

async fn read_bounded(mut body: Incoming, max: usize) -> CallResult<Bytes> {
    use http_body_util::BodyExt;
    let mut out: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| {
            CallError::new(
                v1::ErrorCode::Unavailable,
                format!("response body error: {error}"),
            )
        })?;
        if let Some(chunk) = frame.data_ref() {
            if out.len() + chunk.len() > max {
                return Err(CallError::new(
                    v1::ErrorCode::PayloadTooLarge,
                    "response body exceeds max_response_bytes",
                ));
            }
            out.extend_from_slice(chunk);
        }
    }
    Ok(Bytes::from(out))
}
