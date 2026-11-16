//! Finding the mihomo cores running on this machine and where their controllers listen.
//!
//! A core's command line usually says it all: clients start it with `-d`/`-f` and often
//! `-ext-ctl-unix` / `-ext-ctl-pipe`. Otherwise the controller is in its config file. When the
//! command line cannot be read (a core running as SYSTEM on Windows), the known socket and pipe
//! locations of Clash Verge Rev are tried instead.

use super::mihomo::Endpoint;
use crate::domain::counter::CoreId;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

/// A core process as seen in the process list.
#[derive(Clone, Debug)]
pub struct LocalCore {
    pub core: CoreId,
    pub exe: Option<PathBuf>,
    /// Process name, without `.exe`.
    pub name: String,
    /// Arguments after the program; empty when the system does not show them to us.
    pub args: Vec<String>,
}

/// Names of mihomo builds as clients ship them. Release binaries are also named
/// `mihomo-<os>-<arch>-…`. Matching exactly keeps out tools that merely carry "mihomo" in their
/// name (this app among them).
fn is_core_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let n = n.strip_suffix(".exe").unwrap_or(&n);
    matches!(n, "mihomo" | "verge-mihomo" | "verge-mihomo-alpha" | "clash-meta" | "clash.meta" | "flclashcore")
        || ["darwin", "windows", "linux", "freebsd"].iter().any(|os| n.starts_with(&format!("mihomo-{os}-")))
}

/// Every mihomo process currently running.
pub fn running_cores() -> Vec<LocalCore> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut sys) = System::new() else { return Vec::new() };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_exe(UpdateKind::Always),
    );
    let mut cores: Vec<LocalCore> = sys
        .processes()
        .values()
        .filter(|p| p.thread_kind().is_none())
        .filter(|p| is_core_name(&p.name().to_string_lossy()))
        .map(|p| LocalCore {
            core: CoreId { pid: p.pid().as_u32(), started_at: p.start_time() as i64 },
            exe: p.exe().map(Path::to_path_buf),
            name: p.name().to_string_lossy().trim_end_matches(".exe").to_string(),
            args: p.cmd().iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect(),
        })
        .collect();
    cores.sort_by_key(|c| c.core.pid);
    cores
}

/// The command-line flags of a core that locate its controller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreArgs {
    pub dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub ext_ctl: Option<String>,
    pub ext_ctl_unix: Option<String>,
    pub ext_ctl_pipe: Option<String>,
    pub secret: Option<String>,
}

/// Parses Go-style flags: `-name value`, `-name=value`, with one or two dashes.
pub fn parse_args(args: &[String]) -> CoreArgs {
    let mut out = CoreArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(flag) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')) else { continue };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (flag, None),
        };
        if !matches!(name, "d" | "f" | "ext-ctl" | "ext-ctl-unix" | "ext-ctl-pipe" | "secret") {
            continue;
        }
        let Some(value) = inline.or_else(|| it.next().cloned()) else { break };
        match name {
            "d" => out.dir = Some(PathBuf::from(value)),
            "f" => out.config = Some(PathBuf::from(value)),
            "ext-ctl" => out.ext_ctl = Some(value),
            "ext-ctl-unix" => out.ext_ctl_unix = Some(value),
            "ext-ctl-pipe" => out.ext_ctl_pipe = Some(value),
            _ => out.secret = Some(value),
        }
    }
    out
}

/// Where a core's controller listens, as far as its flags and config say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Controller {
    pub http: Option<String>,
    pub unix: Option<PathBuf>,
    pub pipe: Option<String>,
    pub secret: String,
}

impl Controller {
    /// Ways to reach it, most direct first: the socket and pipe need no secret.
    pub fn endpoints(&self) -> Vec<Endpoint> {
        let mut out = Vec::new();
        if let Some(p) = &self.unix {
            out.push(Endpoint::Unix(p.clone()));
        }
        if let Some(p) = &self.pipe {
            out.push(Endpoint::Pipe(p.clone()));
        }
        if let Some(h) = &self.http {
            out.push(Endpoint::Http { url: h.clone(), secret: self.secret.clone() });
        }
        out
    }
}

/// `external-controller` as written (`127.0.0.1:9090`, `:9090`, `0.0.0.0:9090`, `[::]:9090`)
/// to a URL this machine can open. Empty means the listener is off.
pub fn controller_url(addr: &str) -> Option<String> {
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }
    if addr.starts_with("http://") || addr.starts_with("https://") {
        return Some(addr.trim_end_matches('/').to_string());
    }
    let (host, port) = addr.rsplit_once(':')?;
    port.parse::<u16>().ok()?;
    let host = match host.trim_start_matches('[').trim_end_matches(']') {
        "" | "0.0.0.0" | "::" => "127.0.0.1",
        _ => host,
    };
    Some(format!("http://{host}:{port}"))
}

/// The controller of a core: flags win over the config file, as in mihomo itself.
pub fn controller_of(args: &CoreArgs) -> Controller {
    let config = args.config.clone().or_else(|| args.dir.as_ref().map(|d| d.join("config.yaml")));
    let from_file = config.and_then(|p| std::fs::read_to_string(p).ok()).map(|t| controller_in_config(&t)).unwrap_or_default();
    Controller {
        http: args.ext_ctl.as_deref().and_then(controller_url).or(from_file.http),
        unix: args.ext_ctl_unix.clone().filter(|s| !s.is_empty()).map(PathBuf::from).or(from_file.unix),
        pipe: args.ext_ctl_pipe.clone().filter(|s| !s.is_empty()).or(from_file.pipe),
        secret: args.secret.clone().unwrap_or(from_file.secret),
    }
}

/// The controller settings in a mihomo config file.
pub fn controller_in_config(text: &str) -> Controller {
    let Ok(docs) = YamlLoader::load_from_str(text) else { return Controller::default() };
    let Some(doc) = docs.first() else { return Controller::default() };
    let s = |key: &str| match &doc[key] {
        Yaml::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    };
    Controller {
        http: s("external-controller").and_then(|a| controller_url(&a)),
        unix: s("external-controller-unix").map(PathBuf::from),
        pipe: s("external-controller-pipe"),
        secret: s("secret").unwrap_or_default(),
    }
}

/// Which client a core belongs to, as a stable key and a display name. The key stays the same
/// when the client moves its controller (Clash Verge Rev changes socket paths between versions
/// and between its normal and service modes).
pub fn identity(core: &LocalCore, args: &CoreArgs) -> (String, String) {
    if is_verge(core) {
        return (VERGE_KEY.into(), "Clash Verge Rev".into());
    }
    let place = args
        .dir
        .clone()
        .or_else(|| args.config.as_ref().and_then(|c| c.parent().map(Path::to_path_buf)))
        .or_else(|| core.exe.clone())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| core.name.clone());
    (format!("mihomo:{place}"), core.name.clone())
}

pub const VERGE_KEY: &str = "clash-verge-rev";

pub fn is_verge(core: &LocalCore) -> bool {
    core.name.to_ascii_lowercase().starts_with("verge-mihomo")
        || core.exe.as_ref().is_some_and(|e| e.to_string_lossy().to_ascii_lowercase().contains("clash-verge"))
}

/// Where Clash Verge Rev puts its controller when we cannot read the core's command line:
/// the per-user temporary directory (normal mode), the service's per-user directory (service
/// mode) and the location of releases before 2.4. Windows pipes carry a per-user hash, so they
/// are listed rather than guessed.
pub fn verge_endpoints() -> Vec<Endpoint> {
    #[cfg(unix)]
    {
        let mut paths = Vec::new();
        if let Some(tmp) = std::env::var_os("TMPDIR").filter(|v| !v.is_empty()) {
            paths.push(PathBuf::from(tmp).join("verge-mihomo.sock"));
        }
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        paths.push(PathBuf::from(format!("/var/run/clash-verge-service/users/{uid}/verge-mihomo.sock")));
        paths.push(PathBuf::from("/tmp/verge/verge-mihomo.sock"));
        paths.into_iter().filter(|p| p.exists()).map(Endpoint::Unix).collect()
    }
    #[cfg(windows)]
    {
        let Ok(entries) = std::fs::read_dir(r"\\.\pipe\") else { return Vec::new() };
        let mut pipes: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().starts_with("verge-mihomo"))
            .map(|n| format!(r"\\.\pipe\{n}"))
            .collect();
        pipes.sort();
        pipes.into_iter().map(Endpoint::Pipe).collect()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn recognises_core_builds_but_not_tools_named_after_mihomo() {
        // Arrange
        let cores = ["mihomo", "verge-mihomo", "verge-mihomo-alpha.exe", "mihomo-darwin-arm64-v1.19.31", "FlClashCore"];
        let others = ["mihomo-traffic", "mihomo-monitor", "Mihomo-Monitor.exe", "clash-verge"];

        // Act / Assert
        assert!(cores.iter().all(|n| is_core_name(n)), "{cores:?}");
        assert!(!others.iter().any(|n| is_core_name(n)), "{others:?}");
    }

    #[test]
    fn parses_the_flags_clash_verge_rev_starts_its_core_with() {
        // Arrange: as seen with Clash Verge Rev 2.5.6 in service mode on macOS.
        let a = args("-d /Library/Application_Support/clash-verge-service/users/501/runtime -f /Library/Application_Support/clash-verge-service/users/501/runtime/config.yaml -ext-ctl-unix /var/run/clash-verge-service/users/501/verge-mihomo.sock");

        // Act
        let parsed = parse_args(&a);

        // Assert
        assert_eq!(parsed.ext_ctl_unix.as_deref(), Some("/var/run/clash-verge-service/users/501/verge-mihomo.sock"));
        assert_eq!(parsed.dir, Some(PathBuf::from("/Library/Application_Support/clash-verge-service/users/501/runtime")));
        assert!(parsed.config.is_some());
    }

    #[test]
    fn parses_inline_values_and_double_dashes() {
        // Act
        let parsed = parse_args(&args("--ext-ctl=0.0.0.0:9090 -secret=abc -v -ext-ctl-pipe \\\\.\\pipe\\mihomo"));

        // Assert
        assert_eq!(parsed.ext_ctl.as_deref(), Some("0.0.0.0:9090"));
        assert_eq!(parsed.secret.as_deref(), Some("abc"));
        assert_eq!(parsed.ext_ctl_pipe.as_deref(), Some(r"\\.\pipe\mihomo"));
    }

    #[test]
    fn a_trailing_flag_without_value_is_ignored() {
        // Act
        let parsed = parse_args(&args("-d"));

        // Assert
        assert_eq!(parsed, CoreArgs::default());
    }

    #[test]
    fn controller_urls_point_at_a_reachable_host() {
        // Act
        let urls = ["127.0.0.1:9090", ":9097", "0.0.0.0:9090", "[::]:9090", "192.168.1.1:9090", "", "nonsense"].map(controller_url);

        // Assert
        assert_eq!(
            urls,
            [
                Some("http://127.0.0.1:9090".to_string()),
                Some("http://127.0.0.1:9097".to_string()),
                Some("http://127.0.0.1:9090".to_string()),
                Some("http://127.0.0.1:9090".to_string()),
                Some("http://192.168.1.1:9090".to_string()),
                None,
                None,
            ]
        );
    }

    #[test]
    fn reads_the_controller_from_a_config_file() {
        // Arrange
        let yaml = "mixed-port: 7897\nexternal-controller: 127.0.0.1:9097\nsecret: 's3cret'\nexternal-controller-unix: /tmp/m.sock\n";

        // Act
        let c = controller_in_config(yaml);

        // Assert
        assert_eq!(c.http.as_deref(), Some("http://127.0.0.1:9097"));
        assert_eq!(c.unix, Some(PathBuf::from("/tmp/m.sock")));
        assert_eq!(c.secret, "s3cret");
    }

    #[test]
    fn flags_override_the_config_file() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.yaml"), "external-controller: 127.0.0.1:9090\nsecret: file\n").unwrap();
        let a = CoreArgs { dir: Some(dir.path().to_path_buf()), secret: Some("flag".into()), ..CoreArgs::default() };

        // Act
        let c = controller_of(&a);

        // Assert
        assert_eq!(c.http.as_deref(), Some("http://127.0.0.1:9090"));
        assert_eq!(c.secret, "flag");
    }

    #[test]
    fn socket_comes_before_http() {
        // Arrange
        let c = Controller { http: Some("http://127.0.0.1:9090".into()), unix: Some("/tmp/m.sock".into()), pipe: None, secret: "x".into() };

        // Act
        let eps = c.endpoints();

        // Assert
        assert_eq!(eps[0], Endpoint::Unix("/tmp/m.sock".into()));
        assert!(matches!(&eps[1], Endpoint::Http { secret, .. } if secret == "x"));
    }

    #[test]
    fn verge_cores_share_one_identity_wherever_they_run() {
        // Arrange
        let core = |name: &str, exe: &str| LocalCore {
            core: CoreId { pid: 1, started_at: 0 },
            exe: Some(exe.into()),
            name: name.into(),
            args: vec![],
        };
        let service = core("verge-mihomo", "/Library/Application Support/clash-verge-service/cores/verge-mihomo");
        let sidecar = core("verge-mihomo-alpha", "/Applications/Clash Verge.app/Contents/MacOS/verge-mihomo-alpha");
        let plain = core("mihomo", "/usr/local/bin/mihomo");

        // Act
        let ids = [&service, &sidecar, &plain].map(|c| identity(c, &CoreArgs { dir: Some("/etc/mihomo".into()), ..CoreArgs::default() }).0);

        // Assert
        assert_eq!(ids[0], VERGE_KEY);
        assert_eq!(ids[1], VERGE_KEY);
        assert_eq!(ids[2], "mihomo:/etc/mihomo");
    }
}
