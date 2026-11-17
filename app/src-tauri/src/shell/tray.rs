//! The tray icon: today's traffic on hover, and a small menu.

use super::window::show_main;
use crate::domain::counter::Bytes;
use crate::infra::settings;
use crate::service::monitor::SubscriptionView;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

const TRAY_ID: &str = "main";

/// Binary units, as proxy clients and most airports count: 1 GB = 1024 MB.
pub fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    match i {
        0 => format!("{n} B"),
        _ if v >= 100.0 => format!("{v:.0} {}", UNITS[i]),
        _ => format!("{v:.2} {}", UNITS[i]),
    }
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let m = Menu::new(app)?;
    m.append(&MenuItem::with_id(app, "show", "打开 Mihomo-Monitor", true, None::<&str>)?)?;
    m.append(&CheckMenuItem::with_id(
        app,
        "lightweight",
        "轻量模式（关闭窗口时释放内存）",
        true,
        settings::load().lightweight,
        None::<&str>,
    )?)?;
    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?)?;
    Ok(m)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Mihomo-Monitor")
        .menu(&menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "show" => show_main(app),
            "lightweight" => {
                let _ = settings::update(|s| s.lightweight = !s.lightweight);
                refresh_menu(app);
                let _ = tauri::Emitter::emit(app, "settings-changed", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        });
    // The default window icon is the 32 px one, blurry in a Retina menu bar; the tray gets the
    // 256 px artwork and scales it down itself.
    match tauri::image::Image::from_bytes(include_bytes!("../../icons/128x128@2x.png")) {
        Ok(icon) => tray = tray.icon(icon),
        Err(_) => {
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
        }
    }
    tray.build(app)?;
    Ok(())
}

/// Rebuilds the menu so its check marks follow the settings.
pub fn refresh_menu(app: &AppHandle) {
    if let (Some(tray), Ok(m)) = (app.tray_by_id(TRAY_ID), menu(app)) {
        let _ = tray.set_menu(Some(m));
    }
}

/// Today's traffic, and each billing cycle's use so far.
pub fn show_status(app: &AppHandle, today: Bytes, subscriptions: &[SubscriptionView]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let mut text = format!("Mihomo-Monitor\n今日  ↑ {}  ↓ {}", human(today.up), human(today.down));
    for s in subscriptions {
        let Some(c) = &s.cycle else { continue };
        match c.config.quota {
            Some(q) => text.push_str(&format!("\n{}  本周期 {} / {}", s.name, human(c.used), human(q))),
            None => text.push_str(&format!("\n{}  本周期 {}", s.name, human(c.used))),
        }
    }
    let _ = tray.set_tooltip(Some(text));
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn sizes_use_binary_units() {
        // Act
        let shown = [0, 1023, 1024, 1536, 5 * 1024 * 1024 * 1024, 300 * 1024 * 1024].map(human);

        // Assert
        assert_eq!(shown, ["0 B", "1023 B", "1.00 KB", "1.50 KB", "5.00 GB", "300 MB"]);
    }
}
