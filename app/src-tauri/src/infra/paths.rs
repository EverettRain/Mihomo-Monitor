//! Where Mihomo-Monitor keeps its files, and its small log.

use std::path::PathBuf;

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Data directory: settings, the statistics database and logs. `MIHOMO_MONITOR_HOME` moves it
/// (the screenshot script points it at demo data).
pub fn data_dir() -> PathBuf {
    if let Some(p) = env_dir("MIHOMO_MONITOR_HOME") {
        return p;
    }
    #[cfg(windows)]
    {
        env_dir("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")).join("Mihomo-Monitor")
    }
    #[cfg(target_os = "macos")]
    {
        env_dir("HOME").unwrap_or_else(|| PathBuf::from("/tmp")).join("Library/Application Support/Mihomo-Monitor")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        env_dir("XDG_DATA_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mihomo-monitor")
    }
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn database_path() -> PathBuf {
    data_dir().join("traffic.db")
}

fn log_path() -> PathBuf {
    data_dir().join("logs").join("monitor.log")
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Appends one line to the log; the file is rotated at 256 KB, keeping one older file.
pub fn log(message: &str) {
    use std::io::Write;
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = jiff::Timestamp::now().round(jiff::Unit::Second).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        let _ = writeln!(f, "[{stamp}] {message}");
    }
}
