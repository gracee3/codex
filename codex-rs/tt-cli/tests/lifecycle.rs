use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use codex_utils_cargo_bin::cargo_bin;
use pretty_assertions::assert_eq;

fn run_tt(tt_bin: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(tt_bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|err| panic!("run tt command failed: {err}"))
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
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

fn create_source_repo(root: &Path) -> PathBuf {
    let source_repo = root.join("source-repo");
    fs::create_dir_all(&source_repo).unwrap_or_else(|err| panic!("create source repo dir: {err}"));
    git(&source_repo, &["init", "-q"]);
    fs::write(source_repo.join("README.md"), "# tt workspace test\n")
        .unwrap_or_else(|err| panic!("write readme: {err}"));
    git(&source_repo, &["add", "README.md"]);

    let output = Command::new("git")
        .args([
            "-c",
            "user.name=TT Tests",
            "-c",
            "user.email=tt@example.com",
            "commit",
            "-qm",
            "initial commit",
        ])
        .current_dir(&source_repo)
        .output()
        .unwrap_or_else(|err| panic!("commit source repo: {err}"));
    assert!(
        output.status.success(),
        "git commit failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    source_repo
}

fn setup_workspace(tt_bin: &Path, root: &Path) -> PathBuf {
    let source_repo = create_source_repo(root);
    let clone_output = run_tt(
        tt_bin,
        root,
        &[
            "clone",
            source_repo.to_string_lossy().as_ref(),
            "workspace-under-test",
        ],
    );
    assert!(
        clone_output.status.success(),
        "tt clone failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&clone_output.stdout),
        String::from_utf8_lossy(&clone_output.stderr)
    );

    root.join("workspace-under-test")
}

#[test]
fn start_status_stop_lifecycle_updates_runtime_state() {
    let tempdir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let app_server_bin = cargo_bin("codex-app-server")
        .unwrap_or_else(|err| panic!("resolve codex-app-server binary: {err}"));

    assert!(
        app_server_bin.exists(),
        "codex-app-server binary should exist"
    );

    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    assert!(workspace_root.join(".tt/activate").exists());
    assert!(workspace_root.join(".codex/tt/state.json").exists());
    assert!(primary_checkout.join(".git").exists());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert!(
        start_output.status.success(),
        "tt start failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&start_output.stdout),
        String::from_utf8_lossy(&start_output.stderr)
    );
    let start_stdout = String::from_utf8_lossy(&start_output.stdout);
    assert!(start_stdout.contains("workspace: "));
    assert!(start_stdout.contains("runtime_running: true"));
    assert!(start_stdout.contains("runtime_websocket_url: ws://127.0.0.1:"));
    assert!(start_stdout.contains("supervisor_thread_id: "));
    assert!(start_stdout.contains("director [director]"));
    assert!(start_stdout.contains("developer [developer]"));

    let worker_add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert!(
        worker_add_output.status.success(),
        "tt worker add failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&worker_add_output.stdout),
        String::from_utf8_lossy(&worker_add_output.stderr)
    );
    assert!(workspace_root.join("worktrees/feature-a/.git").exists());

    let worker_list_output = run_tt(&tt_bin, &workspace_root, &["worker", "list"]);
    assert!(
        worker_list_output.status.success(),
        "tt worker list failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&worker_list_output.stdout),
        String::from_utf8_lossy(&worker_list_output.stderr)
    );
    let worker_list_stdout = String::from_utf8_lossy(&worker_list_output.stdout);
    assert!(worker_list_stdout.contains("director\tdirector"));
    assert!(worker_list_stdout.contains("developer\tdeveloper"));
    assert!(worker_list_stdout.contains("feature-a\tworker"));

    let status_output = run_tt(&tt_bin, &workspace_root, &["status"]);
    assert!(
        status_output.status.success(),
        "tt status failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(status_stdout.contains("runtime_running: true"));
    assert!(status_stdout.contains("supervisor_thread_id: "));
    assert!(status_stdout.contains("feature-a [worker]"));

    let state_text = fs::read_to_string(workspace_root.join(".codex/tt/state.json"))
        .unwrap_or_else(|err| panic!("read tt state: {err}"));
    let state_json: serde_json::Value =
        serde_json::from_str(&state_text).unwrap_or_else(|err| panic!("parse tt state: {err}"));
    assert_eq!(state_json["runtime_running"], serde_json::Value::Bool(true));
    assert_eq!(state_json["auto_loop"], serde_json::Value::Bool(false));
    assert!(state_json["supervisor_thread_id"].as_str().is_some());
    assert!(state_json["workers"].as_array().is_some());
    assert!(state_json["runtime_websocket_url"].as_str().is_some());

    let stop_output = run_tt(&tt_bin, &workspace_root, &["stop"]);
    assert!(
        stop_output.status.success(),
        "tt stop failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stop_output.stdout),
        String::from_utf8_lossy(&stop_output.stderr)
    );
    let stop_stdout = String::from_utf8_lossy(&stop_output.stdout);
    assert!(stop_stdout.contains("runtime_running: false"));

    let final_status = run_tt(&tt_bin, &workspace_root, &["status"]);
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
    let tempdir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);

    let output = run_tt(&tt_bin, &workspace_root.join("primary"), &["open"]);
    assert!(
        !output.status.success(),
        "tt open should fail when runtime is not running"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TT runtime is not running; use `tt start` first"));
}
