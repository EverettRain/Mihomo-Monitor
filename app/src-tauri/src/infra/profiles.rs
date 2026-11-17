//! Rule configurations: each one a rules file (see `rules_file`) in a folder of the data
//! directory, named after the file. One is in use at a time; the app keeps it in step with the
//! file both ways, so the file can be opened and edited directly.

use std::path::{Path, PathBuf};

/// The configuration every installation starts with.
pub const DEFAULT: &str = "Default";

const EXT: &str = "yaml";

/// Where the configurations of the data directory at `data` live.
pub fn dir_in(data: &Path) -> PathBuf {
    data.join("rules")
}

pub fn path_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXT}"))
}

/// A configuration name as it may be used for a file: trimmed, not empty, not too long, and
/// without characters any of the supported systems refuse in file names.
pub fn checked_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("请填写配置名称。".into());
    }
    if name.chars().count() > 40 {
        return Err("配置名称最多 40 个字。".into());
    }
    if name.starts_with('.') || name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err("配置名称不能以 . 开头，也不能含有 / \\ : * ? \" < > | 等字符。".into());
    }
    Ok(name.to_string())
}

/// The configurations present, the default first, then by name.
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension().and_then(|x| x.to_str()) == Some(EXT)).then(|| path.file_stem()?.to_str().map(str::to_string)).flatten()
        })
        .filter(|n| checked_name(n).is_ok())
        .collect();
    names.sort_by(|a, b| (a != DEFAULT).cmp(&(b != DEFAULT)).then_with(|| a.cmp(b)));
    names
}

pub fn read(dir: &Path, name: &str) -> Result<String, String> {
    std::fs::read_to_string(path_of(dir, name)).map_err(|e| format!("无法读取配置「{name}」：{e}"))
}

/// Writes a configuration whole: to a temporary file first, then in its place, so an editor or
/// a reader never sees half of it.
pub fn write(dir: &Path, name: &str, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    let path = path_of(dir, name);
    let tmp = dir.join(format!(".{name}.{EXT}.tmp"));
    std::fs::write(&tmp, text).map_err(|e| format!("无法写入配置「{name}」：{e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("无法写入配置「{name}」：{e}"))
}

pub fn delete(dir: &Path, name: &str) -> Result<(), String> {
    std::fs::remove_file(path_of(dir, name)).map_err(|e| format!("无法删除配置「{name}」：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_make_file_names_anywhere() {
        // Act
        let ok = ["Default", " 公司 ", "Home-NAS 2"].map(|n| checked_name(n).is_ok());
        let bad = ["", "a/b", "a\\b", ".hidden", "x:y", &"名".repeat(41)].map(|n| checked_name(n).is_ok());

        // Assert
        assert_eq!(ok, [true, true, true]);
        assert_eq!(bad, [false; 6]);
    }

    #[test]
    fn configurations_are_listed_default_first_and_written_whole() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        for name in ["工作", "Default", "Alpha"] {
            write(dir.path(), name, "groups: []\n").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), "not a configuration").unwrap();

        // Act
        let names = list(dir.path());
        delete(dir.path(), "Alpha").unwrap();

        // Assert
        assert_eq!(names, ["Default", "Alpha", "工作"]);
        assert_eq!(list(dir.path()), ["Default", "工作"]);
        assert_eq!(read(dir.path(), "工作").unwrap(), "groups: []\n");
        assert!(!dir.path().join(".工作.yaml.tmp").exists());
    }
}
