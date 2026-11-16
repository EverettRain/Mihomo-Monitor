//! Mihomo-Monitor: vnstat-style traffic statistics for the mihomo core you already run.
//!
//! Layers, inner to outer: `domain` (pure accounting rules), `infra` (database, settings,
//! the mihomo API, process discovery), `service` (the background collector and the queries
//! behind the views), `shell` (Tauri: tray, window, commands).

pub mod domain;
pub mod infra;
pub mod service;
mod shell;

pub use shell::run;
