use std::ffi::OsString;

use codex_arg0::Arg0PathEntryGuard;
use codex_arg0::arg0_dispatch;
use ctor::ctor;
use tempfile::TempDir;

struct TestCodexAliasesGuard {
    _codex_home: TempDir,
    _arg0: Arg0PathEntryGuard,
    _previous_codex_home: Option<OsString>,
}

const CODEX_HOME_ENV_VAR: &str = "CODEX_HOME";

// This code runs before any other tests are run.
// It allows the test binary to behave like codex and dispatch to apply_patch and codex-linux-sandbox
// based on the arg0.
// NOTE: this doesn't work on ARM
#[ctor]
pub static CODEX_ALIASES_TEMP_DIR: TestCodexAliasesGuard = unsafe {
    let codex_home = tempfile::Builder::new()
        .prefix("codex-core-tests")
        .tempdir()
        .expect("create codex-core-tests tempdir");
    let previous_codex_home = std::env::var_os(CODEX_HOME_ENV_VAR);
    // arg0_dispatch() creates helper links under CODEX_HOME/tmp. Point it at a
    // test-owned temp dir so startup never mutates the developer's real ~/.codex.
    //
    // Safety: #[ctor] runs before tests start, so no test threads exist yet.
    unsafe {
        std::env::set_var(CODEX_HOME_ENV_VAR, codex_home.path());
    }

    let arg0 = arg0_dispatch().expect("set up arg0 dispatch helpers");
    // Restore the process environment immediately so later tests observe the
    // same CODEX_HOME state they started with.
    match previous_codex_home.as_ref() {
        Some(value) => unsafe {
            std::env::set_var(CODEX_HOME_ENV_VAR, value);
        },
        None => unsafe {
            std::env::remove_var(CODEX_HOME_ENV_VAR);
        },
    }

    TestCodexAliasesGuard {
        _codex_home: codex_home,
        _arg0: arg0,
        _previous_codex_home: previous_codex_home,
    }
};
