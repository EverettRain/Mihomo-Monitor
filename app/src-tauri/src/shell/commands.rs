//! Tauri commands the interface calls.

use super::tray;
use crate::infra::discovery::controller_url;
use crate::infra::paths;
use crate::infra::releases::{self, ReleaseNote};
use crate::infra::settings::{self, ManualSource};
use crate::service::monitor::{LivePoint, Monitor, Overview, SourceView};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

type Mon<'a> = State<'a, Arc<Monitor>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    platform: &'static str,
    data_dir: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        platform: if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        },
        data_dir: paths::data_dir().display().to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    lightweight: bool,
    autostart: bool,
    week_starts_sunday: bool,
    auto_discover: bool,
    manual_sources: Vec<ManualSource>,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> SettingsView {
    let s = settings::load();
    SettingsView {
        lightweight: s.lightweight,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        week_starts_sunday: s.week_starts_sunday,
        auto_discover: s.auto_discover,
        manual_sources: s.manual_sources,
    }
}

#[tauri::command]
pub fn set_lightweight(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update(|s| s.lightweight = enabled)?;
    tray::refresh_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| format!("无法更改开机自启：{e}"))
}

#[tauri::command]
pub fn set_week_starts_sunday(enabled: bool) -> Result<(), String> {
    settings::update(|s| s.week_starts_sunday = enabled).map(|_| ())
}

#[tauri::command]
pub fn set_auto_discover(monitor: Mon, enabled: bool) -> Result<(), String> {
    settings::update(|s| s.auto_discover = enabled)?;
    monitor.rediscover();
    Ok(())
}

#[tauri::command]
pub fn list_sources(monitor: Mon) -> Vec<SourceView> {
    monitor.sources()
}

#[tauri::command]
pub fn add_manual_source(monitor: Mon, name: String, url: String, secret: String) -> Result<(), String> {
    let url = controller_url(&url).ok_or("地址格式应为 127.0.0.1:9090 或 http://主机:端口")?;
    let name = if name.trim().is_empty() { url.trim_start_matches("http://").to_string() } else { name.trim().to_string() };
    settings::update(|s| {
        s.manual_sources.retain(|m| m.url != url);
        s.manual_sources.push(ManualSource { name, url, secret: secret.trim().to_string() });
    })?;
    monitor.rediscover();
    Ok(())
}

#[tauri::command]
pub fn remove_manual_source(monitor: Mon, url: String) -> Result<(), String> {
    settings::update(|s| s.manual_sources.retain(|m| m.url != url))?;
    monitor.rediscover();
    Ok(())
}

#[tauri::command]
pub fn overview(monitor: Mon, source: Option<i64>) -> Result<Overview, String> {
    monitor.overview(source, settings::load().week_starts_sunday)
}

#[tauri::command]
pub fn live(monitor: Mon, source: Option<i64>) -> Vec<LivePoint> {
    monitor.live(source)
}

#[tauri::command]
pub async fn release_notes(after: String, upto: String) -> Result<Vec<ReleaseNote>, String> {
    releases::notes(&after, &upto).await
}

#[tauri::command]
pub fn open_release_page(app: AppHandle, version: String) -> Result<(), String> {
    let (a, b, c) = releases::parse(&version).ok_or("版本号无效。")?;
    let url = format!("https://github.com/{}/releases/tag/v{a}.{b}.{c}", releases::REPO);
    app.opener().open_url(url, None::<&str>).map_err(|e| format!("无法打开发布页面：{e}"))
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    app.opener().open_path(paths::data_dir().display().to_string(), None::<&str>).map_err(|e| format!("无法打开数据目录：{e}"))
}

/// Relaunches after an update was installed; the traffic still in memory is written first.
#[tauri::command]
pub fn restart_app(app: AppHandle, monitor: Mon) {
    let _ = monitor.flush();
    app.restart();
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
