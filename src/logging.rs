use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

const RETENTION_DAYS: u64 = 30;
const LOG_FILE_PREFIX: &str = "linggen-agent";
const ROTATION_DAILY: &str = "daily";

pub struct LoggingSettings<'a> {
    pub level: Option<&'a str>,
    pub directory: Option<&'a str>,
    pub rotation: Option<&'a str>,
    pub retention_days: Option<u64>,
}

pub fn setup_tracing_with_settings(settings: LoggingSettings<'_>) -> Option<PathBuf> {
    let log_dir = resolve_log_dir(settings.directory).ok()?;
    let retention_days = settings.retention_days.unwrap_or(RETENTION_DAYS);
    let _ = cleanup_old_logs(&log_dir, retention_days);

    let rotation = settings.rotation.unwrap_or(ROTATION_DAILY);
    // Ensure guard is kept alive by not using `let _ =`
    let file_appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE_PREFIX);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Guard is intentionally stored in OnceLock to prevent it from being dropped
    let _ = LOG_GUARD.set(guard);

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact();

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact();

    let default_filter = || {
        let base = settings.level.unwrap_or("info");
        EnvFilter::new(format!(
            "linggen_agent={level},linggen-agent={level},linggen={level},\
             axum=warn,tower_http=warn,hyper=warn,hyper_util=warn,reqwest=warn,\
             mio=warn,reqwest_retry=warn",
            level = base
        ))
    };

    let filter = if settings.level.is_some() {
        default_filter()
    } else {
        match EnvFilter::try_from_default_env() {
            Ok(env_filter) => env_filter,
            Err(_) => default_filter(),
        }
    };

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    Some(log_dir)
}

pub fn setup_tracing() -> Option<PathBuf> {
    setup_tracing_with_settings(LoggingSettings {
        level: None,
        directory: None,
        rotation: None,
        retention_days: None,
    })
}

fn resolve_log_dir(configured: Option<&str>) -> Result<PathBuf> {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .ok_or_else(|| anyhow!("Could not find data directory"))?;
    let dir = if let Some(path) = configured {
        expand_tilde(path)
    } else {
        base.join("linggen-agent").join("logs")
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn cleanup_old_logs(log_dir: &PathBuf, retention_days: u64) -> Result<()> {
    let now = SystemTime::now();
    let max_age = Duration::from_secs(60 * 60 * 24 * retention_days);
    for entry in std::fs::read_dir(log_dir)? {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(v) => v,
            None => continue,
        };
        if !file_name.starts_with(LOG_FILE_PREFIX) {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let age = match now.duration_since(modified) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if age > max_age {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}
