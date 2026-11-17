//! Client of mihomo's external controller (its RESTful API), over TCP, a unix socket or a
//! Windows named pipe. The socket and pipe listeners do not check the secret (mihomo
//! `hub/route/server.go`); their file permissions are the access control.

use crate::domain::counter::Bytes;
use crate::domain::cycle::Reported;
use crate::domain::split::Conn;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// `http://host:port` and the controller's secret ("" when none).
    Http {
        url: String,
        secret: String,
    },
    Unix(PathBuf),
    Pipe(String),
}

impl Endpoint {
    /// What the interface shows as the address.
    pub fn describe(&self) -> String {
        match self {
            Endpoint::Http { url, .. } => url.clone(),
            Endpoint::Unix(p) => p.display().to_string(),
            Endpoint::Pipe(p) => p.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    Unreachable(String),
    /// The controller wants a (different) secret.
    Unauthorized,
    /// Something answered, but not as a mihomo core does.
    Unexpected(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unreachable(e) => write!(f, "无法连接：{e}"),
            ApiError::Unauthorized => f.write_str("secret 不正确或缺失"),
            ApiError::Unexpected(e) => f.write_str(e),
        }
    }
}

/// One line of `/traffic`: the rate over the last second, and the counters since the core
/// started (missing on cores older than the 2024 releases that added them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub rate: Bytes,
    pub totals: Option<Bytes>,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    secret: String,
}

fn unreachable(e: reqwest::Error) -> ApiError {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    ApiError::Unreachable(msg)
}

impl Client {
    pub fn new(endpoint: &Endpoint) -> Result<Client, ApiError> {
        super::install_tls();
        // The controller is local or on the LAN: never route it through a proxy, and give up
        // quickly when it is gone. `/traffic` writes every second, so a long silence is a
        // dead connection (for example after the machine slept).
        let builder = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(3)).read_timeout(Duration::from_secs(10));
        let (builder, base, secret) = match endpoint {
            Endpoint::Http { url, secret } => (builder, url.trim_end_matches('/').to_string(), secret.clone()),
            #[cfg(unix)]
            Endpoint::Unix(path) => (builder.unix_socket(path.clone()), "http://localhost".to_string(), String::new()),
            #[cfg(windows)]
            Endpoint::Pipe(name) => (builder.windows_named_pipe(name.clone()), "http://localhost".to_string(), String::new()),
            #[allow(unreachable_patterns)]
            other => return Err(ApiError::Unreachable(format!("此系统不支持 {}", other.describe()))),
        };
        let http = builder.build().map_err(unreachable)?;
        Ok(Client { http, base, secret })
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response, ApiError> {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if !self.secret.is_empty() {
            req = req.bearer_auth(&self.secret);
        }
        let resp = req.send().await.map_err(unreachable)?;
        match resp.status() {
            s if s == reqwest::StatusCode::UNAUTHORIZED => Err(ApiError::Unauthorized),
            s if !s.is_success() => Err(ApiError::Unexpected(format!("控制接口返回 HTTP {}", s.as_u16()))),
            _ => Ok(resp),
        }
    }

    /// The core's version, e.g. `v1.19.31`. Original Clash and Clash Premium are refused: they
    /// lack the cumulative counters the statistics are built on.
    pub async fn version(&self) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Version {
            #[serde(default)]
            meta: bool,
            version: String,
        }
        let body = self.get("/version").await?;
        let v: Version = tokio::time::timeout(Duration::from_secs(5), body.json())
            .await
            .map_err(|_| ApiError::Unreachable("读取超时".into()))?
            .map_err(|_| ApiError::Unexpected("返回的不是 mihomo 的版本信息".into()))?;
        if !v.meta {
            return Err(ApiError::Unexpected(format!("{} 不是 mihomo 内核", v.version)));
        }
        Ok(v.version)
    }

    /// The open connections and the core's counters, read at the same instant.
    pub async fn connections(&self) -> Result<Snapshot, ApiError> {
        let body = self.get("/connections").await?.bytes().await.map_err(unreachable)?;
        parse_snapshot(&body).ok_or_else(|| ApiError::Unexpected("连接列表格式无法识别".into()))
    }

    /// Usage reports of the core's proxy-providers that have one (fetched from the provider's
    /// `subscription-userinfo` header), by provider name.
    pub async fn providers(&self) -> Result<Vec<(String, Reported)>, ApiError> {
        let body = self.get("/providers/proxies").await?.bytes().await.map_err(unreachable)?;
        parse_providers(&body).ok_or_else(|| ApiError::Unexpected("订阅信息格式无法识别".into()))
    }

    /// The type of every outbound the core has (`Direct`, `Shadowsocks`, `Selector`, …), by name.
    pub async fn proxy_types(&self) -> Result<std::collections::HashMap<String, String>, ApiError> {
        let body = self.get("/proxies").await?.bytes().await.map_err(unreachable)?;
        parse_proxy_types(&body).ok_or_else(|| ApiError::Unexpected("出站列表格式无法识别".into()))
    }

    pub async fn traffic(&self) -> Result<TrafficStream, ApiError> {
        Ok(TrafficStream { resp: self.get("/traffic").await?, buf: Vec::new() })
    }
}

/// The `/traffic` stream: one JSON object per line, one line per second.
pub struct TrafficStream {
    resp: reqwest::Response,
    buf: Vec<u8>,
}

impl TrafficStream {
    /// The next sample; `None` when the core closed the stream.
    pub async fn next(&mut self) -> Result<Option<Sample>, ApiError> {
        loop {
            if let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=end).collect();
                if let Some(s) = parse_sample(&line) {
                    return Ok(Some(s));
                }
                continue;
            }
            match self.resp.chunk().await.map_err(unreachable)? {
                Some(chunk) => self.buf.extend_from_slice(&chunk),
                None => return Ok(None),
            }
            if self.buf.len() > 64 * 1024 {
                return Err(ApiError::Unexpected("流量数据格式无法识别".into()));
            }
        }
    }
}

/// A `/connections` snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub totals: Bytes,
    pub conns: Vec<Conn>,
}

/// Parses a `/connections` body. Only the fields the statistics use are read.
pub fn parse_snapshot(body: &[u8]) -> Option<Snapshot> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        upload_total: i64,
        download_total: i64,
        #[serde(default)]
        connections: Option<Vec<Raw>>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Raw {
        id: String,
        #[serde(default)]
        upload: i64,
        #[serde(default)]
        download: i64,
        #[serde(default)]
        chains: Vec<String>,
        #[serde(default)]
        provider_chains: Vec<String>,
        #[serde(default)]
        metadata: Meta,
    }
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase", default)]
    struct Meta {
        process: String,
        inbound_name: String,
        host: String,
        #[serde(rename = "destinationIP")]
        destination_ip: String,
    }
    let b: Body = serde_json::from_slice(body).ok()?;
    let n = |v: i64| v.max(0) as u64;
    Some(Snapshot {
        totals: Bytes::new(n(b.upload_total), n(b.download_total)),
        conns: b
            .connections
            .unwrap_or_default()
            .into_iter()
            .map(|c| Conn {
                id: c.id,
                bytes: Bytes::new(n(c.upload), n(c.download)),
                chains: c.chains,
                provider_chains: c.provider_chains,
                process: c.metadata.process,
                inbound: c.metadata.inbound_name,
                host: if c.metadata.host.is_empty() { c.metadata.destination_ip } else { c.metadata.host },
            })
            .collect(),
    })
}

/// Parses a `/providers/proxies` body into the providers' usage reports.
pub fn parse_providers(body: &[u8]) -> Option<Vec<(String, Reported)>> {
    #[derive(Deserialize)]
    struct Body {
        providers: std::collections::HashMap<String, Provider>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Provider {
        #[serde(default)]
        subscription_info: Option<Info>,
        #[serde(default)]
        updated_at: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Info {
        #[serde(default)]
        upload: i64,
        #[serde(default)]
        download: i64,
        #[serde(default)]
        total: i64,
        #[serde(default)]
        expire: i64,
    }
    let b: Body = serde_json::from_slice(body).ok()?;
    let mut out: Vec<(String, Reported)> = b
        .providers
        .into_iter()
        .filter_map(|(name, p)| {
            let i = p.subscription_info?;
            let updated = p.updated_at.and_then(|t| t.parse::<jiff::Timestamp>().ok()).map(|t| t.as_second());
            Some((
                name,
                Reported {
                    upload: i.upload.max(0) as u64,
                    download: i.download.max(0) as u64,
                    total: i.total.max(0) as u64,
                    expire: (i.expire > 0).then_some(i.expire),
                    updated,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Some(out)
}

/// Parses `/proxies` into each outbound's type.
pub fn parse_proxy_types(body: &[u8]) -> Option<std::collections::HashMap<String, String>> {
    #[derive(Deserialize)]
    struct Body {
        proxies: std::collections::HashMap<String, Proxy>,
    }
    #[derive(Deserialize)]
    struct Proxy {
        #[serde(rename = "type")]
        kind: String,
    }
    let b: Body = serde_json::from_slice(body).ok()?;
    Some(b.proxies.into_iter().map(|(name, p)| (name, p.kind)).collect())
}

/// Parses one `/traffic` line; blank or foreign lines give `None`.
pub fn parse_sample(line: &[u8]) -> Option<Sample> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Line {
        up: i64,
        down: i64,
        up_total: Option<i64>,
        down_total: Option<i64>,
    }
    let l: Line = serde_json::from_slice(line.trim_ascii()).ok()?;
    let n = |v: i64| v.max(0) as u64;
    Some(Sample { rate: Bytes::new(n(l.up), n(l.down)), totals: l.up_total.zip(l.down_total).map(|(u, d)| Bytes::new(n(u), n(d))) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_traffic_line_with_counters() {
        // Arrange
        let line = br#"{"up":5250,"down":1602893,"upTotal":446992437,"downTotal":5929316211}
"#;

        // Act
        let s = parse_sample(line).unwrap();

        // Assert
        assert_eq!(s.rate, Bytes::new(5250, 1602893));
        assert_eq!(s.totals, Some(Bytes::new(446992437, 5929316211)));
    }

    #[test]
    fn parses_a_connections_snapshot() {
        // Arrange: trimmed from a real mihomo v1.19.31 response.
        let body = br#"{"downloadTotal":5929316211,"uploadTotal":446992437,"memory":1,"connections":[
            {"id":"c1","metadata":{"network":"tcp","type":"Tun","sourceIP":"198.18.0.1","destinationIP":"142.250.1.1",
             "host":"www.google.com","process":"Google Chrome","inboundName":"DEFAULT-TUN","sourcePort":"5000"},
             "upload":10,"download":20,"start":"2026-11-16T08:00:00Z","chains":["HK 01","Proxy"],"providerChains":["",""],"rule":"Match"},
            {"id":"c2","metadata":{"destinationIP":"1.1.1.1","host":""},"upload":1,"download":2,"chains":["DIRECT"]}
        ]}"#;

        // Act
        let s = parse_snapshot(body).unwrap();

        // Assert
        assert_eq!(s.totals, Bytes::new(446992437, 5929316211));
        assert_eq!(s.conns[0].process, "Google Chrome");
        assert_eq!(s.conns[0].inbound, "DEFAULT-TUN");
        assert_eq!(s.conns[0].chains, ["HK 01", "Proxy"]);
        assert_eq!(s.conns[1].host, "1.1.1.1");
        assert!(s.conns[1].provider_chains.is_empty());
    }

    #[test]
    fn reads_the_providers_usage_reports() {
        // Arrange: providers without a subscription (rule sets, inline proxies) have no info.
        let body = r#"{"providers":{
            "default":{"name":"default","type":"Proxy","vehicleType":"Compatible","proxies":[]},
            "机场A":{"name":"机场A","type":"Proxy","vehicleType":"HTTP","proxies":[],"updatedAt":"2026-11-16T08:00:00.000000+08:00",
                     "subscriptionInfo":{"Upload":10,"Download":20,"Total":300,"Expire":1798000000}}}}"#;

        // Act
        let got = parse_providers(body.as_bytes()).unwrap();

        // Assert
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "机场A");
        assert_eq!(got[0].1, Reported { upload: 10, download: 20, total: 300, expire: Some(1798000000), updated: Some(1794787200) });
    }

    #[test]
    fn an_idle_core_lists_no_connections() {
        // Act: mihomo writes null when nothing is open.
        let s = parse_snapshot(br#"{"downloadTotal":1,"uploadTotal":2,"connections":null}"#).unwrap();

        // Assert
        assert!(s.conns.is_empty());
    }

    #[test]
    fn older_cores_give_rates_without_counters() {
        // Act
        let s = parse_sample(br#"{"up":1,"down":2}"#).unwrap();

        // Assert
        assert_eq!(s.totals, None);
    }

    #[test]
    fn blank_and_foreign_lines_are_skipped() {
        // Act
        let parsed = [&b"\n"[..], b"404 page not found", b"{\"hello\":\"mihomo\"}"].map(parse_sample);

        // Assert
        assert!(parsed.iter().all(Option::is_none));
    }

    /// A one-request-per-connection HTTP server on a unix socket, answering like mihomo. It says
    /// `connection: close`, or the client would reuse a connection the server already dropped.
    #[cfg(unix)]
    fn fake_core(dir: &std::path::Path, secret_required: bool) -> PathBuf {
        use std::io::{BufRead, BufReader, Write};
        let path = dir.join("core.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let mut out = stream;
                if secret_required {
                    let _ = out.write_all(b"HTTP/1.1 401 Unauthorized\r\nconnection: close\r\ncontent-length: 0\r\n\r\n");
                } else if request.starts_with("GET /version") {
                    let body = r#"{"meta":true,"version":"v1.19.31"}"#;
                    let _ = write!(
                        out,
                        "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                        body.len()
                    );
                } else {
                    let _ = out.write_all(
                        b"HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n",
                    );
                    // Two samples, the second split across chunks, then the end of the stream.
                    for chunk in [
                        "{\"up\":10,\"down\":20,\"upTotal\":100,\"downTotal\":200}\n",
                        "{\"up\":1,\"down\":2,",
                        "\"upTotal\":101,\"downTotal\":202}\n",
                    ] {
                        let _ = write!(out, "{:x}\r\n{chunk}\r\n", chunk.len());
                    }
                    let _ = out.write_all(b"0\r\n\r\n");
                }
            }
        });
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reads_version_and_traffic_over_a_unix_socket() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let client = Client::new(&Endpoint::Unix(fake_core(dir.path(), false))).unwrap();

        // Act
        let version = client.version().await.unwrap();
        let mut stream = client.traffic().await.unwrap();
        let samples = [stream.next().await.unwrap(), stream.next().await.unwrap(), stream.next().await.unwrap()];

        // Assert
        assert_eq!(version, "v1.19.31");
        assert_eq!(samples[0].unwrap().totals, Some(Bytes::new(100, 200)));
        assert_eq!(samples[1].unwrap().totals, Some(Bytes::new(101, 202)));
        assert_eq!(samples[2], None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_refused_secret_is_reported_as_such() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let client = Client::new(&Endpoint::Unix(fake_core(dir.path(), true))).unwrap();

        // Act
        let result = client.version().await;

        // Assert
        assert_eq!(result, Err(ApiError::Unauthorized));
    }

    #[test]
    fn reads_the_type_of_each_outbound() {
        // Arrange: trimmed from mihomo v1.19, a group and a custom direct outbound among them.
        let body = r#"{"proxies":{"DIRECT":{"type":"Direct","udp":true},"直连-以太网":{"type":"Direct"},
            "HK 01":{"type":"Shadowsocks"},"节点选择":{"type":"Selector","now":"HK 01","all":["HK 01"]}}}"#;

        // Act
        let types = parse_proxy_types(body.as_bytes()).unwrap();

        // Assert
        assert_eq!(types["直连-以太网"], "Direct");
        assert_eq!(types["HK 01"], "Shadowsocks");
        assert_eq!(types.len(), 4);
    }
}
