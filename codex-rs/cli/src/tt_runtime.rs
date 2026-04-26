use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_arg0::Arg0DispatchPaths;
use codex_tt_core::RuntimeRegistration;
use codex_tt_core::TtProject;
use codex_tt_core::read_runtime_registration;
use codex_tt_core::write_runtime_registration;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::time::Instant;

const TT_APP_SERVER_LISTEN_URL: &str = "ws://127.0.0.1:0";
const TT_APP_SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct TtAppServerConnection {
    pub(crate) websocket_url: String,
}

struct SpawnedTtAppServer {
    websocket_url: String,
    pid: u32,
}

pub(crate) async fn maybe_resolve_tt_app_server(
    cwd: &Path,
    arg0_paths: &Arg0DispatchPaths,
) -> io::Result<Option<TtAppServerConnection>> {
    let Some(project) = TtProject::discover_from(cwd).map_err(io::Error::other)? else {
        return Ok(None);
    };

    if let Some(registration) = read_runtime_registration(&project).map_err(io::Error::other)?
        && can_connect(&registration.endpoint).await
    {
        return Ok(Some(TtAppServerConnection {
            websocket_url: registration.endpoint,
        }));
    }

    let spawned = spawn_tt_app_server(arg0_paths, &project).await?;
    let registration = RuntimeRegistration::new(spawned.pid, spawned.websocket_url.clone());
    write_runtime_registration(&project, &registration).map_err(io::Error::other)?;
    Ok(Some(TtAppServerConnection {
        websocket_url: spawned.websocket_url,
    }))
}

async fn can_connect(websocket_url: &str) -> bool {
    match RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        websocket_url: websocket_url.to_string(),
        auth_token: None,
        client_name: "codex-tt-probe".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: 1,
    })
    .await
    {
        Ok(client) => client.shutdown().await.is_ok(),
        Err(_) => false,
    }
}

async fn spawn_tt_app_server(
    arg0_paths: &Arg0DispatchPaths,
    project: &TtProject,
) -> io::Result<SpawnedTtAppServer> {
    let program = resolve_codex_app_server_binary(arg0_paths);
    let mut command = Command::new(program);
    command
        .arg("app-server")
        .arg("--listen")
        .arg(TT_APP_SERVER_LISTEN_URL)
        .current_dir(project.primary_repo())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("CODEX_HOME", project.codex_home());

    let mut child = command.spawn()?;
    let pid = child.id().unwrap_or(0);
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("capture TT app-server stderr"))?;
    let mut stderr_reader = BufReader::new(stderr).lines();
    let deadline = Instant::now() + TT_APP_SERVER_START_TIMEOUT;
    let websocket_url = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = tokio::time::timeout(remaining, stderr_reader.next_line())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for TT app-server websocket address",
                )
            })?
            .map_err(|err| io::Error::other(format!("read TT app-server stderr: {err}")))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "TT app-server exited before announcing websocket address",
                )
            })?;
        let stripped = strip_ansi(&line);
        if let Some(websocket_url) = stripped
            .split_whitespace()
            .find(|token| token.starts_with("ws://"))
        {
            break websocket_url.to_string();
        }
    };

    tokio::spawn(async move {
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            tracing::info!(target: "codex_cli::tt_runtime", "{line}");
        }
        if let Err(err) = child.wait().await {
            tracing::warn!(target: "codex_cli::tt_runtime", "TT app-server wait failed: {err}");
        }
    });

    Ok(SpawnedTtAppServer { websocket_url, pid })
}

fn resolve_codex_app_server_binary(arg0_paths: &Arg0DispatchPaths) -> PathBuf {
    if let Some(current_exe) = arg0_paths.codex_self_exe.as_ref() {
        return current_exe.to_path_buf();
    }
    PathBuf::from("codex")
}

fn strip_ansi(line: &str) -> String {
    let mut stripped = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some(&'[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        stripped.push(ch);
    }
    stripped
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;
    use pretty_assertions::assert_eq;

    #[test]
    fn strip_ansi_removes_color_sequences() {
        assert_eq!(
            strip_ansi("\u{1b}[32mws://127.0.0.1:1\u{1b}[0m"),
            "ws://127.0.0.1:1"
        );
    }
}
