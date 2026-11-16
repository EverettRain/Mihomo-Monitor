#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let start_in_tray = std::env::args().skip(1).any(|a| a == "--tray");
    mihomo_monitor_lib::run(start_in_tray);
}
