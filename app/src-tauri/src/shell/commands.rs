//! Tauri commands the interface calls.

use super::tray;
use crate::domain::attribution::AttributionRule;
use crate::domain::hostgroup::RuleKind;
use crate::domain::period::Period;
use crate::domain::split::{DimKind, Route};
use crate::infra::discovery::controller_url;
use crate::infra::import::{Entry, Format, Parsed};
use crate::infra::paths;
use crate::infra::releases::{self, ReleaseNote};
use crate::infra::settings::{self, ManualSource};
use crate::infra::store::{Added, CycleRow, HostGroupRow};
use crate::service::monitor::{
    AddResult, Bounds, HostView, LivePoint, Monitor, Overview, PeriodRow, ProfileSummary, ProfileView, RankRow, SourceView,
    SubscriptionView,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
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
    sample_ms: u64,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> SettingsView {
    let s = settings::load();
    SettingsView {
        lightweight: s.lightweight,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        week_starts_sunday: s.week_starts_sunday,
        auto_discover: s.auto_discover,
        sample_ms: settings::sample_ms(s.sample_ms),
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
pub fn set_sample_interval(monitor: Mon, ms: u64) -> Result<(), String> {
    let ms = settings::sample_ms(ms);
    settings::update(|s| s.sample_ms = ms)?;
    monitor.set_sample_interval(ms);
    Ok(())
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
pub async fn overview(monitor: Mon<'_>, source: Option<i64>, route: Option<Route>) -> Result<Overview, String> {
    off_main(monitor, move |m| m.overview(source, route, settings::load().week_starts_sunday)).await
}

/// Runs a query off the main thread. Commands that are not `async` run on it, so a long query
/// would freeze the window, the click that asked for it included.
async fn off_main<T: Send + 'static>(
    monitor: Mon<'_>,
    query: impl FnOnce(&Monitor) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let monitor = monitor.inner().clone();
    tauri::async_runtime::spawn_blocking(move || query(&monitor)).await.map_err(|e| e.to_string())?
}

fn date(s: &str) -> Result<jiff::civil::Date, String> {
    s.parse().map_err(|_| format!("日期格式无效：{s}"))
}

#[tauri::command]
pub async fn history(
    monitor: Mon<'_>,
    unit: Period,
    from: String,
    to: String,
    source: Option<i64>,
    route: Option<Route>,
) -> Result<Vec<PeriodRow>, String> {
    off_main(monitor, move |m| m.history(unit, date(&from)?, date(&to)?, source, route, settings::load().week_starts_sunday)).await
}

#[tauri::command]
pub async fn ranking(
    monitor: Mon<'_>,
    kind: DimKind,
    from: String,
    to: String,
    source: Option<i64>,
    route: Option<Route>,
) -> Result<Vec<RankRow>, String> {
    off_main(monitor, move |m| m.ranking(kind, date(&from)?, date(&to)?, source, route)).await
}

/// Traffic by route over the last `minutes`.
#[tauri::command]
pub async fn recent_routes(monitor: Mon<'_>, source: Option<i64>, minutes: i64) -> Result<Vec<RankRow>, String> {
    off_main(monitor, move |m| m.recent_routes(source, minutes.clamp(15, 24 * 60), paths::now_secs())).await
}

#[tauri::command]
pub async fn routes(monitor: Mon<'_>, from: String, to: String, source: Option<i64>) -> Result<Vec<RankRow>, String> {
    off_main(monitor, move |m| m.routes(date(&from)?, date(&to)?, source)).await
}

/// Hosts most traffic went to; `process` narrows them to one process, `group` to one host group.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn hosts(
    monitor: Mon<'_>,
    from: String,
    to: String,
    source: Option<i64>,
    route: Option<Route>,
    process: Option<String>,
    group: Option<String>,
    view: HostView,
) -> Result<Vec<RankRow>, String> {
    off_main(monitor, move |m| m.hosts(date(&from)?, date(&to)?, source, route, process.as_deref(), group.as_deref(), view, 100)).await
}

#[tauri::command]
pub async fn attribution_rules(monitor: Mon<'_>) -> Result<Vec<AttributionRule>, String> {
    off_main(monitor, move |m| m.attribution_rules()).await
}

#[tauri::command]
pub fn save_attribution_rule(monitor: Mon, rule: AttributionRule) -> Result<i64, String> {
    monitor.save_attribution_rule(rule)
}

#[tauri::command]
pub fn delete_attribution_rule(monitor: Mon, id: i64) -> Result<(), String> {
    monitor.delete_attribution_rule(id)
}

#[tauri::command]
pub async fn host_groups(monitor: Mon<'_>) -> Result<Vec<HostGroupRow>, String> {
    off_main(monitor, move |m| m.host_groups()).await
}

#[tauri::command]
pub fn create_group(monitor: Mon, name: String, note: String) -> Result<i64, String> {
    monitor.create_group(&name, &note)
}

#[tauri::command]
pub fn update_group(monitor: Mon, id: i64, name: String, note: String) -> Result<(), String> {
    monitor.update_group(id, &name, &note)
}

#[tauri::command]
pub fn delete_group(monitor: Mon, id: i64) -> Result<(), String> {
    monitor.delete_group(id)
}

#[tauri::command]
pub fn add_rules(monitor: Mon, group: i64, text: String) -> Result<AddResult, String> {
    monitor.add_rules(group, &text)
}

#[tauri::command]
pub fn update_rule(monitor: Mon, id: i64, kind: RuleKind, value: String) -> Result<(), String> {
    monitor.update_rule(id, kind, &value)
}

#[tauri::command]
pub fn delete_rules(monitor: Mon, ids: Vec<i64>) -> Result<(), String> {
    monitor.delete_rules(&ids)
}

#[tauri::command]
pub fn import_preview(monitor: Mon, text: String, format: Format) -> Parsed {
    monitor.import_preview(&text, format)
}

#[tauri::command]
pub fn import_apply(monitor: Mon, entries: Vec<Entry>, default_group: String) -> Result<Added, String> {
    monitor.import_apply(entries, &default_group)
}

/// The rule configuration in use, read again when its file was edited outside the app.
#[tauri::command]
pub async fn rules_profile(monitor: Mon<'_>) -> Result<ProfileView, String> {
    off_main(monitor, |m| m.sync_profile()).await
}

#[tauri::command]
pub async fn rules_profiles(monitor: Mon<'_>) -> Result<Vec<ProfileSummary>, String> {
    off_main(monitor, |m| m.profiles()).await
}

#[tauri::command]
pub fn switch_rules_profile(monitor: Mon, name: String) -> Result<(), String> {
    let name = monitor.switch_profile(&name)?;
    settings::update(|s| s.rules_profile = name).map(|_| ())
}

/// Adds a configuration, empty or as a copy of the one in use.
#[tauri::command]
pub fn create_rules_profile(monitor: Mon, name: String, copy: bool) -> Result<String, String> {
    monitor.create_profile(&name, copy)
}

/// Adds a configuration from the text of a rules file.
#[tauri::command]
pub fn import_rules_profile(monitor: Mon, name: String, text: String) -> Result<String, String> {
    monitor.import_profile(&name, &text)
}

#[tauri::command]
pub fn delete_rules_profile(monitor: Mon, name: String) -> Result<(), String> {
    monitor.delete_profile(&name)
}

/// Opens the configuration in use with the system's editor for it.
#[tauri::command]
pub fn open_rules_profile(app: AppHandle, monitor: Mon) -> Result<(), String> {
    let path = monitor.profile_path().ok_or("配置还没有打开。")?;
    app.opener().open_path(path.display().to_string(), None::<&str>).map_err(|e| format!("无法打开配置文件：{e}"))
}

/// Shows the configuration in use in Finder or Explorer.
#[tauri::command]
pub fn reveal_rules_profile(app: AppHandle, monitor: Mon) -> Result<(), String> {
    let path = monitor.profile_path().ok_or("配置还没有打开。")?;
    app.opener().reveal_item_in_dir(path).map_err(|e| format!("无法显示配置文件：{e}"))
}

#[tauri::command]
pub async fn subscriptions(monitor: Mon<'_>) -> Result<Vec<SubscriptionView>, String> {
    off_main(monitor, move |m| m.subscriptions(paths::now_secs())).await
}

#[tauri::command]
pub fn save_cycle(monitor: Mon, cycle: CycleRow) -> Result<i64, String> {
    monitor.save_cycle(cycle)
}

#[tauri::command]
pub fn delete_cycle(monitor: Mon, id: i64) -> Result<(), String> {
    monitor.delete_cycle(id)
}

#[tauri::command]
pub async fn data_bounds(monitor: Mon<'_>, source: Option<i64>) -> Result<Option<Bounds>, String> {
    off_main(monitor, move |m| m.bounds(source)).await
}

#[tauri::command]
pub async fn top_days(monitor: Mon<'_>, source: Option<i64>, route: Option<Route>, limit: usize) -> Result<Vec<PeriodRow>, String> {
    off_main(monitor, move |m| m.top_days(source, route, limit.min(100))).await
}

/// Asks where to save and writes the text there, as CSV or JSON. The page hands over only the
/// text; the path comes from the save dialog, so no page code can write elsewhere. `false` when
/// cancelled.
#[tauri::command]
pub async fn export_file(app: AppHandle, name: String, content: String, kind: String) -> Result<bool, String> {
    let csv = kind == "csv";
    tauri::async_runtime::spawn_blocking(move || {
        let (label, ext) = match kind.as_str() {
            "csv" => ("CSV", "csv"),
            "yaml" => ("YAML", "yaml"),
            _ => ("JSON", "json"),
        };
        let Some(path) = app.dialog().file().set_file_name(&name).add_filter(label, &[ext]).blocking_save_file() else {
            return Ok(false);
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        // With a byte-order mark, Excel opens UTF-8 CSV with Chinese text correctly.
        let body = if csv { format!("\u{feff}{content}") } else { content };
        std::fs::write(&path, body).map_err(|e| format!("无法保存：{e}"))?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
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
pub fn restart_app(app: AppHandle) {
    // Not running when the data comes from a later version (see `newer_data`).
    if let Some(monitor) = app.try_state::<Arc<Monitor>>() {
        let _ = monitor.flush();
    }
    app.restart();
}

/// Set when the data on disk comes from a later version: then the app only offers to update,
/// and no other command may be used.
#[tauri::command]
pub fn newer_data(app: AppHandle) -> Option<super::NewerData> {
    app.try_state::<super::NewerData>().map(|s| *s)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestNotification {
    /// A development build: on macOS its notifications go out as Terminal's.
    dev: bool,
}

/// Sends a notification right away, to check that alerts can be seen at all.
#[tauri::command]
pub fn test_notification(app: AppHandle) -> Result<TestNotification, String> {
    super::send_notification(&app, "Mihomo-Monitor 测试通知", "能看到这条通知，配额提醒就能正常弹出。")
        .map_err(|e| format!("发送失败：{e}"))?;
    Ok(TestNotification { dev: tauri::is_dev() })
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
