//! Turning running processes, known client locations and the user's manual entries into cores
//! we can actually read, each verified with `GET /version`.

use crate::domain::counter::CoreId;
use crate::infra::discovery::{self, LocalCore};
use crate::infra::mihomo::{ApiError, Client, Endpoint};
use crate::infra::settings::ManualSource;

/// A core that answered as mihomo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Stable identity of the source across restarts and controller moves.
    pub key: String,
    pub name: String,
    pub endpoint: Endpoint,
    /// The process, for local cores; tells a restart apart from a reconnect.
    pub core: Option<CoreId>,
    pub version: String,
    pub manual: bool,
}

/// A core we know of but cannot read, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    pub key: String,
    pub name: String,
    pub endpoint: Option<String>,
    pub reason: String,
    pub manual: bool,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub found: Vec<Found>,
    pub unreadable: Vec<Unreadable>,
}

/// Ports clients commonly put their HTTP controller on; only tried when nothing else turned up.
const COMMON_PORTS: [u16; 2] = [9090, 9097];

async fn first_answering(endpoints: &[Endpoint]) -> Result<(Endpoint, String), (Option<String>, String)> {
    let mut last = (None, "没有找到控制接口（external-controller）".to_string());
    for ep in endpoints {
        let attempt = match Client::new(ep) {
            Ok(c) => c.version().await,
            Err(e) => Err(e),
        };
        match attempt {
            Ok(v) => return Ok((ep.clone(), v)),
            Err(e) => last = (Some(ep.describe()), e.to_string()),
        }
    }
    Err(last)
}

/// Cores on this machine (unless `auto` is off) and the manual entries.
pub async fn discover(manual: &[ManualSource], auto: bool) -> Discovery {
    let mut out = Discovery::default();
    if auto {
        on_this_machine(&mut out).await;
    }
    for m in manual {
        let ep = Endpoint::Http { url: m.url.clone(), secret: m.secret.clone() };
        let key = manual_key(&m.url);
        match first_answering(std::slice::from_ref(&ep)).await {
            Ok((endpoint, version)) => out.found.push(Found { key, name: m.name.clone(), endpoint, core: None, version, manual: true }),
            Err((_, reason)) => {
                out.unreadable.push(Unreadable { key, name: m.name.clone(), endpoint: Some(m.url.clone()), reason, manual: true })
            }
        }
    }
    // One entry per source: a manual entry for a core that was also found locally adds nothing.
    let mut seen = std::collections::HashSet::new();
    out.found.retain(|f| seen.insert(f.key.clone()));
    out.unreadable.retain(|u| !seen.contains(&u.key) && seen.insert(u.key.clone()));
    out
}

async fn on_this_machine(out: &mut Discovery) {
    let cores = tokio::task::spawn_blocking(discovery::running_cores).await.unwrap_or_default();

    for core in &cores {
        local(out, core).await;
    }
    // A Verge core whose process we cannot see at all (another user's session, or a process
    // list we may not read) can still be reached at its known locations.
    if !out.found.iter().any(|f| f.key == discovery::VERGE_KEY) && !out.unreadable.iter().any(|u| u.key == discovery::VERGE_KEY) {
        if let Ok((endpoint, version)) = first_answering(&discovery::verge_endpoints()).await {
            out.found.push(Found {
                key: discovery::VERGE_KEY.into(),
                name: "Clash Verge Rev".into(),
                endpoint,
                core: None,
                version,
                manual: false,
            });
        }
    }
    if out.found.is_empty() {
        for port in COMMON_PORTS {
            let url = format!("http://127.0.0.1:{port}");
            let ep = Endpoint::Http { url: url.clone(), secret: String::new() };
            let Ok(client) = Client::new(&ep) else { continue };
            match client.version().await {
                Ok(version) => out.found.push(Found {
                    key: format!("http:{url}"),
                    name: "mihomo".into(),
                    endpoint: ep,
                    core: None,
                    version,
                    manual: false,
                }),
                Err(ApiError::Unauthorized) => out.unreadable.push(Unreadable {
                    key: format!("http:{url}"),
                    name: "mihomo".into(),
                    endpoint: Some(url),
                    reason: "控制接口设置了 secret，请在下方手动添加并填写 secret".into(),
                    manual: false,
                }),
                Err(_) => {}
            }
        }
    }
}

pub fn manual_key(url: &str) -> String {
    format!("manual:{}", url.trim_end_matches('/'))
}

async fn local(out: &mut Discovery, core: &LocalCore) {
    let args = discovery::parse_args(&core.args);
    let (key, name) = discovery::identity(core, &args);
    let mut endpoints = discovery::controller_of(&args).endpoints();
    if discovery::is_verge(core) {
        endpoints.extend(discovery::verge_endpoints().into_iter().filter(|e| !endpoints.contains(e)).collect::<Vec<_>>());
    }
    match first_answering(&endpoints).await {
        Ok((endpoint, version)) => out.found.push(Found { key, name, endpoint, core: Some(core.core.clone()), version, manual: false }),
        Err((endpoint, reason)) => {
            let reason = if core.args.is_empty() && endpoint.is_none() {
                "无法读取内核的启动参数，也没有在常见位置找到它的控制接口；可以在下方手动添加".to_string()
            } else {
                reason
            };
            out.unreadable.push(Unreadable { key, name, endpoint, reason, manual: false });
        }
    }
}
