//! Release notes for the in-app update dialog, read from the project's GitHub releases.

use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

pub const REPO: &str = "EverettRain/Mihomo-Monitor";

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNote {
    pub version: String,
    /// RFC 3339 publish time.
    pub date: String,
    /// Markdown body of the release (may be empty).
    pub body: String,
}

/// `major.minor.patch` of a stable version or `v`-prefixed tag; pre-releases are ignored.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let n = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    it.next().is_none().then_some(n)
}

/// Published stable releases with `after < version <= upto`, newest first.
fn select(releases: &Value, after: &str, upto: &str) -> Vec<ReleaseNote> {
    let (Some(lo), Some(hi)) = (parse(after), parse(upto)) else { return Vec::new() };
    let mut out: Vec<_> = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| !r["draft"].as_bool().unwrap_or(true) && !r["prerelease"].as_bool().unwrap_or(true))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let v = parse(tag)?;
            (v > lo && v <= hi).then(|| {
                (
                    v,
                    ReleaseNote {
                        version: tag.trim_start_matches('v').to_string(),
                        date: r["published_at"].as_str().unwrap_or_default().to_string(),
                        body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
                    },
                )
            })
        })
        .collect();
    out.sort_by_key(|n| std::cmp::Reverse(n.0));
    out.into_iter().map(|(_, n)| n).collect()
}

/// Notes of every release after `after` up to and including `upto`, newest first. Uses the
/// system proxy, which is often what makes GitHub reachable.
pub async fn notes(after: &str, upto: &str) -> Result<Vec<ReleaseNote>, String> {
    super::install_tls();
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|_| "无法初始化网络。")?;
    let body: Value = client
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=50"))
        .header("User-Agent", "Mihomo-Monitor")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "无法读取更新说明。")?
        .json()
        .await
        .map_err(|_| "更新说明格式无法识别。")?;
    Ok(select(&body, after, upto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_releases_between_versions_newest_first() {
        // Arrange
        let r = serde_json::json!([
            {"tag_name": "v1.0.0", "draft": false, "prerelease": false, "published_at": "a", "body": "old"},
            {"tag_name": "v1.0.1", "draft": false, "prerelease": false, "published_at": "b", "body": "one\r\ntwo"},
            {"tag_name": "v1.1.0", "draft": false, "prerelease": false, "published_at": "c", "body": "new"},
            {"tag_name": "v1.2.0", "draft": true, "prerelease": false, "body": "draft"},
            {"tag_name": "v1.1.1-beta.1", "draft": false, "prerelease": true, "body": "beta"},
            {"tag_name": "v2.0.0", "draft": false, "prerelease": false, "body": "too new"},
        ]);

        // Act
        let got = select(&r, "1.0.0", "1.1.0");

        // Assert
        assert_eq!(got.iter().map(|n| n.version.as_str()).collect::<Vec<_>>(), ["1.1.0", "1.0.1"]);
        assert_eq!(got[1].body, "one\ntwo");
        assert!(select(&r, "x", "1.1.0").is_empty());
    }

    #[test]
    fn parses_stable_versions_only() {
        // Act / Assert
        assert_eq!(parse("v1.10.2"), Some((1, 10, 2)));
        assert!(parse("1.0.1-beta").is_none());
        assert!(parse("1.0").is_none());
    }
}
