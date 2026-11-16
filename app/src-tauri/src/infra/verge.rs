//! Clash Verge Rev's own files: which profile (the subscription, or a local config) is active.
//!
//! Verge downloads a subscription and expands its nodes into the core's config, so the core
//! cannot say which subscription a node came from. The active profile can: `profiles.yaml` in
//! Verge's data folder names it (`current` is the uid of an entry in `items`). Verified with
//! Verge 2.5.6, also in service mode, where the file stays in the user's own folder.

use std::path::PathBuf;
use std::time::SystemTime;
use yaml_rust2::{Yaml, YamlLoader};

const APP_ID: &str = "io.github.clash-verge-rev.clash-verge-rev";

fn profiles_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env("HOME").map(|h| h.join("Library/Application Support"));
    #[cfg(windows)]
    let base = env("APPDATA");
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")));
    base.map(|b| b.join(APP_ID).join("profiles.yaml"))
}

/// Name of the active profile in a `profiles.yaml`.
pub fn active_profile(text: &str) -> Option<String> {
    let docs = YamlLoader::load_from_str(text).ok()?;
    let doc = docs.first()?;
    let current = doc["current"].as_str()?;
    let item = doc["items"].as_vec()?.iter().find(|i| i["uid"].as_str() == Some(current))?;
    match &item["name"] {
        Yaml::String(n) if !n.trim().is_empty() => Some(n.trim().to_string()),
        _ => None,
    }
}

/// The active profile, re-read only when `profiles.yaml` changes.
#[derive(Default)]
pub struct ActiveProfile {
    seen: Option<SystemTime>,
    name: Option<String>,
}

impl ActiveProfile {
    pub fn get(&mut self) -> Option<&str> {
        let path = profiles_path()?;
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified != self.seen {
            self.seen = modified;
            self.name = std::fs::read_to_string(&path).ok().and_then(|t| active_profile(&t));
        }
        self.name.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_profile_current_points_at() {
        // Arrange: the shape of Verge 2.5.6's file (enhancement items have no name).
        let text = "current: LyLJArAEaosC
items:
- uid: m1
  type: merge
  name: null
- uid: L4pPHMNxMfE8
  type: local
  name: VPS TUN 配置
- uid: LyLJArAEaosC
  type: remote
  name: 白月光
  extra:
    upload: 1
    download: 2
    total: 3
    expire: 4
";

        // Act
        let name = active_profile(text);

        // Assert
        assert_eq!(name.as_deref(), Some("白月光"));
    }

    #[test]
    fn no_name_without_a_current_profile() {
        // Act
        let missing = active_profile("items: []\n");
        let dangling = active_profile("current: x\nitems:\n- uid: y\n  name: A\n");

        // Assert
        assert_eq!((missing, dangling), (None, None));
    }
}
