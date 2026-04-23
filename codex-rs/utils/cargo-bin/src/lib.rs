use std::io;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CargoBinError {
    #[error("failed to read current directory")]
    CurrentDir {
        #[source]
        source: std::io::Error,
    },
    #[error("CARGO_BIN_EXE env var {key} resolved to {path:?}, but it does not exist")]
    ResolvedPathDoesNotExist { key: String, path: PathBuf },
    #[error("could not locate binary {name:?}; tried env vars {env_keys:?}; {fallback}")]
    NotFound {
        name: String,
        env_keys: Vec<String>,
        fallback: String,
    },
}

/// Returns an absolute path to a binary target built for the current Cargo test run.
#[allow(deprecated)]
pub fn cargo_bin(name: &str) -> Result<PathBuf, CargoBinError> {
    let env_keys = cargo_bin_env_keys(name);
    for key in &env_keys {
        if let Some(value) = std::env::var_os(key) {
            return resolve_bin_from_env(key, PathBuf::from(value));
        }
    }
    match assert_cmd::Command::cargo_bin(name) {
        Ok(cmd) => {
            let path = PathBuf::from(cmd.get_program());
            resolve_bin_from_env("assert_cmd::Command::cargo_bin", path)
        }
        Err(err) => Err(CargoBinError::NotFound {
            name: name.to_owned(),
            env_keys,
            fallback: format!("assert_cmd fallback failed: {err}"),
        }),
    }
}

fn cargo_bin_env_keys(name: &str) -> Vec<String> {
    let mut keys = Vec::with_capacity(2);
    keys.push(format!("CARGO_BIN_EXE_{name}"));

    let underscore_name = name.replace('-', "_");
    if underscore_name != name {
        keys.push(format!("CARGO_BIN_EXE_{underscore_name}"));
    }

    keys
}

fn resolve_bin_from_env(key: &str, mut path: PathBuf) -> Result<PathBuf, CargoBinError> {
    if !path.is_absolute() {
        path = std::env::current_dir()
            .map_err(|source| CargoBinError::CurrentDir { source })?
            .join(path);
    }

    if path.exists() {
        Ok(path)
    } else {
        Err(CargoBinError::ResolvedPathDoesNotExist {
            key: key.to_owned(),
            path,
        })
    }
}

/// Macro that derives the path to a test resource at runtime.
///
/// The return value may be relative or absolute depending on the call site.
#[macro_export]
macro_rules! find_resource {
    ($resource:expr) => {{
        let resource = std::path::Path::new(&$resource);
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::io::Result::<std::path::PathBuf>::Ok(manifest_dir.join(resource))
    }};
}

pub fn resolve_cargo_resource(resource: &Path) -> std::io::Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest_dir.join(resource))
}

pub fn repo_root() -> io::Result<PathBuf> {
    let marker = resolve_cargo_resource(Path::new("repo_root.marker"))?;
    let mut root = marker;
    for _ in 0..4 {
        root = root
            .parent()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "repo_root.marker did not have expected parent depth",
                )
            })?
            .to_path_buf();
    }
    Ok(root)
}
