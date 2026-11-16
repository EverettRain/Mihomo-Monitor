//! Client of mihomo's external controller (its RESTful API), over TCP, a unix socket or a
//! Windows named pipe. The socket and pipe listeners do not check the secret (mihomo
//! `hub/route/server.go`); their file permissions are the access control.

use crate::domain::counter::Bytes;
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

    /// A one-connection-at-a-time HTTP server on a unix socket, answering like mihomo.
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
                    let _ = out.write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n");
                } else if request.starts_with("GET /version") {
                    let body = r#"{"meta":true,"version":"v1.19.31"}"#;
                    let _ = write!(out, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len());
                } else {
                    let _ = out.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n");
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
}
