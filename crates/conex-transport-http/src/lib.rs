//! Verified HTTPS/HTTP egress. Peer identity is proven before any business
//! request is sent (design S8.4); no connection pooling in P0.
#![forbid(unsafe_code)]

pub mod connect;
pub mod connection;
pub mod resolve;

pub use connect::{HttpConnector, TlsTrustConfig};
pub use connection::HttpConnection;
pub use resolve::TokioResolver;

use tokio::io::{AsyncRead, AsyncWrite};

/// Object-safe async byte stream used by the hyper client connection.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}
