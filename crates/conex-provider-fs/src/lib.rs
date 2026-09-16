//! Restricted filesystem source provider. Confinement uses cap-std directory
//! capabilities; core gets no fs branch (design J2).
#![forbid(unsafe_code)]

pub mod list;
pub mod read;
pub mod search;

use std::path::Path;
use std::sync::{Arc, Mutex};

use conex_core::{CallError, CallResult, Handler, Installation, Route};
use conex_proto::v1;
use conex_source::contracts::{SOURCE_LIST, SOURCE_READ, SOURCE_SEARCH};
use conex_source::pagination::{Clock, SnapshotCache, SnapshotLimits, SystemClock};

pub use read::{FsRoot, MAX_DOC_BYTES, ReadSnapshot};

pub fn factory(installation: &Installation) -> CallResult<Vec<Route>> {
    let root_path = installation
        .provider
        .get("root")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::Internal,
                "fs installation requires provider.root",
            )
        })?;
    let root = Arc::new(FsRoot::open(Path::new(root_path))?);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let cache = Arc::new(Mutex::new(SnapshotCache::new(
        clock,
        SnapshotLimits::default(),
    )));

    let mut routes = Vec::new();
    for (method, contract) in conex_source::contracts() {
        let handler: Arc<dyn Handler> = match method {
            SOURCE_READ => Arc::new(read::ReadHandler { root: root.clone() }),
            SOURCE_LIST => Arc::new(list::ListHandler {
                root: root.clone(),
                cache: cache.clone(),
            }),
            SOURCE_SEARCH => Arc::new(search::SearchHandler {
                root: root.clone(),
                cache: cache.clone(),
            }),
            other => {
                return Err(CallError::new(
                    v1::ErrorCode::UnsupportedCapability,
                    format!("unexpected source method {other}"),
                ));
            }
        };
        routes.push(Route {
            protocol: installation.factory.protocol.clone(),
            version: installation.factory.version,
            endpoint: installation.endpoint.clone(),
            method: method.to_string(),
            contract,
            handler,
            target: installation.target.clone(),
            credential: installation.credential.clone(),
        });
    }
    Ok(routes)
}
