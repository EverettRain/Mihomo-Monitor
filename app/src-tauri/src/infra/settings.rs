//! User settings, stored as JSON next to the database.

use super::paths;
use serde::{Deserialize, Serialize};

/// A core the user added by hand: a controller address on this machine or elsewhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualSource {
    pub name: String,
    /// `http://host:port` of the external controller.
    pub url: String,
    #[serde(default)]
    pub secret: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Closing the window destroys the interface (and frees its memory) instead of hiding it.
    pub lightweight: bool,
    /// Start-at-login is turned on once, at the first launch; afterwards the user's choice
    /// stays with the system.
    pub autostart_initialized: bool,
    /// Weeks start on Sunday instead of Monday.
    pub week_starts_sunday: bool,
    /// Look for cores on this machine; off, only the manual entries are read.
    pub auto_discover: bool,
    pub manual_sources: Vec<ManualSource>,
    /// How often each core's connections are read, in milliseconds.
    pub sample_ms: u64,
    /// The rule configuration in use (host groups and attribution rules).
    pub rules_profile: String,
}

/// The sampling intervals offered, in milliseconds. Shorter misses fewer connections that open
/// and close between two readings; each reading costs about 1 ms per 1000 open connections.
pub const SAMPLE_CHOICES: [u64; 3] = [500, 1_000, 2_000];

/// `ms` when it is one of the choices, else the default.
pub fn sample_ms(ms: u64) -> u64 {
    if SAMPLE_CHOICES.contains(&ms) {
        ms
    } else {
        1_000
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lightweight: true,
            autostart_initialized: false,
            week_starts_sunday: false,
            auto_discover: true,
            manual_sources: Vec::new(),
            sample_ms: 1_000,
            rules_profile: crate::infra::profiles::DEFAULT.to_string(),
        }
    }
}

/// Missing or unreadable settings fall back to the defaults rather than stopping the app.
pub fn load() -> Settings {
    std::fs::read(paths::settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) -> Result<(), String> {
    let path = paths::settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建数据目录：{e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| format!("无法保存设置：{e}"))
}

/// Reads, changes and saves the settings in one step.
pub fn update(change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let mut s = load();
    change(&mut s);
    save(&s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_missing_from_older_files_take_their_defaults() {
        // Arrange
        let old = r#"{"lightweight": false}"#;

        // Act
        let s: Settings = serde_json::from_str(old).unwrap();

        // Assert
        assert_eq!(s, Settings { lightweight: false, ..Settings::default() });
    }
}
