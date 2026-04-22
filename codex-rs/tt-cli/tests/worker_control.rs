use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::create_shell_command_sse_response;
use app_test_support::write_mock_responses_config_toml_with_chatgpt_base_url;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ReviewDelivery;
use codex_app_server_protocol::ReviewStartParams;
use codex_app_server_protocol::ReviewStartResponse;
use codex_app_server_protocol::ReviewTarget;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_tt_core::WorkerBinding;
use codex_tt_core::WorkspacePaths;
use codex_tt_core::load_state;
use codex_tt_core::save_state;
use codex_utils_cargo_bin::cargo_bin;
use pretty_assertions::assert_eq;
use tempfile::tempdir;
use tokio::time::Instant;
use tokio::time::sleep;

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

fn write_mock_config(workspace_root: &Path, server_uri: &str) {
    write_mock_responses_config_toml_with_chatgpt_base_url(
        &workspace_root.join(".codex"),
        server_uri,
        server_uri,
    )
    .unwrap_or_else(|err| panic!("write mock config: {err}"));
}

fn load_tt_state(workspace_root: &Path) -> codex_tt_core::TtState {
    let paths = WorkspacePaths::new(workspace_root.to_path_buf());
    load_state(&paths).unwrap_or_else(|err| panic!("load tt state: {err}"))
}

fn save_tt_state(workspace_root: &Path, state: &codex_tt_core::TtState) {
    let paths = WorkspacePaths::new(workspace_root.to_path_buf());
    save_state(&paths, state).unwrap_or_else(|err| panic!("save tt state: {err}"));
}

async fn connect_runtime(workspace_root: &Path) -> RemoteAppServerClient {
    let state = load_tt_state(workspace_root);
    RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        websocket_url: state
            .runtime_websocket_url
            .clone()
            .unwrap_or_else(|| panic!("runtime websocket url")),
        auth_token: state.runtime_auth_token.clone(),
        client_name: "tt-worker-control-tests".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    .unwrap_or_else(|err| panic!("connect runtime client: {err}"))
}

fn request_id(next: &mut i64) -> RequestId {
    let value = *next;
    *next += 1;
    RequestId::Integer(value)
}

async fn thread_read(
    client: &RemoteAppServerClient,
    next_request_id: &mut i64,
    thread_id: &str,
) -> ThreadReadResponse {
    match client
        .request_typed(ClientRequest::ThreadRead {
            request_id: request_id(next_request_id),
            params: ThreadReadParams {
                thread_id: thread_id.to_string(),
                include_turns: true,
            },
        })
        .await
    {
        Ok(response) => response,
        Err(err)
            if err
                .to_string()
                .contains("includeTurns is unavailable before first user message") =>
        {
            client
                .request_typed(ClientRequest::ThreadRead {
                    request_id: request_id(next_request_id),
                    params: ThreadReadParams {
                        thread_id: thread_id.to_string(),
                        include_turns: false,
                    },
                })
                .await
                .unwrap_or_else(|err| panic!("thread/read without turns: {err}"))
        }
        Err(err) => panic!("thread/read: {err:?}"),
    }
}

async fn wait_for_thread_idle(
    client: &RemoteAppServerClient,
    next_request_id: &mut i64,
    thread_id: &str,
) -> ThreadReadResponse {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = thread_read(client, next_request_id, thread_id).await;
        if response
            .thread
            .turns
            .iter()
            .all(|turn| turn.status != TurnStatus::InProgress)
        {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for thread {thread_id} to idle"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

fn worker_thread_id(workspace_root: &Path, name: &str) -> String {
    load_tt_state(workspace_root)
        .workers
        .into_iter()
        .find(|worker| worker.name == name)
        .and_then(|worker| worker.thread_id)
        .unwrap_or_else(|| panic!("missing thread id for worker `{name}`"))
}

fn worker_binding(workspace_root: &Path, name: &str) -> WorkerBinding {
    load_tt_state(workspace_root)
        .workers
        .into_iter()
        .find(|worker| worker.name == name)
        .map(|worker| worker.binding)
        .unwrap_or_else(|| panic!("missing binding for worker `{name}`"))
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure_contains(output: &std::process::Output, needle: &str) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "expected stderr to contain `{needle}`\nstderr:\n{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_read_send_and_remove_cover_history_windows_and_summaries() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let responses = vec![
        create_final_assistant_message_sse_response("director seeded")
            .unwrap_or_else(|err| panic!("response: {err}")),
        create_final_assistant_message_sse_response("first worker reply")
            .unwrap_or_else(|err| panic!("response: {err}")),
        create_final_assistant_message_sse_response("second worker reply")
            .unwrap_or_else(|err| panic!("response: {err}")),
        create_shell_command_sse_response(
            if cfg!(windows) {
                vec![
                    "powershell".to_string(),
                    "-Command".to_string(),
                    "Write-Output shell-summary".to_string(),
                ]
            } else {
                vec![
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    "printf shell-summary".to_string(),
                ]
            },
            Some(&workspace_root.join("worktrees/feature-a")),
            Some(1_000),
            "call-shell",
        )
        .unwrap_or_else(|err| panic!("shell response: {err}")),
        create_final_assistant_message_sse_response("shell complete")
            .unwrap_or_else(|err| panic!("response: {err}")),
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");

    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");

    let director_send = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "director", "--message", "seed director"],
    );
    assert_success(&director_send, "tt worker send director");

    let worker_send_one = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "feature-a", "--message", "first turn"],
    );
    assert_success(&worker_send_one, "tt worker send feature-a first");
    let worker_send_two = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "feature-a", "--message", "second turn"],
    );
    assert_success(&worker_send_two, "tt worker send feature-a second");
    let worker_send_three = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "run shell summary",
        ],
    );
    assert_success(&worker_send_three, "tt worker send feature-a shell");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let director_thread_id = worker_thread_id(&workspace_root, "director");
    let feature_thread_id = worker_thread_id(&workspace_root, "feature-a");
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &director_thread_id).await;
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &feature_thread_id).await;

    let read_last_two = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "read", "feature-a", "--turns", "2"],
    );
    assert_success(&read_last_two, "tt worker read --turns");
    let read_last_two_stdout = String::from_utf8_lossy(&read_last_two.stdout);
    assert!(read_last_two_stdout.contains("worker: feature-a"));
    assert!(read_last_two_stdout.contains("selected_turns: 2"));
    assert!(read_last_two_stdout.contains("second turn"));
    assert!(read_last_two_stdout.contains("run shell summary"));
    assert!(!read_last_two_stdout.contains("first turn"));

    let read_all = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "read", "feature-a", "--all"],
    );
    assert_success(&read_all, "tt worker read --all");
    let read_all_stdout = String::from_utf8_lossy(&read_all.stdout);
    assert!(read_all_stdout.contains("first turn"));
    assert!(read_all_stdout.contains("second turn"));
    assert!(read_all_stdout.contains("[command_execution]"));
    assert!(read_all_stdout.contains("shell complete"));

    let worker_list = run_tt(&tt_bin, &workspace_root, &["worker", "list"]);
    assert_success(&worker_list, "tt worker list");
    let worker_list_stdout = String::from_utf8_lossy(&worker_list.stdout);
    assert!(worker_list_stdout.contains("NAME\tKIND\tSTATUS\tCWD\tTHREAD_ID"));
    assert!(worker_list_stdout.contains("feature-a\tworker\tidle"));

    let preserved_thread_id = feature_thread_id.clone();
    let remove_output = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "feature-a"]);
    assert_success(&remove_output, "tt worker remove");
    assert!(workspace_root.join("worktrees/feature-a").exists());

    let removed_thread = thread_read(&client, &mut next_request_id, &preserved_thread_id).await;
    assert_eq!(removed_thread.thread.id, preserved_thread_id);

    let remove_director = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "director"]);
    assert_failure_contains(&remove_director, "cannot remove preset worker `director`");
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_read_defaults_to_last_ten_turns_for_preset_worker() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let server = create_mock_responses_server_repeating_assistant("director reply").await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let director_thread_id = worker_thread_id(&workspace_root, "director");

    for index in 0..12 {
        let message = format!("director turn #{index:02}");
        let send_output = run_tt(
            &tt_bin,
            &workspace_root,
            &["worker", "send", "director", "--message", &message],
        );
        assert_success(&send_output, "tt worker send director");
        let _ = wait_for_thread_idle(&client, &mut next_request_id, &director_thread_id).await;
    }

    let read_output = run_tt(&tt_bin, &workspace_root, &["worker", "read", "director"]);
    assert_success(&read_output, "tt worker read director");
    let read_stdout = String::from_utf8_lossy(&read_output.stdout);
    assert!(read_stdout.contains("worker: director"));
    assert!(read_stdout.contains("selected_turns: 10"));
    assert!(read_stdout.contains("director turn #11"));
    assert!(!read_stdout.contains("director turn #00"));
    assert!(!read_stdout.contains("director turn #01"));
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_adopt_rejects_outside_workspace_and_survives_restart() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let server = create_mock_responses_server_repeating_assistant("adopted reply").await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let outside_dir = root.join("outside-thread");
    fs::create_dir_all(&outside_dir).unwrap_or_else(|err| panic!("create outside dir: {err}"));

    let outside_thread: ThreadStartResponse = client
        .request_typed(ClientRequest::ThreadStart {
            request_id: request_id(&mut next_request_id),
            params: ThreadStartParams {
                cwd: Some(outside_dir.display().to_string()),
                ephemeral: Some(false),
                persist_extended_history: true,
                ..Default::default()
            },
        })
        .await
        .unwrap_or_else(|err| panic!("start outside thread: {err}"));
    let reject_outside = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "adopt",
            "outside",
            "--thread-id",
            outside_thread.thread.id.as_str(),
        ],
    );
    assert_failure_contains(&reject_outside, "cwd is outside this workspace");

    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");
    let managed_thread_id = worker_thread_id(&workspace_root, "feature-a");
    let managed_send = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "seed managed thread",
        ],
    );
    assert_success(&managed_send, "tt worker send feature-a");
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &managed_thread_id).await;

    let remove_managed = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "feature-a"]);
    assert_success(&remove_managed, "tt worker remove feature-a");
    let adopt_output = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "adopt",
            "adopted",
            "--thread-id",
            managed_thread_id.as_str(),
        ],
    );
    assert_success(&adopt_output, "tt worker adopt");
    assert_eq!(
        worker_binding(&workspace_root, "adopted"),
        WorkerBinding::Adopted
    );

    let duplicate_adopt = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "adopt",
            "adopted",
            "--thread-id",
            managed_thread_id.as_str(),
        ],
    );
    assert_failure_contains(&duplicate_adopt, "worker `adopted` already exists");

    let adopted_send = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "adopted", "--message", "before restart"],
    );
    assert_success(&adopted_send, "tt worker send adopted");
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &managed_thread_id).await;

    let stop_output = run_tt(&tt_bin, &workspace_root, &["stop"]);
    assert_success(&stop_output, "tt stop");
    let restart_output = run_tt(&tt_bin, &workspace_root, &["start"]);
    assert_success(&restart_output, "tt restart");

    let restarted_state = load_tt_state(&workspace_root);
    let restarted_thread_id = restarted_state
        .workers
        .into_iter()
        .find(|worker| worker.name == "adopted")
        .and_then(|worker| worker.thread_id)
        .unwrap_or_else(|| panic!("adopted thread id after restart"));
    assert_eq!(restarted_thread_id, managed_thread_id);

    let read_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "read", "adopted", "--all"],
    );
    assert_success(&read_output, "tt worker read adopted");
    let read_stdout = String::from_utf8_lossy(&read_output.stdout);
    assert!(read_stdout.contains("before restart"));

    let restarted_client = connect_runtime(&workspace_root).await;
    let adopted_send_after_restart = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "adopted", "--message", "after restart"],
    );
    assert_success(
        &adopted_send_after_restart,
        "tt worker send adopted after restart",
    );
    let _ = wait_for_thread_idle(
        &restarted_client,
        &mut next_request_id,
        &restarted_thread_id,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_adopted_binding_stays_bound_and_unavailable_after_restart() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let server = create_mock_responses_server_repeating_assistant("adopted reply").await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");
    let managed_thread_id = worker_thread_id(&workspace_root, "feature-a");
    let managed_send = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "seed managed thread",
        ],
    );
    assert_success(&managed_send, "tt worker send feature-a");
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &managed_thread_id).await;

    let remove_managed = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "feature-a"]);
    assert_success(&remove_managed, "tt worker remove feature-a");
    let adopt_output = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "adopt",
            "adopted",
            "--thread-id",
            managed_thread_id.as_str(),
        ],
    );
    assert_success(&adopt_output, "tt worker adopt");

    let stop_output = run_tt(&tt_bin, &workspace_root, &["stop"]);
    assert_success(&stop_output, "tt stop");

    let broken_thread_id = "thr_broken_adopted_binding".to_string();
    let mut state = load_tt_state(&workspace_root);
    let adopted = state
        .workers
        .iter_mut()
        .find(|worker| worker.name == "adopted")
        .unwrap_or_else(|| panic!("adopted worker"));
    adopted.thread_id = Some(broken_thread_id.clone());
    adopted.binding = WorkerBinding::Adopted;
    save_tt_state(&workspace_root, &state);

    let restart_output = run_tt(&tt_bin, &workspace_root, &["start"]);
    assert_success(&restart_output, "tt restart");

    let restarted_state = load_tt_state(&workspace_root);
    let adopted = restarted_state
        .workers
        .iter()
        .find(|worker| worker.name == "adopted")
        .unwrap_or_else(|| panic!("adopted worker after restart"));
    assert_eq!(adopted.binding, WorkerBinding::Adopted);
    assert_eq!(
        adopted.thread_id.as_deref(),
        Some(broken_thread_id.as_str())
    );
    let log_text = fs::read_to_string(workspace_root.join(".codex/tt/log.ndjson"))
        .unwrap_or_else(|err| panic!("read tt log: {err}"));
    assert!(log_text.contains("worker-resume-failed"));
    assert!(log_text.contains(&broken_thread_id));

    let worker_list = run_tt(&tt_bin, &workspace_root, &["worker", "list"]);
    assert_success(&worker_list, "tt worker list");
    let worker_list_stdout = String::from_utf8_lossy(&worker_list.stdout);
    assert!(worker_list_stdout.contains("adopted\tworker\tunavailable"));

    let read_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "read", "adopted", "--all"],
    );
    assert_failure_contains(&read_output, "worker `adopted` is bound to adopted thread");
    assert_failure_contains(&read_output, "remove or re-adopt the worker");

    let send_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "adopted", "--message", "hello"],
    );
    assert_failure_contains(&send_output, "worker `adopted` is bound to adopted thread");
    assert_failure_contains(&send_output, "remove or re-adopt the worker");

    let remove_output = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "adopted"]);
    assert_success(&remove_output, "tt worker remove adopted");
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_read_send_and_adopt_require_running_runtime() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);

    let read_output = run_tt(&tt_bin, &workspace_root, &["worker", "read", "director"]);
    assert_failure_contains(
        &read_output,
        "TT runtime is not running; use `tt start` first",
    );

    let send_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "director", "--message", "hello"],
    );
    assert_failure_contains(
        &send_output,
        "TT runtime is not running; use `tt start` first",
    );

    let adopt_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "adopt", "imported", "--thread-id", "thr_missing"],
    );
    assert_failure_contains(
        &adopt_output,
        "TT runtime is not running; use `tt start` first",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_control_reports_unknown_workers_and_invalid_adoptions_clearly() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let server = create_mock_responses_server_repeating_assistant("worker reply").await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");

    let read_output = run_tt(&tt_bin, &workspace_root, &["worker", "read", "missing"]);
    assert_failure_contains(&read_output, "unknown worker `missing`");

    let send_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "missing", "--message", "hello"],
    );
    assert_failure_contains(&send_output, "unknown worker `missing`");

    let remove_output = run_tt(&tt_bin, &workspace_root, &["worker", "remove", "missing"]);
    assert_failure_contains(&remove_output, "unknown worker `missing`");

    let adopt_output = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "adopt", "imported", "--thread-id", "thr_missing"],
    );
    assert_failure_contains(&adopt_output, "validate thread `thr_missing`");
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_send_can_steer_active_turn() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let responses = vec![
        create_shell_command_sse_response(
            if cfg!(windows) {
                vec![
                    "powershell".to_string(),
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 2".to_string(),
                ]
            } else {
                vec!["sleep".to_string(), "2".to_string()]
            },
            Some(&workspace_root.join("worktrees/feature-a")),
            Some(5_000),
            "sleep-call",
        )
        .unwrap_or_else(|err| panic!("shell response: {err}")),
        create_final_assistant_message_sse_response("steered completion")
            .unwrap_or_else(|err| panic!("response: {err}")),
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");
    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");

    let first_send = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "start long turn",
        ],
    );
    assert_success(&first_send, "tt worker send first");
    let second_send = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "steer this turn",
        ],
    );
    assert_success(&second_send, "tt worker send second");
    let second_stdout = String::from_utf8_lossy(&second_send.stdout);
    assert!(second_stdout.contains("mode: steer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_send_rejects_non_steerable_active_review_turn() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let responses = vec![
        create_shell_command_sse_response(
            if cfg!(windows) {
                vec![
                    "powershell".to_string(),
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 2".to_string(),
                ]
            } else {
                vec!["sleep".to_string(), "2".to_string()]
            },
            Some(&workspace_root.join("worktrees/feature-a")),
            Some(5_000),
            "review-sleep",
        )
        .unwrap_or_else(|err| panic!("shell response: {err}")),
        create_final_assistant_message_sse_response("review complete")
            .unwrap_or_else(|err| panic!("response: {err}")),
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");
    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let feature_thread_id = worker_thread_id(&workspace_root, "feature-a");

    let review_start: ReviewStartResponse = client
        .request_typed(ClientRequest::ReviewStart {
            request_id: request_id(&mut next_request_id),
            params: ReviewStartParams {
                thread_id: feature_thread_id.clone(),
                delivery: Some(ReviewDelivery::Inline),
                target: ReviewTarget::Custom {
                    instructions: "Check the current changes".to_string(),
                },
            },
        })
        .await
        .unwrap_or_else(|err| panic!("review/start: {err}"));
    assert_eq!(review_start.review_thread_id, feature_thread_id);
    assert_eq!(review_start.turn.status, TurnStatus::InProgress);

    let send_output = run_tt(
        &tt_bin,
        &workspace_root,
        &[
            "worker",
            "send",
            "feature-a",
            "--message",
            "interrupt review",
        ],
    );
    assert_failure_contains(&send_output, "active turn is not steerable");
}

#[tokio::test(flavor = "multi_thread")]
async fn supervisor_tools_are_handled_only_for_supervisor_threads() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let responses = vec![
        create_final_assistant_message_sse_response("seeded director")
            .unwrap_or_else(|err| panic!("response: {err}")),
        core_test_support::responses::sse(vec![
            core_test_support::responses::ev_response_created("resp-list"),
            core_test_support::responses::ev_function_call("call-list", "tt_worker_list", "{}"),
            core_test_support::responses::ev_completed("resp-list"),
        ]),
        create_final_assistant_message_sse_response("listed workers")
            .unwrap_or_else(|err| panic!("response: {err}")),
        core_test_support::responses::sse(vec![
            core_test_support::responses::ev_response_created("resp-read"),
            core_test_support::responses::ev_function_call(
                "call-read",
                "tt_worker_read",
                r#"{"name":"director","all":true}"#,
            ),
            core_test_support::responses::ev_completed("resp-read"),
        ]),
        create_final_assistant_message_sse_response("read worker")
            .unwrap_or_else(|err| panic!("response: {err}")),
        core_test_support::responses::sse(vec![
            core_test_support::responses::ev_response_created("resp-send"),
            core_test_support::responses::ev_function_call(
                "call-send",
                "tt_worker_send",
                r#"{"name":"director","message":"from supervisor tool"}"#,
            ),
            core_test_support::responses::ev_completed("resp-send"),
        ]),
        create_final_assistant_message_sse_response("director tool reply")
            .unwrap_or_else(|err| panic!("response: {err}")),
        create_final_assistant_message_sse_response("sent worker")
            .unwrap_or_else(|err| panic!("response: {err}")),
        core_test_support::responses::sse(vec![
            core_test_support::responses::ev_response_created("resp-remove"),
            core_test_support::responses::ev_function_call(
                "call-remove",
                "tt_worker_remove",
                r#"{"name":"feature-a"}"#,
            ),
            core_test_support::responses::ev_completed("resp-remove"),
        ]),
        create_final_assistant_message_sse_response("removed worker")
            .unwrap_or_else(|err| panic!("response: {err}")),
        create_final_assistant_message_sse_response("director plain reply")
            .unwrap_or_else(|err| panic!("response: {err}")),
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");
    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");
    let seed_director = run_tt(
        &tt_bin,
        &workspace_root,
        &["worker", "send", "director", "--message", "seed director"],
    );
    assert_success(&seed_director, "seed director");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let state = load_tt_state(&workspace_root);
    let supervisor_thread_id = state
        .supervisor_thread_id
        .clone()
        .unwrap_or_else(|| panic!("supervisor thread id"));
    let director_thread_id = worker_thread_id(&workspace_root, "director");
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &director_thread_id).await;

    for prompt in [
        "list workers",
        "read worker",
        "send worker",
        "remove worker",
    ] {
        let _: TurnStartResponse = client
            .request_typed(ClientRequest::TurnStart {
                request_id: request_id(&mut next_request_id),
                params: TurnStartParams {
                    thread_id: supervisor_thread_id.clone(),
                    input: vec![UserInput::Text {
                        text: prompt.to_string(),
                        text_elements: Vec::new(),
                    }],
                    ..Default::default()
                },
            })
            .await
            .unwrap_or_else(|err| panic!("start supervisor turn: {err}"));
        let _ = wait_for_thread_idle(&client, &mut next_request_id, &supervisor_thread_id).await;
    }

    let supervisor_history =
        thread_read(&client, &mut next_request_id, &supervisor_thread_id).await;
    let dynamic_tool_names = supervisor_history
        .thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter_map(|item| match item {
            codex_app_server_protocol::ThreadItem::DynamicToolCall { tool, .. } => {
                Some(tool.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        dynamic_tool_names
            .iter()
            .any(|tool| tool == "tt_worker_list")
    );
    assert!(
        dynamic_tool_names
            .iter()
            .any(|tool| tool == "tt_worker_read")
    );
    assert!(
        dynamic_tool_names
            .iter()
            .any(|tool| tool == "tt_worker_send")
    );
    assert!(
        dynamic_tool_names
            .iter()
            .any(|tool| tool == "tt_worker_remove")
    );

    let director_history_after_tool_send =
        thread_read(&client, &mut next_request_id, &director_thread_id).await;
    let director_messages = director_history_after_tool_send
        .thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter_map(|item| match item {
            codex_app_server_protocol::ThreadItem::UserMessage { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|input| match input {
                        UserInput::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        director_messages
            .iter()
            .any(|message| message.contains("from supervisor tool")),
        "expected supervisor tool send to inject a director user message"
    );

    let worker_list = run_tt(&tt_bin, &workspace_root, &["worker", "list"]);
    assert_success(&worker_list, "tt worker list after supervisor remove");
    let worker_list_stdout = String::from_utf8_lossy(&worker_list.stdout);
    assert!(!worker_list_stdout.contains("feature-a\tworker"));

    let _: TurnStartResponse = client
        .request_typed(ClientRequest::TurnStart {
            request_id: request_id(&mut next_request_id),
            params: TurnStartParams {
                thread_id: director_thread_id.clone(),
                input: vec![UserInput::Text {
                    text: "director plain prompt".to_string(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await
        .unwrap_or_else(|err| panic!("start director turn: {err}"));
    let _ = wait_for_thread_idle(&client, &mut next_request_id, &director_thread_id).await;

    let requests = server
        .received_requests()
        .await
        .unwrap_or_else(|| panic!("fetch received requests"));
    let response_bodies = requests
        .into_iter()
        .filter(|request| request.url.path().ends_with("/responses"))
        .map(|request| {
            request
                .body_json::<serde_json::Value>()
                .unwrap_or_else(|err| panic!("responses request body json: {err}"))
        })
        .collect::<Vec<_>>();

    let supervisor_request = response_bodies
        .iter()
        .find(|body| body.to_string().contains("list workers"))
        .unwrap_or_else(|| panic!("supervisor request body"));
    assert!(
        supervisor_request
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("name").and_then(serde_json::Value::as_str) == Some("tt_worker_list")
                })
            }),
        "supervisor turn should expose tt_worker_list"
    );

    let director_plain_request = response_bodies
        .iter()
        .find(|body| body.to_string().contains("director plain prompt"))
        .unwrap_or_else(|| panic!("director plain request"));
    assert!(
        !director_plain_request
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("name")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|name| name.starts_with("tt_worker_"))
                })
            }),
        "non-supervisor turn should not expose TT supervisor tools"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn supervisor_tool_worker_failures_return_failed_results_without_mutating_state() {
    let tempdir = tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let root = tempdir.path();
    let tt_bin = cargo_bin("tt").unwrap_or_else(|err| panic!("resolve tt binary: {err}"));
    let workspace_root = setup_workspace(&tt_bin, root);
    let primary_checkout = workspace_root.join("primary");

    let responses = vec![
        core_test_support::responses::sse(vec![
            core_test_support::responses::ev_response_created("resp-fail"),
            core_test_support::responses::ev_function_call(
                "call-remove-missing",
                "tt_worker_remove",
                r#"{"name":"missing"}"#,
            ),
            core_test_support::responses::ev_completed("resp-fail"),
        ]),
        create_final_assistant_message_sse_response("tool failure handled")
            .unwrap_or_else(|err| panic!("response: {err}")),
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    write_mock_config(&workspace_root, &server.uri());

    let start_output = run_tt(&tt_bin, &primary_checkout, &["start"]);
    assert_success(&start_output, "tt start");
    let add_output = run_tt(&tt_bin, &workspace_root, &["worker", "add", "feature-a"]);
    assert_success(&add_output, "tt worker add");

    let client = connect_runtime(&workspace_root).await;
    let mut next_request_id = 1;
    let supervisor_thread_id = load_tt_state(&workspace_root)
        .supervisor_thread_id
        .unwrap_or_else(|| panic!("supervisor thread id"));

    let _: TurnStartResponse = client
        .request_typed(ClientRequest::TurnStart {
            request_id: request_id(&mut next_request_id),
            params: TurnStartParams {
                thread_id: supervisor_thread_id.clone(),
                input: vec![UserInput::Text {
                    text: "remove missing worker".to_string(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await
        .unwrap_or_else(|err| panic!("start supervisor turn: {err}"));
    let supervisor_history =
        wait_for_thread_idle(&client, &mut next_request_id, &supervisor_thread_id).await;

    let failed_tool_call = supervisor_history
        .thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .find_map(|item| match item {
            codex_app_server_protocol::ThreadItem::DynamicToolCall {
                tool,
                success,
                content_items,
                ..
            } if tool == "tt_worker_remove" => Some((success, content_items)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing tt_worker_remove tool call"));
    assert_eq!(failed_tool_call.0, &Some(false));
    let tool_text = failed_tool_call
        .1
        .as_ref()
        .unwrap_or_else(|| panic!("tool content items"))
        .iter()
        .filter_map(|item| match item {
            codex_app_server_protocol::DynamicToolCallOutputContentItem::InputText { text } => {
                Some(text.as_str())
            }
            codex_app_server_protocol::DynamicToolCallOutputContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(tool_text.contains("unknown worker `missing`"));

    let worker_list = run_tt(&tt_bin, &workspace_root, &["worker", "list"]);
    assert_success(&worker_list, "tt worker list");
    let worker_list_stdout = String::from_utf8_lossy(&worker_list.stdout);
    assert!(worker_list_stdout.contains("feature-a\tworker"));
}
