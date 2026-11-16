//! Adapters to the outside world: storage, settings, the mihomo API, the process list.

pub mod discovery;
pub mod mihomo;
pub mod paths;
pub mod releases;
pub mod settings;
pub mod store;

/// reqwest is built with rustls but no bundled crypto provider (the updater plugin's choice);
/// every client needs one installed first. Installing twice is harmless.
pub fn install_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
