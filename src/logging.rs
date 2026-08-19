//! Log destinations for the two run modes.

use std::path::PathBuf;

/// Environment variable that overrides the log directory outright.
pub const LOG_DIR_ENV: &str = "ASSETUI_LOG_DIR";

pub const LOG_FILE_PREFIX: &str = "assetui";

/// Number of daily log files kept before the oldest is deleted.
pub const LOG_FILES_KEPT: usize = 7;

/// The directory the TUI writes its log file to.
pub fn log_dir() -> PathBuf {
    resolve_log_dir(
        std::env::var_os(LOG_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        platform_log_dir(),
        std::env::temp_dir().join(LOG_FILE_PREFIX),
    )
}

pub fn resolve_log_dir(
    explicit: Option<PathBuf>,
    platform: Option<PathBuf>,
    fallback: PathBuf,
) -> PathBuf {
    explicit.or(platform).unwrap_or(fallback)
}

fn platform_log_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return home().map(|h| h.join("Library/Logs").join(LOG_FILE_PREFIX));
    }
    if let Some(state) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(state).join(LOG_FILE_PREFIX));
    }
    home().map(|h| h.join(".local/state").join(LOG_FILE_PREFIX))
}

/// The user's home directory, or `None` when the platform can't say.
fn home() -> Option<PathBuf> {
    std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn resolve_log_dir_prefers_the_explicit_override() {
        let resolved = resolve_log_dir(
            Some(path("/explicit")),
            Some(path("/platform")),
            path("/fallback"),
        );

        assert_eq!(resolved, path("/explicit"));
    }

    #[test]
    fn resolve_log_dir_uses_the_platform_dir_without_an_override() {
        let resolved = resolve_log_dir(None, Some(path("/platform")), path("/fallback"));

        assert_eq!(resolved, path("/platform"));
    }

    #[test]
    fn resolve_log_dir_falls_back_when_the_platform_has_no_home() {
        let resolved = resolve_log_dir(None, None, path("/fallback"));

        assert_eq!(resolved, path("/fallback"));
    }
}
