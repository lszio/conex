//! Fixed built-in counter worker. Application packages cannot select an executable,
//! argv, cwd or environment. Control identities stay in the restricted node store.
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlIdentity {
    pub socket: PathBuf,
    pub token: String,
    pub pid: u32,
    pub start_ticks: String,
    pub boot_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerConfig {
    socket: PathBuf,
    token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    token: String,
    method: String,
    #[serde(default)]
    delta: i32,
}

pub struct OwnedProcess {
    pub identity: ControlIdentity,
    child: Child,
}

pub struct SpawnFailure {
    pub message: String,
    pub confirmed_exit: bool,
}

pub fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("local entropy: {error}"))?;
    Ok(hex::encode(bytes))
}

pub fn process_identity(pid: u32) -> Result<(String, String), String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| format!("process identity: {error}"))?;
    // comm may contain spaces and ')'; starttime is field 22 after the last ')'.
    let tail = stat.rsplit_once(')').ok_or("invalid proc stat")?.1;
    let ticks = tail.split_whitespace().nth(19).ok_or("missing starttime")?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| error.to_string())?;
    Ok((ticks.to_owned(), boot.trim().to_owned()))
}

/// Inspection failure is unknown, never evidence of exit.
pub fn known_exited(identity: &ControlIdentity) -> bool {
    if identity.pid == 0 {
        return false;
    }
    let Ok(boot) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") else {
        return false;
    };
    if boot.trim() != identity.boot_id {
        return true;
    }
    match std::fs::read_to_string(format!("/proc/{}/stat", identity.pid)) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(stat) => {
            let Some((_, tail)) = stat.rsplit_once(')') else {
                return false;
            };
            let fields: Vec<_> = tail.split_whitespace().collect();
            fields
                .get(19)
                .is_some_and(|ticks| *ticks != identity.start_ticks)
                || fields
                    .first()
                    .is_some_and(|state| matches!(*state, "Z" | "X"))
        }
    }
}

impl OwnedProcess {
    /// Persist the preallocated identity before calling this; any uncertain spawn
    /// remains an unreconciled constraint instead of permitting a conflicting start.
    pub async fn spawn(socket: PathBuf, token: String) -> Result<Self, SpawnFailure> {
        let exe = std::env::current_exe().map_err(|error| SpawnFailure {
            message: error.to_string(),
            confirmed_exit: true,
        })?;
        let mut child = Command::new(exe)
            .arg("--module-worker")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| SpawnFailure {
                message: format!("spawn built-in worker: {error}"),
                confirmed_exit: true,
            })?;
        let result: Result<ControlIdentity, String> = async {
            let pid = child.id().ok_or("worker exited during creation")?;
            let (start_ticks, boot_id) = process_identity(pid)?;
            let identity = ControlIdentity {
                socket: socket.clone(),
                token: token.clone(),
                pid,
                start_ticks,
                boot_id,
            };
            let input = serde_json::to_vec(&WorkerConfig { socket, token })
                .map_err(|error| error.to_string())?;
            let mut stdin = child.stdin.take().ok_or("missing worker stdin")?;
            stdin
                .write_all(&input)
                .await
                .map_err(|error| error.to_string())?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|error| error.to_string())?;
            drop(stdin);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(reply) = request(&identity, "inspect", 0).await {
                    if reply.get("pid").and_then(Value::as_u64) == Some(u64::from(pid)) {
                        return Ok(identity);
                    }
                    return Err("worker identity mismatch".into());
                }
                if child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    return Err("worker failed before readiness".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("worker readiness deadline".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        .await;
        match result {
            Ok(identity) => Ok(Self { identity, child }),
            Err(error) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), child.kill()).await;
                let confirmed_exit = child.try_wait().is_ok_and(|status| status.is_some());
                Err(SpawnFailure {
                    message: error,
                    confirmed_exit,
                })
            }
        }
    }

    pub async fn stop(&mut self, deadline: tokio::time::Instant) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Ok(());
        }
        let _ = tokio::time::timeout_at(deadline, request(&self.identity, "stop", 0)).await;
        if tokio::time::timeout_at(deadline, self.child.wait())
            .await
            .is_err()
        {
            // A live Child handle owns this unreaped process, not an arbitrary PID.
            self.child.kill().await.map_err(|error| error.to_string())?;
            self.child.wait().await.map_err(|error| error.to_string())?;
        }
        let _ = std::fs::remove_file(&self.identity.socket);
        Ok(())
    }
}

pub async fn request(
    identity: &ControlIdentity,
    method: &str,
    delta: i32,
) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = UnixStream::connect(&identity.socket)
            .await
            .map_err(|error| error.to_string())?;
        let input = serde_json::to_vec(&ControlRequest {
            token: identity.token.clone(),
            method: method.into(),
            delta,
        })
        .map_err(|error| error.to_string())?;
        stream
            .write_all(&input)
            .await
            .map_err(|error| error.to_string())?;
        stream
            .write_all(b"\n")
            .await
            .map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(stream);
        let bytes = read_frame(&mut reader, 4096).await?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if value.get("error").is_some() {
            return Err("worker rejected request".into());
        }
        Ok(value)
    })
    .await
    .map_err(|_| "worker request deadline".to_owned())?
}

pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if available.is_empty() {
            return Err("incomplete frame".into());
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(available.len(), |index| index + 1);
        if output.len() + take > limit {
            return Err("payload_too_large".into());
        }
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if end.is_some() {
            return Ok(output);
        }
    }
}

/// Invoked only through the fixed internal binary mode; configuration is delivered
/// over inherited stdin, never command-line secrets or package-controlled commands.
pub async fn worker() -> Result<(), String> {
    let mut input = BufReader::new(tokio::io::stdin());
    let config: WorkerConfig = serde_json::from_slice(&read_frame(&mut input, 4096).await?)
        .map_err(|error| error.to_string())?;
    let listener = UnixListener::bind(&config.socket).map_err(|error| error.to_string())?;
    std::fs::set_permissions(&config.socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let start = Instant::now();
    let mut adjustment = 0i32;
    let mut mutations = 0u64;
    loop {
        let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let mut stream = BufReader::new(stream);
        let parsed =
            tokio::time::timeout(Duration::from_secs(2), read_frame(&mut stream, 4096)).await;
        let Ok(Ok(bytes)) = parsed else {
            continue;
        };
        let Ok(request) = serde_json::from_slice::<ControlRequest>(&bytes) else {
            continue;
        };
        if request.token != config.token {
            continue;
        }
        let ticks = start.elapsed().as_millis() / 100;
        let count = i32::try_from(ticks)
            .ok()
            .and_then(|tick| tick.checked_add(adjustment));
        let mut stop = false;
        let response = match request.method.as_str() {
            "inspect" => {
                json!({"count":count,"revision":(ticks + u128::from(mutations)).to_string(),"pid":std::process::id()})
            }
            "add" => {
                if let Some(next) = adjustment.checked_add(request.delta).filter(|_| {
                    count.is_some_and(|value| value.checked_add(request.delta).is_some())
                }) {
                    adjustment = next;
                    mutations += 1;
                    json!({"count":count.unwrap() + request.delta,"revision":(ticks + u128::from(mutations)).to_string(),"pid":std::process::id()})
                } else {
                    json!({"error":"overflow"})
                }
            }
            "stop" => {
                stop = true;
                json!({"stopped":true})
            }
            _ => json!({"error":"unsupported"}),
        };
        let mut bytes = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        let _ = stream.get_mut().write_all(&bytes).await;
        if stop {
            break;
        }
    }
    drop(listener);
    let _ = std::fs::remove_file(&config.socket);
    Ok(())
}

/// Reconciliation never kills a PID. Only the authenticated original controller
/// can request shutdown; inability to prove exit leaves the resource unknown.
pub async fn reconcile(identity: &ControlIdentity) -> bool {
    // A crash can occur between reserving the control channel and persisting
    // readiness. Recover that gap only through the secret authenticated channel.
    let mut recovered = identity.clone();
    if recovered.pid == 0 {
        let Ok(reply) = request(&recovered, "inspect", 0).await else {
            return false;
        };
        let Some(pid) = reply
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            return false;
        };
        let Ok((ticks, boot)) = process_identity(pid) else {
            return false;
        };
        recovered.pid = pid;
        recovered.start_ticks = ticks;
        recovered.boot_id = boot;
    }
    let identity = &recovered;
    if known_exited(identity) {
        return true;
    }
    if request(identity, "stop", 0).await.is_err() {
        return false;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !known_exited(identity) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    known_exited(identity)
}

pub fn ensure_private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o077 != 0 {
        return Err("runtime directory must be a private non-symlink directory (0700)".into());
    }
    Ok(())
}
