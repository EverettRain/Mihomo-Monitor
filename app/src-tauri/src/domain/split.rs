//! Splitting traffic by where it went: outbound node, route (through a proxy or not),
//! subscription, process, inbound, host.
//!
//! mihomo lists the open connections with their own byte counters. Between two snapshots a
//! connection's growth is attributed to its dimensions. Connections that closed in between take
//! their last bytes with them, so the core's total grows by more than the connections we saw;
//! that remainder is "unattributed" (the empty key). The outbound and route splits always add up
//! to the total; the other dimensions keep only what they know, and the views derive their
//! remainder.
//! A connection without a value for a dimension is kept under that dimension's empty key: for
//! processes, that is traffic the core saw but could not tie to a program, apart from the
//! remainder no connection accounts for.

use super::attribution::Attributor;
use super::counter::Bytes;
use std::collections::HashMap;

/// Kinds of dimension, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DimKind {
    Outbound,
    /// Through a proxy or not: keys `proxy` and `direct`.
    Route,
    Subscription,
    Process,
    Inbound,
}

impl DimKind {
    pub const ALL: [DimKind; 5] = [DimKind::Outbound, DimKind::Route, DimKind::Subscription, DimKind::Process, DimKind::Inbound];

    pub fn as_str(self) -> &'static str {
        match self {
            DimKind::Outbound => "outbound",
            DimKind::Route => "route",
            DimKind::Subscription => "subscription",
            DimKind::Process => "process",
            DimKind::Inbound => "inbound",
        }
    }
}

/// Subscription key of traffic that went out directly, not through any subscription's node.
pub const DIRECT: &str = "DIRECT";

/// Whether traffic went out through a proxy. Traffic no connection accounts for has no route
/// (the empty key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Proxy,
    /// Not through a proxy: direct, or rejected.
    Direct,
}

impl Route {
    pub fn key(self) -> &'static str {
        match self {
            Route::Proxy => "proxy",
            Route::Direct => "direct",
        }
    }
}

/// The type of each outbound the core has, by name, as `/proxies` reports it.
pub type OutboundTypes = HashMap<String, String>;

/// One connection as listed by `/connections`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conn {
    pub id: String,
    pub bytes: Bytes,
    /// Outbound chain, actual node first (`chains`).
    pub chains: Vec<String>,
    /// Provider of each hop where it came from a proxy-provider (`providerChains`).
    pub provider_chains: Vec<String>,
    pub process: String,
    pub inbound: String,
    /// `metadata.host`, else the destination IP.
    pub host: String,
}

/// Where one connection's traffic is counted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dims {
    pub outbound: String,
    /// The route's key; empty when the connection has no outbound.
    pub route: String,
    pub subscription: String,
    pub process: String,
    pub inbound: String,
    pub host: String,
}

/// mihomo's built-in outbounds that do not go through a proxy.
fn builtin_direct(outbound: &str) -> bool {
    outbound == "DIRECT" || outbound.starts_with("REJECT") || outbound == "PASS"
}

/// Outbound types that do not go through a proxy.
fn direct_type(kind: &str) -> bool {
    matches!(kind, "Direct" | "Reject" | "RejectDrop" | "Pass" | "Compatible" | "Dns")
}

/// The route of traffic out through `outbound`: by the outbound's type when the core reported
/// it, which also catches direct outbounds of the user's own (`type: direct` with a name of
/// their choosing); else by mihomo's built-in names.
pub fn route_of(outbound: &str, types: &OutboundTypes) -> Route {
    let direct = match types.get(outbound) {
        Some(kind) => direct_type(kind),
        None => builtin_direct(outbound),
    };
    if direct {
        Route::Direct
    } else {
        Route::Proxy
    }
}

/// The dimensions of a connection. Its subscription is the proxy-provider of its node when it
/// has one; otherwise `client_subscription`, the profile the client had active (clients such as
/// Clash Verge Rev expand a subscription into plain `proxies`, so the core never sees it). Its
/// process is the core's, or, when the core has none, what the attribution rules name it.
pub fn dims_of(c: &Conn, client_subscription: Option<&str>, rules: &Attributor, types: &OutboundTypes) -> Dims {
    let outbound = c.chains.first().cloned().unwrap_or_default();
    let route = (!outbound.is_empty()).then(|| route_of(&outbound, types));
    let subscription = match route {
        None => String::new(),
        Some(Route::Direct) => DIRECT.to_string(),
        Some(Route::Proxy) => c
            .provider_chains
            .iter()
            .find(|p| !p.is_empty())
            .cloned()
            .or_else(|| client_subscription.map(str::to_string))
            .unwrap_or_default(),
    };
    Dims {
        outbound,
        route: route.map_or("", Route::key).to_string(),
        subscription,
        process: rules.process_of(c).to_string(),
        inbound: c.inbound.clone(),
        host: c.host.clone(),
    }
}

/// Remembers each open connection's counters between snapshots.
#[derive(Debug, Default)]
pub struct Tracker {
    seen: HashMap<String, Bytes>,
    started: bool,
}

impl Tracker {
    /// Growth of every connection since the previous snapshot. The first snapshot only sets the
    /// baseline: what its connections carried before is not ours to place in time. A connection
    /// new since the previous snapshot opened in between, so all its bytes are new.
    pub fn step<'a>(&mut self, conns: &'a [Conn]) -> Vec<(&'a Conn, Bytes)> {
        let mut out = Vec::new();
        let mut next = HashMap::with_capacity(conns.len());
        for c in conns {
            let delta = match self.seen.get(&c.id) {
                Some(prev) => super::counter::step(*prev, c.bytes),
                None if self.started => c.bytes,
                None => Bytes::ZERO,
            };
            if !delta.is_zero() {
                out.push((c, delta));
            }
            next.insert(c.id.clone(), c.bytes);
        }
        self.seen = next;
        self.started = true;
        out
    }
}

/// Traffic of one interval, by dimension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Per kind and key. Outbound includes the unattributed remainder under "".
    pub dims: HashMap<(DimKind, String), Bytes>,
    /// Per (process, host).
    pub hosts: HashMap<(String, String), Bytes>,
}

fn sub(a: u64, b: u64) -> u64 {
    a.saturating_sub(b)
}

/// Splits `total`, the core's growth over the interval, by the connections' growth.
pub fn split(total: Bytes, deltas: &[(Dims, Bytes)]) -> Split {
    let mut s = Split::default();
    let mut seen = Bytes::ZERO;
    for (d, b) in deltas {
        seen += *b;
        for (kind, key) in [
            (DimKind::Outbound, &d.outbound),
            (DimKind::Route, &d.route),
            (DimKind::Subscription, &d.subscription),
            (DimKind::Process, &d.process),
            (DimKind::Inbound, &d.inbound),
        ] {
            *s.dims.entry((kind, key.clone())).or_default() += *b;
        }
        *s.hosts.entry((d.process.clone(), d.host.clone())).or_default() += *b;
    }
    let rest = Bytes::new(sub(total.up, seen.up), sub(total.down, seen.down));
    if !rest.is_zero() {
        *s.dims.entry((DimKind::Outbound, String::new())).or_default() += rest;
        *s.dims.entry((DimKind::Route, String::new())).or_default() += rest;
    }
    s.dims.retain(|_, v| !v.is_zero());
    s
}

/// The registrable domain of a host (`a.cdn.example.co.uk` → `example.co.uk`); IP addresses and
/// names without a public suffix stay as they are.
pub fn registrable(host: &str) -> &str {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    psl::domain_str(host).unwrap_or(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, up: u64, down: u64, chains: &[&str]) -> Conn {
        Conn {
            id: id.into(),
            bytes: Bytes::new(up, down),
            chains: chains.iter().map(|s| s.to_string()).collect(),
            provider_chains: vec![String::new(); chains.len()],
            process: "Safari".into(),
            inbound: "DEFAULT-MIXED".into(),
            host: "example.com".into(),
        }
    }

    #[test]
    fn the_actual_node_is_the_first_hop() {
        // Arrange: as seen on Clash Verge Rev 2.5.6.
        let c = conn("1", 0, 0, &["🇺🇸 美国 xtls-reality", "US-Global", "US-Workstation"]);

        // Act
        let d = dims_of(&c, Some("白月光"), &Attributor::default(), &OutboundTypes::new());

        // Assert
        assert_eq!(d.outbound, "🇺🇸 美国 xtls-reality");
        assert_eq!(d.subscription, "白月光");
    }

    #[test]
    fn a_provider_names_the_subscription_before_the_client_profile() {
        // Arrange
        let mut c = conn("1", 0, 0, &["HK 01", "Auto"]);
        c.provider_chains = vec!["机场A".into(), String::new()];

        // Act
        let d = dims_of(&c, Some("白月光"), &Attributor::default(), &OutboundTypes::new());

        // Assert
        assert_eq!(d.subscription, "机场A");
    }

    #[test]
    fn direct_traffic_belongs_to_no_subscription() {
        // Act
        let d = [&["DIRECT"][..], &["REJECT-DROP"], &[]]
            .map(|chains| dims_of(&conn("1", 0, 0, chains), Some("白月光"), &Attributor::default(), &OutboundTypes::new()).subscription);

        // Assert
        assert_eq!(d, [DIRECT.to_string(), DIRECT.to_string(), String::new()]);
    }

    #[test]
    fn the_first_snapshot_is_a_baseline_and_new_connections_count_in_full() {
        // Arrange
        let mut t = Tracker::default();
        t.step(&[conn("a", 100, 1_000, &["DIRECT"])]);

        // Act
        let next = [conn("a", 150, 1_500, &["DIRECT"]), conn("b", 7, 70, &["DIRECT"])];
        let stepped = t.step(&next);
        let got: Vec<(String, Bytes)> = stepped.into_iter().map(|(c, b)| (c.id.clone(), b)).collect();

        // Assert
        assert_eq!(got, vec![("a".into(), Bytes::new(50, 500)), ("b".into(), Bytes::new(7, 70))]);
    }

    #[test]
    fn closed_connections_are_forgotten() {
        // Arrange
        let mut t = Tracker::default();
        t.step(&[conn("a", 1, 1, &[])]);
        t.step(&[]);
        let back = [conn("a", 5, 5, &[])];

        // Act: an id seen before it closed is a new connection if it comes back.
        let got = t.step(&back);

        // Assert
        assert_eq!(got[0].1, Bytes::new(5, 5));
    }

    #[test]
    fn the_outbound_split_adds_up_to_the_total() {
        // Arrange
        let hk = dims_of(&conn("1", 0, 0, &["HK", "Proxy"]), Some("白月光"), &Attributor::default(), &OutboundTypes::new());
        let direct = dims_of(&conn("2", 0, 0, &["DIRECT"]), Some("白月光"), &Attributor::default(), &OutboundTypes::new());

        // Act
        let s = split(Bytes::new(100, 1_000), &[(hk, Bytes::new(30, 600)), (direct, Bytes::new(10, 100))]);

        // Assert
        let outbound: Bytes = s.dims.iter().filter(|((k, _), _)| *k == DimKind::Outbound).fold(Bytes::ZERO, |a, (_, v)| a + *v);
        assert_eq!(outbound, Bytes::new(100, 1_000));
        assert_eq!(s.dims[&(DimKind::Outbound, String::new())], Bytes::new(60, 300));
        assert_eq!(s.dims[&(DimKind::Subscription, "白月光".into())], Bytes::new(30, 600));
        assert_eq!(s.dims[&(DimKind::Subscription, DIRECT.into())], Bytes::new(10, 100));
        assert_eq!(s.hosts[&("Safari".into(), "example.com".into())], Bytes::new(40, 700));
        assert!(!s.dims.contains_key(&(DimKind::Process, String::new())));
    }

    #[test]
    fn naming_traffic_moves_it_between_processes_without_adding_any() {
        // Arrange: one connection the core named, two it did not, one of them through the
        // inbound a rule names.
        use crate::domain::attribution::{AttributionRule, Field};
        let rules = Attributor::new([AttributionRule {
            id: Some(1),
            name: "Container: us-workstation".into(),
            field: Field::Inbound,
            value: "US-Workstation-Port".into(),
            priority: 10,
            enabled: true,
        }]);
        let conns = [
            Conn { process: "Safari".into(), inbound: "US-Workstation-Port".into(), ..conn("1", 0, 0, &["HK"]) },
            Conn { process: String::new(), inbound: "US-Workstation-Port".into(), ..conn("2", 0, 0, &["HK"]) },
            Conn { process: String::new(), inbound: "DEFAULT-MIXED".into(), ..conn("3", 0, 0, &["DIRECT"]) },
        ];
        let grown = [Bytes::new(10, 100), Bytes::new(20, 300), Bytes::new(5, 50)];
        let total = Bytes::new(40, 500);
        let by = |rules: &Attributor| -> Vec<(Dims, Bytes)> {
            conns.iter().zip(grown).map(|(c, b)| (dims_of(c, None, rules, &OutboundTypes::new()), b)).collect()
        };
        let sum = |s: &Split, kind: DimKind| s.dims.iter().filter(|((k, _), _)| *k == kind).fold(Bytes::ZERO, |a, (_, v)| a + *v);

        // Act
        let plain = split(total, &by(&Attributor::default()));
        let named = split(total, &by(&rules));

        // Assert
        assert_eq!(named.dims[&(DimKind::Process, "Container: us-workstation".into())], Bytes::new(20, 300));
        assert_eq!(named.dims[&(DimKind::Process, "Safari".into())], Bytes::new(10, 100));
        assert_eq!(named.dims[&(DimKind::Process, String::new())], Bytes::new(5, 50));
        assert_eq!(plain.dims[&(DimKind::Process, String::new())], Bytes::new(25, 350));
        for kind in DimKind::ALL {
            assert_eq!(sum(&named, kind), sum(&plain, kind), "{kind:?}");
        }
        assert_eq!(sum(&named, DimKind::Outbound), total);
        let hosts = |s: &Split| s.hosts.values().fold(Bytes::ZERO, |a, v| a + *v);
        assert_eq!(hosts(&named), hosts(&plain));
        assert_eq!(named.hosts[&("Container: us-workstation".into(), "example.com".into())], Bytes::new(20, 300));
    }

    #[test]
    fn the_core_reported_type_decides_the_route() {
        // Arrange: a direct outbound of the user's own, a built-in one, and a node, with and
        // without the types from `/proxies`.
        let types: OutboundTypes = [("直连-以太网", "Direct"), ("HK 01", "Shadowsocks"), ("广告拦截", "Reject")]
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .into();
        let names = ["直连-以太网", "DIRECT", "REJECT-DROP", "HK 01", "广告拦截"];

        // Act
        let known = names.map(|n| route_of(n, &types));
        let unknown = names.map(|n| route_of(n, &OutboundTypes::new()));

        // Assert
        assert_eq!(known, [Route::Direct, Route::Direct, Route::Direct, Route::Proxy, Route::Direct]);
        assert_eq!(unknown, [Route::Proxy, Route::Direct, Route::Direct, Route::Proxy, Route::Proxy]);
    }

    #[test]
    fn a_direct_outbound_of_the_users_own_belongs_to_no_subscription() {
        // Arrange
        let types: OutboundTypes = [("直连-以太网".to_string(), "Direct".to_string())].into();
        let c = conn("1", 0, 0, &["直连-以太网"]);

        // Act
        let d = dims_of(&c, Some("白月光"), &Attributor::default(), &types);

        // Assert
        assert_eq!((d.route.as_str(), d.subscription.as_str()), ("direct", DIRECT));
    }

    #[test]
    fn the_route_split_adds_up_to_the_total_like_the_outbound_one() {
        // Arrange
        let none = OutboundTypes::new();
        let proxied = dims_of(&conn("1", 0, 0, &["HK", "Proxy"]), None, &Attributor::default(), &none);
        let direct = dims_of(&conn("2", 0, 0, &["DIRECT"]), None, &Attributor::default(), &none);
        let total = Bytes::new(100, 1_000);

        // Act
        let s = split(total, &[(proxied, Bytes::new(30, 600)), (direct, Bytes::new(10, 100))]);

        // Assert
        let route: Bytes = s.dims.iter().filter(|((k, _), _)| *k == DimKind::Route).fold(Bytes::ZERO, |a, (_, v)| a + *v);
        assert_eq!(route, total);
        assert_eq!(s.dims[&(DimKind::Route, "proxy".into())], Bytes::new(30, 600));
        assert_eq!(s.dims[&(DimKind::Route, "direct".into())], Bytes::new(10, 100));
        assert_eq!(s.dims[&(DimKind::Route, String::new())], Bytes::new(60, 300));
    }

    #[test]
    fn registrable_domains_merge_subdomains_but_not_addresses() {
        // Act
        let got = ["a.cdn.example.co.uk", "www.google.com", "localhost", "203.0.113.5", "2001:db8::1"].map(registrable);

        // Assert
        assert_eq!(got, ["example.co.uk", "google.com", "localhost", "203.0.113.5", "2001:db8::1"]);
    }
}
