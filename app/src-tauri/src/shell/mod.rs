//! The Tauri application: plugins, tray, window lifecycle and the background tasks.

mod commands;
mod tray;
mod window;

use crate::infra::{paths, settings, store::Store};
use crate::service::monitor::{Alert, Event, Monitor, FLUSH_EVERY};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt as _;

/// Turns start-at-login on at the first launch: a traffic monitor only counts while it runs.
/// Development builds leave it alone, so a debug binary never ends up in the login items.
fn first_launch_autostart(app: &AppHandle) {
    if cfg!(debug_assertions) || settings::load().autostart_initialized {
        return;
    }
    if let Err(e) = app.autolaunch().enable() {
        paths::log(&format!("无法开启开机自启：{e}"));
    }
    let _ = settings::update(|s| s.autostart_initialized = true);
}

fn start_background(app: &AppHandle, monitor: Arc<Monitor>) {
    tauri::async_runtime::spawn(monitor.clone().supervise());
    tauri::async_runtime::spawn(monitor.clone().flush_forever());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let cycles = monitor.subscriptions(paths::now_secs()).unwrap_or_default();
            tray::show_status(&app, monitor.today(), &cycles);
            tokio::time::sleep(FLUSH_EVERY).await;
        }
    });
}

/// Forwards service events to the window, only while there is one to see them.
fn sink(app: AppHandle) -> impl Fn(Event) + Send + Sync + 'static {
    move |e| match e {
        Event::Sample(p) if window::main_window_open(&app) => {
            let _ = app.emit("traffic-sample", p);
        }
        Event::Sample(_) => {}
        Event::SourcesChanged => {
            let _ = app.emit("sources-changed", ());
        }
        Event::Alert(a) => notify(&app, &a),
    }
}

fn notify(app: &AppHandle, a: &Alert) {
    let resets = jiff::Timestamp::from_second(a.resets_at)
        .map(|t| t.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime("%-m 月 %-d 日 %H:%M").to_string())
        .unwrap_or_default();
    let title = if a.threshold >= 100 {
        format!("{} 本周期流量已用完", a.subscription)
    } else {
        format!("{} 本周期已用 {}%", a.subscription, a.threshold)
    };
    let body = format!("已用 {}，配额 {}，{resets} 重置。", tray::human(a.used), tray::human(a.quota));
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        paths::log(&format!("无法发送通知：{e}"));
    }
}

pub fn run(start_in_tray: bool) {
    tauri::Builder::default()
        // Must come first: a second launch only brings up the running instance's window
        // (`--tray` launches come from start-at-login and want nothing shown).
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == "--tray") {
                window::show_main(app);
            }
        }))
        // On macOS the plugin registers a LaunchAgent (its default).
        .plugin(tauri_plugin_autostart::Builder::new().args(["--tray"]).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(window::on_event)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::set_lightweight,
            commands::set_autostart,
            commands::set_week_starts_sunday,
            commands::set_auto_discover,
            commands::list_sources,
            commands::add_manual_source,
            commands::remove_manual_source,
            commands::overview,
            commands::live,
            commands::history,
            commands::ranking,
            commands::hosts,
            commands::data_bounds,
            commands::top_days,
            commands::export_csv,
            commands::subscriptions,
            commands::save_cycle,
            commands::delete_cycle,
            commands::release_notes,
            commands::open_release_page,
            commands::open_data_dir,
            commands::restart_app,
            commands::quit_app,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let store = Store::open(&paths::database_path()).inspect_err(|e| paths::log(e))?;
            let monitor = Monitor::new(store, sink(handle.clone()));
            app.manage(monitor.clone());
            tray::build(&handle)?;
            first_launch_autostart(&handle);
            start_background(&handle, monitor);
            if start_in_tray {
                window::in_dock(&handle, false);
            } else {
                window::show_main(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Mihomo-Monitor")
        .run(|app, event| match event {
            // Clicking the Dock icon or opening the app again from Finder reaches the running
            // instance as a reopen request, not as a second launch.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => window::show_main(app),
            RunEvent::Exit => {
                if let Some(m) = app.try_state::<Arc<Monitor>>() {
                    if let Err(e) = m.flush() {
                        paths::log(&format!("退出时写入数据库失败：{e}"));
                    }
                }
            }
            // Closing the last window keeps the app in the tray; only `exit` (quit) ends it.
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            _ => {}
        });
}
