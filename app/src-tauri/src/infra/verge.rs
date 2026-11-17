//! Clash Verge Rev's own files: which profile (the subscription, or a local config) is active,
//! and what each subscription's provider reported.
//!
//! Verge downloads a subscription and expands its nodes into the core's config, so the core
//! cannot say which subscription a node came from. The active profile can: `profiles.yaml` in
//! Verge's data folder names it (`current` is the uid of an entry in `items`), and a remote
//! profile keeps the provider's usage report in `extra`. Verified with Verge 2.5.6, also in
//! service mode, where the file stays in the user's own folder.

use crate::domain::cycle::Reported;
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

/// Each named profile's usage report (`extra`); local profiles have none.
pub fn reported(text: &str) -> Vec<(String, Reported)> {
    let Ok(docs) = YamlLoader::load_from_str(text) else { return Vec::new() };
    let Some(items) = docs.first().and_then(|d| d["items"].as_vec()) else { return Vec::new() };
    let int = |y: &Yaml| y.as_i64().filter(|v| *v > 0);
    items
        .iter()
        .filter_map(|i| {
            let name = i["name"].as_str()?.trim();
            let extra = &i["extra"];
            if name.is_empty() || extra.is_badvalue() || extra.is_null() {
                return None;
            }
            let r = Reported {
                upload: int(&extra["upload"]).unwrap_or(0) as u64,
                download: int(&extra["download"]).unwrap_or(0) as u64,
                total: int(&extra["total"]).unwrap_or(0) as u64,
                expire: int(&extra["expire"]),
                updated: int(&i["updated"]),
            };
            Some((name.to_string(), r))
        })
        .collect()
}

/// `profiles.yaml`, re-read only when it changes.
#[derive(Default)]
pub struct Profiles {
    seen: Option<SystemTime>,
    active: Option<String>,
    reported: Vec<(String, Reported)>,
}

impl Profiles {
    pub fn refresh(&mut self) {
        let Some(path) = profiles_path() else { return };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified != self.seen {
            self.seen = modified;
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            self.active = active_profile(&text);
            self.reported = reported(&text);
        }
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn reported(&self) -> &[(String, Reported)] {
        &self.reported
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
    fn remote_profiles_carry_the_providers_report() {
        // Arrange
        let text = "items:
- uid: a
  type: local
  name: VPS
- uid: b
  type: remote
  name: 白月光
  updated: 1794800000
  extra:
    upload: 1073741824
    download: 10737418240
    total: 214748364800
    expire: 1798000000
- uid: c
  type: remote
  name: 不限量
  extra:
    upload: 0
    download: 5
    total: 0
    expire: 0
";

        // Act
        let got = reported(text);

        // Assert
        assert_eq!(
            got,
            vec![
                (
                    "白月光".to_string(),
                    Reported { upload: 1 << 30, download: 10 << 30, total: 200 << 30, expire: Some(1798000000), updated: Some(1794800000) }
                ),
                ("不限量".to_string(), Reported { upload: 0, download: 5, total: 0, expire: None, updated: None }),
            ]
        );
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
