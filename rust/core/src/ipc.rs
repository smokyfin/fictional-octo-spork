//! Unix-Socket IPC server between the platform UI and the VPN core.
//!
//! - Bound inside the application's *private* directory (the `private_dir`
//!   handed in by the platform layer); *not* world-readable.
//! - Accepts one auth token per session, generated freshly at startup and
//!   communicated back to the UI via the platform channel.
//! - Each client message is one JSON object per line; replies are likewise
//!   single-line JSON.

use crate::engine::Status;
use crate::error::{Error, Result};
use crate::runtime::Cancel;
use crate::util::{ct_eq, random_password};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Authenticate the connection. Must be the first message on every link.
    Auth { token: String },

    /// Snapshot of the current engine status.
    Status,

    /// Subscribe to the live log stream (server pushes log lines).
    SubscribeLogs,

    /// Stop the engine.
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Err { message: String },
    Status(Status),
    Log { line: String },
}

/// Per-process auth token, regenerated on every engine start. Exposed to the
/// UI through the platform channel; not stored on disk.
static AUTH_TOKEN: once_cell::sync::OnceCell<parking_lot::RwLock<String>> =
    once_cell::sync::OnceCell::new();

pub fn current_auth_token() -> String {
    AUTH_TOKEN
        .get_or_init(|| parking_lot::RwLock::new(random_password(32)))
        .read()
        .clone()
}

pub fn rotate_auth_token() -> String {
    let cell = AUTH_TOKEN.get_or_init(|| parking_lot::RwLock::new(random_password(32)));
    let new_token = random_password(32);
    *cell.write() = new_token.clone();
    new_token
}

#[cfg(unix)]
pub async fn spawn_ipc_server(
    private_dir: PathBuf,
    status: Arc<Mutex<Status>>,
    cancel: Cancel,
) -> Result<JoinHandle<()>> {
    let socket_path = private_dir.join("ipc.sock");
    // Remove any stale socket — but only if it lives in our private dir.
    if socket_path.exists() {
        if let Err(e) = std::fs::remove_file(&socket_path) {
            warn!(?e, path = %socket_path.display(), "could not remove stale ipc socket");
        }
    }
    let listener =
        UnixListener::bind(&socket_path).map_err(|e| Error::Ipc(format!("bind: {e}")))?;
    // Lock down permissions to user-only.
    set_owner_only(&socket_path)?;
    rotate_auth_token();
    debug!(path = %socket_path.display(), "ipc listener up");

    Ok(tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    debug!("ipc cancelled");
                    let _ = std::fs::remove_file(&socket_path);
                    return;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _)) => {
                            let status = status.clone();
                            let cancel = cancel.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, status, cancel).await {
                                    debug!(?e, "ipc client closed with error");
                                }
                            });
                        }
                        Err(e) => warn!(?e, "ipc accept error"),
                    }
                }
            }
        }
    }))
}

#[cfg(not(unix))]
pub async fn spawn_ipc_server(
    _private_dir: PathBuf,
    _status: Arc<Mutex<Status>>,
    _cancel: Cancel,
) -> Result<JoinHandle<()>> {
    // Windows/desktop dev fallback: no-op IPC.
    Ok(tokio::spawn(async {}))
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(Error::from)
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn handle_client(
    stream: UnixStream,
    status: Arc<Mutex<Status>>,
    cancel: Cancel,
) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut authed = false;
    let expected = current_auth_token();

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| Error::Ipc(e.to_string()))?;
        if n == 0 {
            return Ok(()); // peer closed
        }
        let req: Request = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                let _ = send(
                    &mut write,
                    &Reply::Err {
                        message: e.to_string(),
                    },
                )
                .await;
                continue;
            }
        };

        if !authed {
            match req {
                Request::Auth { token } => {
                    if ct_eq(token.as_bytes(), expected.as_bytes()) {
                        authed = true;
                        send(&mut write, &Reply::Ok).await?;
                    } else {
                        send(
                            &mut write,
                            &Reply::Err {
                                message: "invalid auth token".into(),
                            },
                        )
                        .await?;
                        return Ok(());
                    }
                }
                _ => {
                    send(
                        &mut write,
                        &Reply::Err {
                            message: "auth required".into(),
                        },
                    )
                    .await?;
                    return Ok(());
                }
            }
            continue;
        }

        match req {
            Request::Auth { .. } => {
                send(
                    &mut write,
                    &Reply::Err {
                        message: "already authenticated".into(),
                    },
                )
                .await?;
            }
            Request::Status => {
                let snapshot = status.lock().clone();
                send(&mut write, &Reply::Status(snapshot)).await?;
            }
            Request::SubscribeLogs => {
                // Real implementation tails an in-memory ring buffer; for the
                // scaffold we send a single hello and keep the connection
                // alive until cancellation.
                send(
                    &mut write,
                    &Reply::Log {
                        line: "log stream up".into(),
                    },
                )
                .await?;
                cancel.cancelled().await;
                return Ok(());
            }
            Request::Stop => {
                send(&mut write, &Reply::Ok).await?;
                cancel.cancel();
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
async fn send<W: AsyncWriteExt + Unpin>(w: &mut W, reply: &Reply) -> Result<()> {
    let mut bytes = serde_json::to_vec(reply)?;
    bytes.push(b'\n');
    w.write_all(&bytes)
        .await
        .map_err(|e| Error::Ipc(e.to_string()))?;
    w.flush().await.map_err(|e| Error::Ipc(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_tokens_rotate() {
        let a = rotate_auth_token();
        let b = rotate_auth_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }
}
