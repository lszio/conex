//! Restricted filesystem source provider. Confinement uses cap-std directory
//! capabilities; core gets no fs branch (design J2).
#![forbid(unsafe_code)]

pub mod list;
pub mod read;
pub mod search;

use std::path::Path;
use std::sync::{Arc, Mutex};

use conex_core::{CallError, CallResult, Handler, Installation, Route};
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
                conex_proto::ErrorCode::Internal,
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
    let read_handler = Arc::new(read::ReadHandler { root: root.clone() });
    for (method, contract) in conex_source::contracts() {
        let handler: Arc<dyn Handler> = match method {
            SOURCE_READ => read_handler.clone(),
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
                    conex_proto::ErrorCode::UnsupportedCapability,
                    format!("unexpected source method {other}"),
                ));
            }
        };
        let range_reader = if method == SOURCE_READ {
            Some(Arc::new(read::ReadHandler { root: root.clone() })
                as Arc<dyn conex_core::RangeReader>)
        } else {
            None
        };
        routes.push(Route {
            range_reader,
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
