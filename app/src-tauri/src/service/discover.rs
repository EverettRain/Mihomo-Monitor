//! Turning running processes, known client locations and the user's manual entries into cores
//! we can actually read, each verified with `GET /version`.

use crate::domain::counter::CoreId;
use crate::infra::discovery::{self, Client as Known, LocalCore};
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

const NO_CONTROLLER: &str = "没有找到控制接口（external-controller）";

/// FlClash keeps its core's socket and pipe off and opens the HTTP controller only with a secret.
const FLCLASH_CONTROLLER: &str = "http://127.0.0.1:9090";

/// The first endpoint that answers as mihomo, or the last one tried and why it did not.
async fn first_answering(endpoints: &[Endpoint]) -> Result<(Endpoint, String), Option<(Endpoint, ApiError)>> {
    let mut last = None;
    for ep in endpoints {
        let attempt = match Client::new(ep) {
            Ok(c) => c.version().await,
            Err(e) => Err(e),
        };
        match attempt {
            Ok(v) => return Ok((ep.clone(), v)),
            Err(e) => last = Some((ep.clone(), e)),
        }
    }
    Err(last)
}

fn reason_of(last: &Option<(Endpoint, ApiError)>) -> (Option<String>, String) {
    match last {
        Some((ep, e)) => (Some(ep.describe()), e.to_string()),
        None => (None, NO_CONTROLLER.into()),
    }
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
            Err(last) => {
                let reason = reason_of(&last).1;
                out.unreadable.push(Unreadable { key, name: m.name.clone(), endpoint: Some(m.url.clone()), reason, manual: true })
            }
        }
    }
    // One entry per source: a manual entry for a core that was also found locally adds nothing,
    // and a local core the user reached by adding its controller by hand is no longer missing.
    let mut seen = std::collections::HashSet::new();
    out.found.retain(|f| seen.insert(f.key.clone()));
    let reached: Vec<String> = out.found.iter().map(|f| f.endpoint.describe()).collect();
    out.unreadable
        .retain(|u| !seen.contains(&u.key) && !u.endpoint.as_ref().is_some_and(|e| reached.contains(e)) && seen.insert(u.key.clone()));
    out
}

async fn on_this_machine(out: &mut Discovery) {
    let cores = tokio::task::spawn_blocking(discovery::running_cores).await.unwrap_or_default();

    let mut anonymous = Vec::new();
    for core in &cores {
        if let Some(key) = local(out, core).await {
            anonymous.push(key);
        }
    }
    // A client's core whose process we cannot see at all (another user's session, or a process
    // list we may not read) can still be reached where the client puts its controller.
    let mut reached_blind = false;
    for client in Known::ALL {
        if out.found.iter().map(|f| &f.key).chain(out.unreadable.iter().map(|u| &u.key)).any(|k| k == client.key()) {
            continue;
        }
        if let Ok((endpoint, version)) = first_answering(&client.known_endpoints()).await {
            reached_blind = true;
            out.found.push(Found { key: client.key().into(), name: client.name().into(), endpoint, core: None, version, manual: false });
        }
    }
    // Those are most likely the cores we saw but could tell nothing about.
    if reached_blind {
        out.unreadable.retain(|u| !anonymous.contains(&u.key));
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

/// Reads one core seen in the process list. Returns its key when it could not be read and
/// nothing about it says which client it belongs to.
async fn local(out: &mut Discovery, core: &LocalCore) -> Option<String> {
    let args = discovery::parse_args(&core.args);
    let client = Known::of(core, &args);
    let (key, name) = discovery::identity(core, &args);
    let mut endpoints = discovery::controller_of(&args).endpoints();
    let mut more = client.map(Known::known_endpoints).unwrap_or_default();
    if client == Some(Known::FlClash) {
        more.push(Endpoint::Http { url: FLCLASH_CONTROLLER.into(), secret: String::new() });
    }
    endpoints.extend(more.into_iter().filter(|e| !endpoints.contains(e)).collect::<Vec<_>>());
    match first_answering(&endpoints).await {
        Ok((endpoint, version)) => {
            out.found.push(Found { key, name, endpoint, core: Some(core.core.clone()), version, manual: false });
            None
        }
        Err(last) => {
            let (endpoint, reason) = reason_of(&last);
            let reason = match (client, &last) {
                (Some(Known::FlClash), Some((_, ApiError::Unauthorized))) => {
                    format!("FlClash 的外部控制器设置了密钥：请在下方手动添加 {FLCLASH_CONTROLLER} 并填写这个密钥")
                }
                (Some(Known::FlClash), _) => {
                    format!("FlClash 默认不开放控制接口：在 FlClash 的设置里打开「外部控制器」并设置密钥，然后在下方手动添加 {FLCLASH_CONTROLLER} 和这个密钥")
                }
                _ if core.args.is_empty() && endpoint.is_none() => {
                    "无法读取内核的启动参数，也没有在常见位置找到它的控制接口；可以在下方手动添加".to_string()
                }
                _ => reason,
            };
            let anonymous = (client.is_none() && core.args.is_empty()).then(|| key.clone());
            out.unreadable.push(Unreadable { key, name, endpoint, reason, manual: false });
            anonymous
        }
    }
}
