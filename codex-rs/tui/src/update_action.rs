#[cfg(any(not(debug_assertions), test))]
use codex_install_context::InstallContext;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::StandalonePlatform;

/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Rebuild the supported local binaries from source.
    RebuildFromSource,
}

impl UpdateAction {
    #[cfg(any(not(debug_assertions), test))]
    pub(crate) fn from_install_context(context: &InstallContext) -> Option<Self> {
        match context {
            InstallContext::Npm => Some(UpdateAction::NpmGlobalLatest),
            InstallContext::Bun => Some(UpdateAction::BunGlobalLatest),
            InstallContext::Brew => Some(UpdateAction::BrewUpgrade),
            InstallContext::Standalone { platform, .. } => Some(match platform {
                StandalonePlatform::Unix => UpdateAction::StandaloneUnix,
                StandalonePlatform::Windows => UpdateAction::StandaloneWindows,
            }),
            InstallContext::Other => None,
        }
    }

    /// Returns the list of command-line arguments for invoking the update.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        let _ = self;
        ("make", &["install"])
    }

    /// Returns string representation of the command-line arguments for invoking the update.
    pub fn command_str(self) -> String {
        let (command, args) = self.command_args();
        shlex::try_join(std::iter::once(command).chain(args.iter().copied()))
            .unwrap_or_else(|_| format!("{command} {}", args.join(" ")))
    }
}

#[cfg(not(debug_assertions))]
pub(crate) fn get_update_action() -> Option<UpdateAction> {
    Some(UpdateAction::RebuildFromSource)
}

#[cfg(any(not(debug_assertions), test))]
fn detect_update_action(
    _is_macos: bool,
    current_exe: &std::path::Path,
    _managed_by_npm: bool,
    _managed_by_bun: bool,
) -> Option<UpdateAction> {
    if current_exe.as_os_str().is_empty() {
        None
    } else {
        Some(UpdateAction::RebuildFromSource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn maps_install_context_to_update_action() {
        let native_release_dir = PathBuf::from("/tmp/native-release");

        assert_eq!(
            detect_update_action(
                /*is_macos*/ false,
                std::path::Path::new("/any/path"),
                /*managed_by_npm*/ false,
                /*managed_by_bun*/ false
            ),
            Some(UpdateAction::RebuildFromSource)
        );
        assert_eq!(
            detect_update_action(
                /*is_macos*/ false,
                std::path::Path::new("/any/path"),
                /*managed_by_npm*/ true,
                /*managed_by_bun*/ false
            ),
            Some(UpdateAction::RebuildFromSource)
        );
        assert_eq!(
            detect_update_action(
                /*is_macos*/ false,
                std::path::Path::new("/any/path"),
                /*managed_by_npm*/ false,
                /*managed_by_bun*/ true
            ),
            Some(UpdateAction::RebuildFromSource)
        );
        assert_eq!(
            detect_update_action(
                /*is_macos*/ true,
                std::path::Path::new("/opt/homebrew/bin/codex"),
                /*managed_by_npm*/ false,
                /*managed_by_bun*/ false
            ),
            Some(UpdateAction::RebuildFromSource)
        );
        assert_eq!(
            detect_update_action(
                /*is_macos*/ true,
                std::path::Path::new("/usr/local/bin/codex"),
                /*managed_by_npm*/ false,
                /*managed_by_bun*/ false
            ),
            Some(UpdateAction::RebuildFromSource)
        );
    }
}
