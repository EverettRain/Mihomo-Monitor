//! Host groups: named sets of domains and IP ranges ("YouTube", "公司内网"), so the views can show
//! a service instead of a pile of hosts and addresses.
//!
//! An entry is one of Clash's matching rules: an exact domain, a domain with its subdomains, a
//! keyword, or an IP range. Matching follows Clash's precedence: exact domain, then the longest
//! matching suffix, then keywords; addresses by the longest matching prefix.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    /// This domain only (Clash `DOMAIN`).
    Domain,
    /// The domain and its subdomains (Clash `DOMAIN-SUFFIX`).
    Suffix,
    /// Any domain containing the text (Clash `DOMAIN-KEYWORD`).
    Keyword,
    /// An address or a range of them (Clash `IP-CIDR`).
    Cidr,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Domain => "domain",
            RuleKind::Suffix => "suffix",
            RuleKind::Keyword => "keyword",
            RuleKind::Cidr => "cidr",
        }
    }

    pub fn parse(s: &str) -> Option<RuleKind> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "domain" | "域名" | "exact" | "full" => Some(RuleKind::Domain),
            "suffix" | "domain-suffix" | "域名后缀" | "子域名" | "域名及子域名" => Some(RuleKind::Suffix),
            "keyword" | "domain-keyword" | "关键字" | "关键词" | "包含关键字" => Some(RuleKind::Keyword),
            "cidr" | "ip" | "ip-cidr" | "ip-cidr6" | "ipcidr" | "ip段" | "网段" | "ip / 网段" => Some(RuleKind::Cidr),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Rule {
    pub kind: RuleKind,
    pub value: String,
}

/// An address range: network address and prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Net {
    addr: IpAddr,
    prefix: u8,
}

fn bits(a: IpAddr) -> (u128, u8) {
    match a {
        IpAddr::V4(v) => (u128::from(u32::from(v)), 32),
        IpAddr::V6(v) => (u128::from(v), 128),
    }
}

fn masked(value: u128, width: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        value & (u128::MAX << (width - prefix)) & (if width == 32 { u128::from(u32::MAX) } else { u128::MAX })
    }
}

impl Net {
    fn parse(s: &str) -> Option<Net> {
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p.parse::<u8>().ok()?)),
            None => (s, None),
        };
        let addr: IpAddr = ip.trim().trim_start_matches('[').trim_end_matches(']').parse().ok()?;
        let (value, width) = bits(addr);
        let prefix = prefix.unwrap_or(width);
        if prefix > width {
            return None;
        }
        let network = masked(value, width, prefix);
        let addr = match addr {
            IpAddr::V4(_) => IpAddr::V4((network as u32).into()),
            IpAddr::V6(_) => IpAddr::V6(network.into()),
        };
        Some(Net { addr, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        let (net, width) = bits(self.addr);
        let (value, w) = bits(ip);
        w == width && masked(value, width, self.prefix) == net
    }

    /// A single address prints as itself, a range as `network/prefix`.
    fn text(&self) -> String {
        if self.prefix == bits(self.addr).1 {
            self.addr.to_string()
        } else {
            format!("{}/{}", self.addr, self.prefix)
        }
    }
}

fn valid_domain(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 253
        && !d.starts_with('.')
        && !d.ends_with('.')
        && !d.contains("..")
        && d.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Rule {
    /// A rule with its value cleaned up (lower case, no trailing dot, ranges to their network
    /// address); `Err` says why the value does not fit the kind.
    pub fn new(kind: RuleKind, value: &str) -> Result<Rule, String> {
        let v = value.trim().trim_end_matches('.').to_lowercase();
        let value = match kind {
            RuleKind::Domain | RuleKind::Suffix => {
                let v = v.trim_start_matches("+.").trim_start_matches("*.").trim_start_matches('.').to_string();
                if !valid_domain(&v) {
                    return Err(format!("“{value}”不是有效的域名"));
                }
                v
            }
            RuleKind::Keyword => {
                if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == ',') {
                    return Err(format!("“{value}”不是有效的关键字"));
                }
                v
            }
            RuleKind::Cidr => Net::parse(&v).ok_or_else(|| format!("“{value}”不是有效的 IP 或网段"))?.text(),
        };
        Ok(Rule { kind, value })
    }
}

/// Reads one entry as people write them. Blank lines and comments give `None`.
///
/// - Clash rule lines: `DOMAIN-SUFFIX,google.com` (a trailing policy is ignored);
/// - Clash domain sets: `+.google.com`, `*.google.com`, `.google.com` (with subdomains);
/// - an address or range: `1.1.1.1`, `10.0.0.0/8`, `2001:db8::/32`;
/// - a URL or `host:port`: its host;
/// - a bare domain: `bare` decides whether it includes subdomains.
pub fn parse_entry(line: &str, bare: RuleKind) -> Result<Option<Rule>, String> {
    let line = line.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with("//") || line.starts_with(';') {
        return Ok(None);
    }
    if let Some((head, rest)) = line.split_once(',') {
        let value = rest.split(',').next().unwrap_or("").trim();
        let kind = match head.trim().to_ascii_uppercase().as_str() {
            "DOMAIN" => RuleKind::Domain,
            "DOMAIN-SUFFIX" => RuleKind::Suffix,
            "DOMAIN-KEYWORD" => RuleKind::Keyword,
            "IP-CIDR" | "IP-CIDR6" => RuleKind::Cidr,
            other => return Err(format!("不支持的规则类型 {other}（只支持域名、域名后缀、关键字与 IP 段）")),
        };
        return Rule::new(kind, value).map(Some);
    }
    if let Some(rest) = line.strip_prefix("+.").or_else(|| line.strip_prefix("*.")).or_else(|| line.strip_prefix('.')) {
        return Rule::new(RuleKind::Suffix, rest).map(Some);
    }
    if Net::parse(line).is_some() {
        return Rule::new(RuleKind::Cidr, line).map(Some);
    }
    let host = match line.split_once("://") {
        Some((_, after)) => after.split(['/', '?', '#']).next().unwrap_or(""),
        None => line.split('/').next().unwrap_or(""),
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) && !h.contains(':') => h,
        _ => host,
    };
    if Net::parse(host).is_some() {
        return Rule::new(RuleKind::Cidr, host).map(Some);
    }
    Rule::new(bare, host).map(Some)
}

/// Finds the group a host belongs to.
#[derive(Default)]
pub struct Matcher {
    names: Vec<String>,
    exact: HashMap<String, usize>,
    suffix: HashMap<String, usize>,
    keyword: Vec<(String, usize)>,
    nets: Vec<(Net, usize)>,
}

impl Matcher {
    /// Groups in order of precedence: when two groups claim the same entry, the first wins.
    pub fn new(groups: &[(String, Vec<Rule>)]) -> Matcher {
        let mut m = Matcher::default();
        for (i, (name, rules)) in groups.iter().enumerate() {
            m.names.push(name.clone());
            for r in rules {
                match r.kind {
                    RuleKind::Domain => {
                        m.exact.entry(r.value.clone()).or_insert(i);
                    }
                    RuleKind::Suffix => {
                        m.suffix.entry(r.value.clone()).or_insert(i);
                    }
                    RuleKind::Keyword => m.keyword.push((r.value.clone(), i)),
                    RuleKind::Cidr => {
                        if let Some(n) = Net::parse(&r.value) {
                            m.nets.push((n, i));
                        }
                    }
                }
            }
        }
        // Longest prefix first, so the most specific range wins.
        m.nets.sort_by_key(|(n, _)| std::cmp::Reverse(n.prefix));
        m
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn group_of(&self, host: &str) -> Option<&str> {
        let host = host.trim().trim_end_matches('.').to_lowercase();
        let i = if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
            self.nets.iter().find(|(n, _)| n.contains(ip)).map(|(_, i)| *i)
        } else {
            self.exact.get(&host).copied().or_else(|| {
                let mut rest = host.as_str();
                loop {
                    if let Some(i) = self.suffix.get(rest) {
                        break Some(*i);
                    }
                    match rest.split_once('.') {
                        Some((_, r)) => rest = r,
                        None => break None,
                    }
                }
                .or_else(|| self.keyword.iter().find(|(k, _)| host.contains(k.as_str())).map(|(_, i)| *i))
            })
        };
        i.map(|i| self.names[i].as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: RuleKind, value: &str) -> Rule {
        Rule::new(kind, value).unwrap()
    }

    #[test]
    fn values_are_normalized() {
        // Act
        let got = [
            Rule::new(RuleKind::Suffix, " WWW.YouTube.com. ").unwrap().value,
            Rule::new(RuleKind::Suffix, "+.google.com").unwrap().value,
            Rule::new(RuleKind::Cidr, "10.1.2.3/8").unwrap().value,
            Rule::new(RuleKind::Cidr, "1.1.1.1/32").unwrap().value,
            Rule::new(RuleKind::Cidr, "2001:DB8::1/32").unwrap().value,
        ];

        // Assert
        assert_eq!(got, ["www.youtube.com", "google.com", "10.0.0.0/8", "1.1.1.1", "2001:db8::/32"]);
    }

    #[test]
    fn invalid_values_are_refused_with_a_reason() {
        // Act
        let bad = [
            Rule::new(RuleKind::Domain, "not a domain"),
            Rule::new(RuleKind::Suffix, "a..b"),
            Rule::new(RuleKind::Cidr, "10.0.0.0/33"),
            Rule::new(RuleKind::Keyword, ""),
        ];

        // Assert
        assert!(bad.iter().all(|r| r.is_err()), "{bad:?}");
    }

    #[test]
    fn entries_are_read_the_ways_people_write_them() {
        // Arrange
        let lines = [
            "DOMAIN-SUFFIX,googlevideo.com,Proxy",
            "domain,www.google.com",
            "DOMAIN-KEYWORD,youtube",
            "IP-CIDR,91.108.4.0/22,no-resolve",
            "+.ytimg.com",
            "*.ggpht.com",
            "149.154.167.51",
            "https://www.netflix.com/browse?x=1",
            "api.github.com:443",
            "github.com",
        ];

        // Act
        let got: Vec<Rule> = lines.iter().map(|l| parse_entry(l, RuleKind::Suffix).unwrap().unwrap()).collect();

        // Assert
        assert_eq!(
            got,
            vec![
                rule(RuleKind::Suffix, "googlevideo.com"),
                rule(RuleKind::Domain, "www.google.com"),
                rule(RuleKind::Keyword, "youtube"),
                rule(RuleKind::Cidr, "91.108.4.0/22"),
                rule(RuleKind::Suffix, "ytimg.com"),
                rule(RuleKind::Suffix, "ggpht.com"),
                rule(RuleKind::Cidr, "149.154.167.51"),
                rule(RuleKind::Suffix, "www.netflix.com"),
                rule(RuleKind::Suffix, "api.github.com"),
                rule(RuleKind::Suffix, "github.com"),
            ]
        );
    }

    #[test]
    fn blank_lines_comments_and_other_rule_types() {
        // Act
        let skipped = ["", "   ", "# comment", "// note"].map(|l| parse_entry(l, RuleKind::Suffix));
        let unsupported = parse_entry("PROCESS-NAME,Telegram", RuleKind::Suffix);

        // Assert
        assert!(skipped.iter().all(|r| matches!(r, Ok(None))));
        assert!(unsupported.unwrap_err().contains("PROCESS-NAME"));
    }

    #[test]
    fn matching_follows_clash_precedence() {
        // Arrange
        let groups = vec![
            ("Google".to_string(), vec![rule(RuleKind::Suffix, "google.com"), rule(RuleKind::Keyword, "goog")]),
            ("YouTube".to_string(), vec![rule(RuleKind::Domain, "youtube.google.com"), rule(RuleKind::Suffix, "googlevideo.com")]),
            ("Telegram".to_string(), vec![rule(RuleKind::Cidr, "91.108.0.0/16")]),
            ("Telegram DC".to_string(), vec![rule(RuleKind::Cidr, "91.108.4.0/22"), rule(RuleKind::Cidr, "2001:b28:f23d::/48")]),
        ];
        let m = Matcher::new(&groups);

        // Act
        let got = [
            "youtube.google.com",
            "mail.google.com",
            "rr3---sn.googlevideo.com",
            "googleapis.cn",
            "91.108.5.1",
            "91.108.200.1",
            "2001:b28:f23d:f001::a",
            "example.com",
        ]
        .map(|h| m.group_of(h));

        // Assert
        assert_eq!(
            got,
            [
                Some("YouTube"),
                Some("Google"),
                Some("YouTube"),
                Some("Google"),
                Some("Telegram DC"),
                Some("Telegram"),
                Some("Telegram DC"),
                None
            ]
        );
    }

    #[test]
    fn the_first_group_wins_a_shared_entry() {
        // Arrange
        let groups =
            vec![("A".to_string(), vec![rule(RuleKind::Suffix, "x.com")]), ("B".to_string(), vec![rule(RuleKind::Suffix, "x.com")])];

        // Act
        let m = Matcher::new(&groups);
        let got = m.group_of("a.x.com");

        // Assert
        assert_eq!(got, Some("A"));
    }
}
