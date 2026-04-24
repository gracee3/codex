//! OSS provider utilities shared between TUI and exec.

use codex_core::config::Config;

/// Hook for future local OSS provider readiness checks.
///
/// The generic OSS path intentionally has no provider-specific side effects:
/// callers select a configured OpenAI-compatible provider, and the normal
/// model client surfaces connection/model errors.
pub async fn ensure_oss_provider_ready(
    _provider_id: &str,
    _config: &Config,
) -> std::io::Result<()> {
    Ok(())
}
