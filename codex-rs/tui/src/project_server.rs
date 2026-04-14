use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_arg0::Arg0DispatchPaths;
use codex_config::config_toml::ConfigToml;
use codex_config::default_project_root_markers;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::time::Instant;

const PROJECT_SERVER_RUNTIME_FILENAME: &str = "project-server.json";
const PROJECT_SERVER_LISTEN_URL: &str = "ws://127.0.0.1:0";
const PROJECT_SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectServerConnection {
    pub(crate) websocket_url: String,
    pub(crate) project_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProjectServerRuntimeMetadata {
    version: u32,
    websocket_url: String,
    project_root: PathBuf,
    pid: Option<u32>,
}

pub(crate) async fn maybe_resolve(
    config_toml: &ConfigToml,
    cwd: Option<&AbsolutePathBuf>,
    arg0_paths: &Arg0DispatchPaths,
    codex_home: &Path,
) -> io::Result<Option<ProjectServerConnection>> {
    let Some(cwd) = cwd else {
        return Ok(None);
    };
    let Some(project_server) = config_toml.project_server.as_ref() else {
        return Ok(None);
    };
    if !project_server.enabled.unwrap_or(false) {
        return Ok(None);
    }

    let Some(project_root) = detect_project_root(cwd.as_path(), config_toml) else {
        return Ok(None);
    };
    let runtime_path = project_root
        .join(".codex")
        .join(PROJECT_SERVER_RUNTIME_FILENAME);

    if let Some(metadata) = read_runtime_metadata(&runtime_path)?
        && metadata.project_root == project_root
        && can_connect(&metadata.websocket_url).await
    {
        return Ok(Some(ProjectServerConnection {
            websocket_url: metadata.websocket_url,
            project_root,
        }));
    }

    if !project_server.auto_start.unwrap_or(true) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "project server is enabled for {} but no running app-server was discovered",
                project_root.display()
            ),
        ));
    }

    let metadata = spawn_project_server(arg0_paths, codex_home, &project_root).await?;
    write_runtime_metadata(&runtime_path, &metadata)?;
    Ok(Some(ProjectServerConnection {
        websocket_url: metadata.websocket_url,
        project_root,
    }))
}

fn detect_project_root(cwd: &Path, config_toml: &ConfigToml) -> Option<PathBuf> {
    let markers = config_toml
        .project_root_markers
        .clone()
        .unwrap_or_else(default_project_root_markers);
    if markers.is_empty() {
        return None;
    }

    cwd.ancestors().find_map(|ancestor| {
        markers
            .iter()
            .any(|marker| ancestor.join(marker).exists())
            .then(|| ancestor.to_path_buf())
    })
}

fn read_runtime_metadata(path: &Path) -> io::Result<Option<ProjectServerRuntimeMetadata>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    match serde_json::from_str::<ProjectServerRuntimeMetadata>(&contents) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) => {
            tracing::warn!(
                error = %err,
                path = %path.display(),
                "ignoring invalid project-server metadata"
            );
            Ok(None)
        }
    }
}

fn write_runtime_metadata(path: &Path, metadata: &ProjectServerRuntimeMetadata) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("runtime metadata path has no parent: {}", path.display()),
        ));
    };
    std::fs::create_dir_all(parent)?;
    let serialized = serde_json::to_string_pretty(metadata)
        .map_err(|err| io::Error::other(format!("serialize project-server metadata: {err}")))?;
    std::fs::write(path, format!("{serialized}\n"))
}

async fn can_connect(websocket_url: &str) -> bool {
    match RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        websocket_url: websocket_url.to_string(),
        auth_token: None,
        client_name: "codex-tui-project-server-probe".to_string(),
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

async fn spawn_project_server(
    arg0_paths: &Arg0DispatchPaths,
    codex_home: &Path,
    project_root: &Path,
) -> io::Result<ProjectServerRuntimeMetadata> {
    let program = resolve_codex_app_server_binary(arg0_paths);
    let mut command = Command::new(program);
    command
        .arg("--listen")
        .arg(PROJECT_SERVER_LISTEN_URL)
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("CODEX_HOME", codex_home);
    let mut child = command.spawn()?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("capture project-server stderr"))?;
    let mut stderr_reader = BufReader::new(stderr).lines();
    let deadline = Instant::now() + PROJECT_SERVER_START_TIMEOUT;
    let websocket_url = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = tokio::time::timeout(remaining, stderr_reader.next_line())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for project-server websocket address",
                )
            })?
            .map_err(|err| io::Error::other(format!("read project-server stderr: {err}")))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "project-server exited before announcing websocket address",
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
    let pid = child.id();
    tokio::spawn(async move {
        let mut stderr_reader = stderr_reader;
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            tracing::info!(target: "codex_tui::project_server", "{line}");
        }
    });

    Ok(ProjectServerRuntimeMetadata {
        version: 1,
        websocket_url,
        project_root: project_root.to_path_buf(),
        pid,
    })
}

fn resolve_codex_app_server_binary(arg0_paths: &Arg0DispatchPaths) -> PathBuf {
    if let Some(current_exe) = arg0_paths.codex_self_exe.as_ref() {
        let file_name =
            if let Some(extension) = current_exe.extension().and_then(|ext| ext.to_str()) {
                format!("codex-app-server.{extension}")
            } else {
                "codex-app-server".to_string()
            };
        let sibling = current_exe.with_file_name(file_name);
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("codex-app-server")
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
    use super::detect_project_root;
    use codex_config::config_toml::ConfigToml;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn detect_project_root_uses_configured_marker() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        let nested = repo.join("src");
        std::fs::create_dir_all(&nested).expect("create nested");
        std::fs::write(repo.join(".project-root"), "").expect("write marker");
        let cwd = AbsolutePathBuf::try_from(nested).expect("absolute path");
        let mut config_toml: ConfigToml = toml::from_str("").expect("empty config");
        config_toml.project_root_markers = Some(vec![".project-root".to_string()]);

        let detected = detect_project_root(cwd.as_path(), &config_toml);

        assert_eq!(detected, Some(repo));
    }
}
