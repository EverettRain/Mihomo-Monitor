//! Splitting traffic by where it went: outbound node, subscription, process, inbound, host.
//!
//! mihomo lists the open connections with their own byte counters. Between two snapshots a
//! connection's growth is attributed to its dimensions. Connections that closed in between take
//! their last bytes with them, so the core's total grows by more than the connections we saw;
//! that remainder is "unattributed" (the empty key). The outbound split always adds up to the
//! total; the other dimensions keep only what they know, and the views derive their remainder.

use super::counter::Bytes;
use std::collections::HashMap;

/// Kinds of dimension, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DimKind {
    Outbound,
    Subscription,
    Process,
    Inbound,
}

impl DimKind {
    pub const ALL: [DimKind; 4] = [DimKind::Outbound, DimKind::Subscription, DimKind::Process, DimKind::Inbound];

    pub fn as_str(self) -> &'static str {
        match self {
            DimKind::Outbound => "outbound",
            DimKind::Subscription => "subscription",
            DimKind::Process => "process",
            DimKind::Inbound => "inbound",
        }
    }
}

/// Subscription key of traffic that went out directly, not through any subscription's node.
pub const DIRECT: &str = "DIRECT";

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
    pub subscription: String,
    pub process: String,
    pub inbound: String,
    pub host: String,
}

fn is_direct(outbound: &str) -> bool {
    outbound == "DIRECT" || outbound.starts_with("REJECT") || outbound == "PASS"
}

/// The dimensions of a connection. Its subscription is the proxy-provider of its node when it
/// has one; otherwise `client_subscription`, the profile the client had active (clients such as
/// Clash Verge Rev expand a subscription into plain `proxies`, so the core never sees it).
pub fn dims_of(c: &Conn, client_subscription: Option<&str>) -> Dims {
    let outbound = c.chains.first().cloned().unwrap_or_default();
    let subscription = if outbound.is_empty() {
        String::new()
    } else if is_direct(&outbound) {
        DIRECT.to_string()
    } else {
        c.provider_chains.iter().find(|p| !p.is_empty()).cloned().or_else(|| client_subscription.map(str::to_string)).unwrap_or_default()
    };
    Dims { outbound, subscription, process: c.process.clone(), inbound: c.inbound.clone(), host: c.host.clone() }
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
        let d = dims_of(&c, Some("白月光"));

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
        let d = dims_of(&c, Some("白月光"));

        // Assert
        assert_eq!(d.subscription, "机场A");
    }

    #[test]
    fn direct_traffic_belongs_to_no_subscription() {
        // Act
        let d = [&["DIRECT"][..], &["REJECT-DROP"], &[]].map(|chains| dims_of(&conn("1", 0, 0, chains), Some("白月光")).subscription);

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
        let hk = dims_of(&conn("1", 0, 0, &["HK", "Proxy"]), Some("白月光"));
        let direct = dims_of(&conn("2", 0, 0, &["DIRECT"]), Some("白月光"));

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
    fn registrable_domains_merge_subdomains_but_not_addresses() {
        // Act
        let got = ["a.cdn.example.co.uk", "www.google.com", "localhost", "203.0.113.5", "2001:db8::1"].map(registrable);

        // Assert
        assert_eq!(got, ["example.co.uk", "google.com", "localhost", "203.0.113.5", "2001:db8::1"]);
    }
}
