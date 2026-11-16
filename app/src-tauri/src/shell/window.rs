//! The main window: created when asked for, destroyed or hidden when closed.
//!
//! With the window gone only the Rust process remains, so the app costs almost nothing while it
//! sits in the tray. On macOS the app leaves the Dock and the app switcher while no window is
//! shown, as tray utilities do.

use crate::infra::settings;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent};

pub const MAIN: &str = "main";

pub fn show_main(app: &AppHandle) {
    in_dock(app, true);
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == MAIN).cloned() else { return };
    let Ok(builder) = WebviewWindowBuilder::from_config(app, &cfg) else { return };
    #[cfg(target_os = "macos")]
    let builder = builder
        .decorations(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        // Vertically centre the traffic lights on the 48px sidebar header row.
        .traffic_light_position(tauri::LogicalPosition::new(18.0, 22.0));
    if let Ok(w) = builder.build() {
        fit_to_screen(&w);
        // The page shows the window once it has painted; if that never comes (a broken
        // WebView), show it anyway rather than leave an invisible window behind.
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            if let Some(w) = app.get_webview_window(MAIN) {
                if !w.is_visible().unwrap_or(true) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
    }
}

fn fit_to_screen(w: &WebviewWindow) {
    let (Ok(Some(mon)), Ok(size)) = (w.current_monitor(), w.outer_size()) else { return };
    let area = mon.work_area().size;
    let max_w = (area.width as f64 * 0.9) as u32;
    let max_h = (area.height as f64 * 0.9) as u32;
    if size.width > max_w || size.height > max_h {
        let _ = w.set_size(tauri::PhysicalSize::new(size.width.min(max_w), size.height.min(max_h)));
        let _ = w.center();
    }
}

/// Lightweight mode lets the close destroy the window; otherwise it is only hidden, which
/// reopens instantly but keeps the WebView in memory.
pub fn on_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    match event {
        WindowEvent::CloseRequested { api, .. } if !settings::load().lightweight => {
            api.prevent_close();
            let _ = window.hide();
            in_dock(window.app_handle(), false);
        }
        WindowEvent::Destroyed => in_dock(window.app_handle(), false),
        _ => {}
    }
}

pub fn main_window_open(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN).is_some_and(|w| w.is_visible().unwrap_or(false))
}

#[cfg(target_os = "macos")]
pub fn in_dock(app: &AppHandle, shown: bool) {
    let policy = if shown { tauri::ActivationPolicy::Regular } else { tauri::ActivationPolicy::Accessory };
    let _ = app.set_activation_policy(policy);
}

#[cfg(not(target_os = "macos"))]
pub fn in_dock(_app: &AppHandle, _shown: bool) {}
