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
    pub manual_sources: Vec<ManualSource>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { lightweight: true, autostart_initialized: false, week_starts_sunday: false, manual_sources: Vec::new() }
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
