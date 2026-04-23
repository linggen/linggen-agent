pub fn now_ts_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn now_ts_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolve a path. Expands a leading `~` to `$HOME` first, then follows
/// symlinks via `canonicalize`. On macOS, `/tmp` → `/private/tmp`.
///
/// Falls back to the expanded-but-not-canonicalized path when the target
/// doesn't exist, so callers that want a best-effort absolute path (e.g.
/// mission cwd before directories are created) still get a usable value.
pub fn resolve_path(path: &std::path::Path) -> std::path::PathBuf {
    let expanded: std::path::PathBuf = {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                path.to_path_buf()
            }
        } else if s == "~" {
            dirs::home_dir().unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    };
    expanded
        .canonicalize()
        .unwrap_or(expanded)
}
