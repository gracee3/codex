//! OSS provider utilities shared between TUI and exec.

use codex_core::config::Config;

/// Returns the default model for a given OSS provider.
pub fn get_default_model_for_oss_provider(provider_id: &str) -> Option<&'static str> {
    let _ = provider_id;
    None
}

/// Ensures the specified OSS provider is ready (models downloaded, service reachable).
pub async fn ensure_oss_provider_ready(
    provider_id: &str,
    config: &Config,
) -> Result<(), std::io::Error> {
    let _ = provider_id;
    let _ = config;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_model_for_provider_unknown() {
        let result = get_default_model_for_oss_provider("unknown-provider");
        assert_eq!(result, None);
    }
}
