//! Host assembly ports that need OS access, plus the authenticated HTTP entry.
#![forbid(unsafe_code)]

pub mod auth;
pub mod binding;
pub mod credentials;
pub mod http;

pub use auth::*;
pub use binding::*;
pub use credentials::*;
pub use http::*;
