use std::fs;
use std::path::Path;
use std::process::Command;

use codex_utils_cargo_bin::cargo_bin;
use pretty_assertions::assert_eq;

fn run_tt(tt_bin: &Path, repo_root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(tt_bin)
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run tt command failed: {err}"))
}

fn git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run git command failed: {err}"));
    assert!(
        output.status.success(),
        "git command failed: {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn start_status_stop_lifecycle_updates_runtime_state() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let repo_root = tempdir.path();
    let tt_bin = cargo_bin("tt").expect("resolve tt binary");
    let app_server_bin = cargo_bin("codex-app-server").expect("resolve codex-app-server binary");

    assert!(
        app_server_bin.exists(),
        "codex-app-server binary should exist"
    );

    git(repo_root, &["init", "-q"]);

    let init_output = run_tt(&tt_bin, repo_root, &["init"]);
    assert!(
        init_output.status.success(),
        "tt init failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&init_output.stdout),
        String::from_utf8_lossy(&init_output.stderr)
    );

    let start_output = run_tt(&tt_bin, repo_root, &["start"]);
    assert!(
        start_output.status.success(),
        "tt start failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&start_output.stdout),
        String::from_utf8_lossy(&start_output.stderr)
    );
    let start_stdout = String::from_utf8_lossy(&start_output.stdout);
    assert!(start_stdout.contains("runtime_running: true"));
    assert!(start_stdout.contains("runtime_websocket_url: ws://127.0.0.1:"));

    let status_output = run_tt(&tt_bin, repo_root, &["status"]);
    assert!(
        status_output.status.success(),
        "tt status failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(status_stdout.contains("runtime_running: true"));
    assert!(status_stdout.contains("director_thread_id: "));
    assert!(status_stdout.contains("developer_thread_id: "));

    let state_text = fs::read_to_string(repo_root.join(".tt/state.json")).expect("read tt state");
    let state_json: serde_json::Value = serde_json::from_str(&state_text).expect("parse tt state");
    assert_eq!(state_json["runtime_running"], serde_json::Value::Bool(true));
    assert_eq!(state_json["auto_loop"], serde_json::Value::Bool(false));
    assert!(state_json["director_thread_id"].as_str().is_some());
    assert!(state_json["developer_thread_id"].as_str().is_some());
    assert!(state_json["runtime_websocket_url"].as_str().is_some());

    let stop_output = run_tt(&tt_bin, repo_root, &["stop"]);
    assert!(
        stop_output.status.success(),
        "tt stop failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stop_output.stdout),
        String::from_utf8_lossy(&stop_output.stderr)
    );
    let stop_stdout = String::from_utf8_lossy(&stop_output.stdout);
    assert!(stop_stdout.contains("runtime_running: false"));

    let final_status = run_tt(&tt_bin, repo_root, &["status"]);
    assert!(
        final_status.status.success(),
        "final tt status failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&final_status.stdout),
        String::from_utf8_lossy(&final_status.stderr)
    );
    let final_stdout = String::from_utf8_lossy(&final_status.stdout);
    assert!(final_stdout.contains("runtime_running: false"));
    assert!(final_stdout.contains("runtime_websocket_url: <none>"));
}

#[test]
fn open_requires_running_runtime() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let repo_root = tempdir.path();
    let tt_bin = cargo_bin("tt").expect("resolve tt binary");

    git(repo_root, &["init", "-q"]);

    let output = run_tt(&tt_bin, repo_root, &["open"]);
    assert!(
        !output.status.success(),
        "tt open should fail when runtime is not running"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TT runtime is not running; use `tt start` first"));
}
