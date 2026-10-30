//! Same domain dispatcher as publication, behind a 0600 Unix socket and OS peer
//! credentials. Loopback is not authentication; no principal is accepted on wire.
use crate::access::TrustedCaller;
use crate::node::{NodeHandle, NodeResult, failure};
use crate::process::read_frame;
use conex_proto::{self as p, message};
use serde::Deserialize;
use serde_json::{Value, json};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InvokeInput {
    instance_id: String,
    module: String,
    generation: String,
    method: String,
    argument: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StopInput {
    instance_id: String,
    deadline_ms: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StartInput {
    app_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

pub struct LocalApi {
    listener: UnixListener,
    path: PathBuf,
    uid: u32,
    caller: TrustedCaller,
}

impl LocalApi {
    pub async fn bind(path: PathBuf, principal: String, node: String) -> Result<Self, String> {
        let parent = path.parent().ok_or("local socket has no parent")?;
        crate::process::ensure_private_dir(parent)?;
        let uid = std::fs::metadata(parent)
            .map_err(|error| error.to_string())?
            .uid();
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            use std::os::unix::fs::FileTypeExt;
            if !metadata.file_type().is_socket() || metadata.uid() != uid {
                return Err("local socket path occupied by an untrusted file".into());
            }
            if UnixStream::connect(&path).await.is_ok() {
                return Err("local API already active".into());
            }
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            listener,
            path,
            uid,
            caller: TrustedCaller::local_operator(principal, node),
        })
    }

    pub async fn serve(
        self,
        node: NodeHandle,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let permits = Arc::new(Semaphore::new(64));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } }
                finished = tasks.join_next(), if !tasks.is_empty() => { let _ = finished; }
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(|error| error.to_string())?;
                    if stream.peer_cred().map_err(|error| error.to_string())?.uid() != self.uid { continue; }
                    let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                    let caller = self.caller.clone();
                    let node = node.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let _ = tokio::time::timeout(Duration::from_secs(35), serve_one(stream, node, caller)).await;
                    });
                }
            }
        }
        // A shutdown request's reply must reach its caller before closing the API.
        while tasks.join_next().await.is_some() {}
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }
}

async fn serve_one(
    stream: UnixStream,
    node: NodeHandle,
    caller: TrustedCaller,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream);
    let bytes = read_frame(&mut reader, 256 * 1024).await?;
    // Reuse the S1 strict JSON visitor: duplicates at any depth, unknown envelope
    // keys, malformed ULIDs and forged S1 caller fields fail before dispatch.
    let decoded = p::wire::decode_wire_for_boundary(&bytes, p::capability::Boundary::Client);
    let (id, result) = match decoded {
        Ok(p::Message {
            body: Some(message::Body::Request(request)),
        }) => {
            let id = request.request_id.clone();
            let result = dispatch(&node, &caller, request).await;
            (Some(id), result)
        }
        Ok(_) => (None, Err(failure("bad_request"))),
        Err(_) => (None, Err(failure("bad_request"))),
    };
    let reply = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => json!({"jsonrpc":"2.0","id":id,"error":{
            "code":error.0.code,"message":error.0.message,
            "execution":error.0.execution,"retry":error.0.retry,
        }}),
    };
    let mut bytes = serde_json::to_vec(&reply).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    reader
        .get_mut()
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())
}

async fn dispatch(node: &NodeHandle, caller: &TrustedCaller, request: p::Request) -> NodeResult {
    let params = request.params.ok_or_else(|| failure("bad_request"))?;
    let context = params.context.ok_or_else(|| failure("bad_request"))?;
    if context.plane != p::Plane::Broker as i32
        || !context.principal_id.is_empty()
        || !context.tenant_id.is_empty()
        || !context.space_id.is_empty()
        || !context.provider_endpoint_id.is_empty()
        || !context.registration_id.is_empty()
        || context.binding_id.is_some()
    {
        return Err(failure("forbidden"));
    }
    let input = params
        .input
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| failure("bad_request"))?
        .unwrap_or_else(|| json!({}));
    match request.method.as_str() {
        "node/snapshot" => {
            let _: Empty = serde_json::from_value(input).map_err(|_| failure("bad_request"))?;
            node.snapshot().await
        }
        "node/start" => {
            let input: StartInput =
                serde_json::from_value(input).map_err(|_| failure("bad_request"))?;
            node.start_package(input.app_id).await
        }
        "node/stop" => {
            let input: StopInput =
                serde_json::from_value(input).map_err(|_| failure("bad_request"))?;
            node.stop_instance(
                input.instance_id,
                Duration::from_millis(input.deadline_ms.unwrap_or(30_000).min(30_000)),
            )
            .await
        }
        "node/invoke" => {
            let input: InvokeInput =
                serde_json::from_value(input).map_err(|_| failure("bad_request"))?;
            let generation = input
                .generation
                .parse::<u64>()
                .map_err(|_| failure("bad_request"))?;
            if generation == 0 || generation.to_string() != input.generation {
                return Err(failure("bad_request"));
            }
            node.invoke(
                caller.clone(),
                input.instance_id,
                input.module,
                generation,
                input.method,
                input.argument,
            )
            .await
        }
        "node/shutdown" => {
            let _: Empty = serde_json::from_value(input).map_err(|_| failure("bad_request"))?;
            node.stop_all(Duration::from_secs(30)).await
        }
        _ => Err(failure("unsupported_capability")),
    }
}
