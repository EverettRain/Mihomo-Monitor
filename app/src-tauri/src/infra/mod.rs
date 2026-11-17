//! Adapters to the outside world: storage, settings, the mihomo API, the process list.

pub mod discovery;
pub mod import;
pub mod mihomo;
pub mod paths;
pub mod profiles;
pub mod releases;
pub mod rules_file;
pub mod settings;
pub mod store;
pub mod verge;

/// reqwest is built with rustls but no bundled crypto provider (the updater plugin's choice);
/// every client needs one installed first. Installing twice is harmless.
pub fn install_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
