//! Where each provider's traffic leaves: directly, through a SOCKS/HTTP proxy, or through an SSH
//! server the proxy tunnels to itself (`ssh -D`, a local SOCKS5 port). Only an SSH server is
//! needed on the other end: `sshd` with key login and TCP forwarding, nothing else.
//!
//! The settings live in `~/.claude-bridge/.env` and are read on every request:
//! - `CLAUDE_BRIDGE_EXITS`: the known exits, comma-separated (`ssh://root@1.2.3.4,socks5://127.0.0.1:1080`);
//! - `CLAUDE_BRIDGE_EXIT_<FAMILY>`: the exit one provider family uses; unset means direct.
//!
//! A configured exit never falls back to direct: when the tunnel is down the request is refused.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::geo::{self, Location};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::error::ProxyError;
use crate::{config, settings};

pub const EXITS_KEY: &str = "CLAUDE_BRIDGE_EXITS";
const ROUTE_PREFIX: &str = "CLAUDE_BRIDGE_EXIT_";

/// Provider families that can be routed (`Provider::family`).
pub const FAMILIES: &[&str] = &["anthropic", "codex", "openai", "openai-compat"];

/// How long a new SSH tunnel may take to log in and open its port.
const TUNNEL_STARTUP: Duration = Duration::from_secs(20);
/// Last lines of ssh's stderr kept for error messages.
const STDERR_LINES: usize = 6;

/// `CLAUDE_BRIDGE_EXIT_OPENAI_COMPAT` for `openai-compat`.
pub fn route_key(family: &str) -> String {
    format!("{ROUTE_PREFIX}{}", family.to_ascii_uppercase().replace('-', "_"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Exit {
    Direct,
    /// `socks5://`, `socks5h://`, `http://` or `https://` proxy URL, credentials included.
    Proxy(String),
    Ssh { user: Option<String>, host: String, port: Option<u16> },
}

impl Exit {
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let spec = spec.trim();
        if spec.is_empty() || spec.eq_ignore_ascii_case("direct") {
            return Ok(Exit::Direct);
        }
        let url = url::Url::parse(spec).map_err(|e| anyhow::anyhow!("bad exit {spec:?}: {e}"))?;
        let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(|| anyhow::anyhow!("exit {spec:?} has no host"))?;
        match url.scheme() {
            "ssh" => Ok(Exit::Ssh {
                user: Some(url.username()).filter(|u| !u.is_empty()).map(String::from),
                host: host.trim_start_matches('[').trim_end_matches(']').to_string(),
                port: url.port(),
            }),
            "socks5" | "socks5h" | "http" | "https" => Ok(Exit::Proxy(spec.to_string())),
            other => anyhow::bail!("exit {spec:?}: unsupported scheme {other} (ssh, socks5, socks5h, http, https)"),
        }
    }

    /// Canonical spelling, also the key of a running tunnel.
    pub fn spec(&self) -> String {
        match self {
            Exit::Direct => "direct".into(),
            Exit::Proxy(url) => url.clone(),
            Exit::Ssh { user, host, port } => {
                let user = user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
                let host = if host.contains(':') { format!("[{host}]") } else { host.clone() };
                let port = port.map(|p| format!(":{p}")).unwrap_or_default();
                format!("ssh://{user}{host}{port}")
            }
        }
    }

    /// What the UI and errors show: no proxy password.
    pub fn label(&self) -> String {
        match self {
            Exit::Proxy(url) => match url::Url::parse(url) {
                Ok(mut u) => {
                    let _ = u.set_password(None);
                    u.to_string().trim_end_matches('/').to_string()
                }
                Err(_) => url.clone(),
            },
            other => other.spec(),
        }
    }
}

fn setting(file: &std::collections::BTreeMap<String, String>, key: &str) -> Option<String> {
    file.get(key).cloned().filter(|v| !v.trim().is_empty()).or_else(|| config::env(key))
}

/// The exits listed in the settings, in order, without duplicates. Bad entries are skipped.
pub fn listed() -> Vec<Exit> {
    let file = settings::read();
    let mut out: Vec<Exit> = Vec::new();
    for spec in setting(&file, EXITS_KEY).unwrap_or_default().split(',') {
        match Exit::parse(spec) {
            Ok(Exit::Direct) => {}
            Ok(exit) if !out.contains(&exit) => out.push(exit),
            Ok(_) => {}
            Err(e) => warn!("{e:#}"),
        }
    }
    out
}

/// The exit a provider family is set to use.
pub fn configured(family: &str) -> anyhow::Result<Exit> {
    Exit::parse(&setting(&settings::read(), &route_key(family)).unwrap_or_default())
}

/// The value to store in `EXITS_KEY`.
pub fn format_list(exits: &[Exit]) -> String {
    exits.iter().map(Exit::spec).collect::<Vec<_>>().join(",")
}

struct Tunnel {
    child: Child,
    client: reqwest::Client,
    port: u16,
    stderr: Arc<std::sync::Mutex<Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub exit: String,
    pub running: bool,
    /// Local SOCKS5 port while running.
    pub port: Option<u16>,
}

/// HTTP clients per exit, and the SSH tunnels behind them.
pub struct Exits {
    direct: reqwest::Client,
    proxies: std::sync::Mutex<HashMap<String, reqwest::Client>>,
    tunnels: Mutex<HashMap<String, Tunnel>>,
}

impl Exits {
    pub fn new(direct: reqwest::Client) -> Self {
        Self { direct, proxies: Default::default(), tunnels: Default::default() }
    }

    /// The client for a provider family's configured exit, starting its SSH tunnel if needed.
    pub async fn for_family(&self, family: &str) -> Result<(Exit, reqwest::Client), ProxyError> {
        let exit = configured(family).map_err(|e| ProxyError::bad_request(format!("{family}: {e:#}")))?;
        let client = self.client(&exit).await.map_err(|e| {
            ProxyError::new(StatusCode::BAD_GATEWAY, format!("request not sent: exit {} for {family} is unavailable: {e:#}", exit.label()))
        })?;
        Ok((exit, client))
    }

    pub async fn client(&self, exit: &Exit) -> anyhow::Result<reqwest::Client> {
        match exit {
            Exit::Direct => Ok(self.direct.clone()),
            Exit::Proxy(url) => {
                let mut proxies = self.proxies.lock().unwrap();
                if let Some(client) = proxies.get(url) {
                    return Ok(client.clone());
                }
                let client = proxied_client(url)?;
                proxies.insert(url.clone(), client.clone());
                Ok(client)
            }
            Exit::Ssh { .. } => self.tunnel(exit).await,
        }
    }

    async fn tunnel(&self, exit: &Exit) -> anyhow::Result<reqwest::Client> {
        let spec = exit.spec();
        let mut tunnels = self.tunnels.lock().await;
        // Close tunnels nothing points at any more.
        let in_use: Vec<String> =
            FAMILIES.iter().filter_map(|f| configured(f).ok()).chain(listed()).map(|e| e.spec()).collect();
        tunnels.retain(|key, _| key == &spec || in_use.contains(key));

        if let Some(tunnel) = tunnels.get_mut(&spec) {
            match tunnel.child.try_wait() {
                Ok(None) => return Ok(tunnel.client.clone()),
                _ => {
                    warn!(exit = %spec, "ssh tunnel exited: {}", tunnel.stderr.lock().unwrap().join(" | "));
                    tunnels.remove(&spec);
                }
            }
        }
        let tunnel = start_tunnel(exit).await?;
        let client = tunnel.client.clone();
        info!(exit = %spec, port = tunnel.port, "ssh tunnel up");
        tunnels.insert(spec, tunnel);
        Ok(client)
    }

    /// Starts (or reuses) the tunnel for an SSH exit and returns its local SOCKS5 port.
    pub async fn tunnel_port(&self, exit: &Exit) -> anyhow::Result<u16> {
        anyhow::ensure!(matches!(exit, Exit::Ssh { .. }), "{} is not an SSH exit", exit.label());
        self.tunnel(exit).await?;
        let tunnels = self.tunnels.lock().await;
        tunnels.get(&exit.spec()).map(|t| t.port).ok_or_else(|| anyhow::anyhow!("tunnel {} closed", exit.label()))
    }

    pub async fn tunnels(&self) -> Vec<TunnelStatus> {
        let mut tunnels = self.tunnels.lock().await;
        tunnels
            .iter_mut()
            .map(|(exit, t)| {
                let running = matches!(t.child.try_wait(), Ok(None));
                TunnelStatus { exit: exit.clone(), running, port: running.then_some(t.port) }
            })
            .collect()
    }
}

/// For processes other than the proxy (UI, CLI): a client that leaves through a family's exit,
/// after the same country check the proxy makes. SSH tunnels belong to the proxy, so for an SSH
/// exit the running proxy is asked for its tunnel's port; without the proxy that fails rather
/// than going direct.
pub async fn outside_client(family: &str) -> anyhow::Result<reqwest::Client> {
    let exit = configured(family)?;
    let client = match &exit {
        Exit::Direct => client_builder().build()?,
        Exit::Proxy(url) => proxied_client(url)?,
        Exit::Ssh { .. } => {
            let port = crate::control::tunnel_port(config::port(), &exit.spec())
                .await
                .map_err(|e| anyhow::anyhow!("{family} goes through {}, whose SSH tunnel runs in the proxy: {e:#}", exit.label()))?;
            proxied_client(&format!("socks5h://127.0.0.1:{port}"))?
        }
    };
    geo::check(&client, family, &crate::server::family_origin(family)).await.map_err(|e| anyhow::anyhow!(e.message))?;
    Ok(client)
}

/// Upstreams drop idle keep-alive sockets; do not hand out stale ones.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).pool_idle_timeout(Duration::from_secs(30))
}

fn proxied_client(url: &str) -> anyhow::Result<reqwest::Client> {
    // socks5h: names are resolved on the far side, so local DNS neither leaks nor misroutes.
    let url = match url.strip_prefix("socks5://") {
        Some(rest) => format!("socks5h://{rest}"),
        None => url.to_string(),
    };
    Ok(client_builder().proxy(reqwest::Proxy::all(&url)?).build()?)
}

async fn start_tunnel(exit: &Exit) -> anyhow::Result<Tunnel> {
    let Exit::Ssh { user, host, port } = exit else { unreachable!("not an ssh exit") };
    let local = free_port()?;
    let target = match user {
        Some(user) => format!("{user}@{host}"),
        None => host.clone(),
    };
    let mut cmd = Command::new("ssh");
    cmd.args(["-N", "-D", &format!("127.0.0.1:{local}")])
        // No prompts: a password or an unknown-key question would hang the proxy.
        .args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new"])
        .args(["-o", "ExitOnForwardFailure=yes", "-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"]);
    if let Some(port) = port {
        cmd.args(["-p", &port.to_string()]);
    }
    cmd.arg(&target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("cannot run ssh: {e}"))?;

    let stderr = Arc::new(std::sync::Mutex::new(Vec::new()));
    if let Some(pipe) = child.stderr.take() {
        let stderr = stderr.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut kept = stderr.lock().unwrap();
                kept.push(line);
                let excess = kept.len().saturating_sub(STDERR_LINES);
                kept.drain(..excess);
            }
        });
    }

    // ssh opens the -D port only after it has logged in.
    let deadline = tokio::time::Instant::now() + TUNNEL_STARTUP;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            // Let the reader catch the last lines.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let text = stderr.lock().unwrap().join(" | ");
            anyhow::bail!("ssh {target} exited ({status}){}", if text.is_empty() { String::new() } else { format!(": {text}") });
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", local)).await.is_ok() {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("ssh {target} did not open its tunnel in {}s", TUNNEL_STARTUP.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let client = proxied_client(&format!("socks5h://127.0.0.1:{local}"))?;
    Ok(Tunnel { child, client, port: local, stderr })
}

fn free_port() -> anyhow::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// Where one exit, or one provider through its exit, comes out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    /// Exit label (`direct`, `ssh://root@1.2.3.4`, proxy URL without password).
    pub exit: String,
    /// The exit's canonical spelling, as stored in the settings.
    pub spec: String,
    pub location: Option<Location>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyProbe {
    pub family: String,
    /// Host whose trace page was asked.
    pub origin: String,
    pub probe: Probe,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// `direct` first, then the listed exits, seen from Cloudflare's neutral host.
    pub exits: Vec<Probe>,
    /// Enabled provider families, seen from each provider's own host through its exit.
    pub families: Vec<FamilyProbe>,
}

impl Exits {
    async fn probe(&self, exit: &Exit, origin: &str, start_tunnels: bool) -> Probe {
        let result = async {
            if matches!(exit, Exit::Ssh { .. }) && !start_tunnels {
                anyhow::bail!("the SSH tunnel runs inside the proxy; start the proxy to check it");
            }
            let client = self.client(exit).await?;
            geo::locate(&client, origin).await
        }
        .await;
        let (location, error) = match result {
            Ok(location) => (Some(location), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        Probe { exit: exit.label(), spec: exit.spec(), location, error }
    }

    /// Checks every exit and every enabled provider family at once. `origin` gives a family's
    /// upstream host. Without `start_tunnels` SSH exits are reported as not checked.
    pub async fn report(&self, origin: impl Fn(&str) -> String, start_tunnels: bool) -> Report {
        let mut all = vec![Exit::Direct];
        all.extend(listed());
        let exits = futures_util::future::join_all(all.iter().map(|e| self.probe(e, geo::NEUTRAL_ORIGIN, start_tunnels)));
        let families: Vec<(&str, String)> = FAMILIES
            .iter()
            .filter(|f| !crate::families::is_disabled(f))
            .map(|f| (*f, origin(f)))
            .collect();
        let family_probes = futures_util::future::join_all(families.iter().map(|(family, origin)| async move {
            let probe = match configured(family) {
                Ok(exit) => self.probe(&exit, origin, start_tunnels).await,
                Err(e) => Probe { exit: "?".into(), spec: String::new(), location: None, error: Some(format!("{e:#}")) },
            };
            FamilyProbe { family: family.to_string(), origin: origin.clone(), probe }
        }));
        let (exits, families) = tokio::join!(exits, family_probes);
        Report { exits, families }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exits() {
        assert_eq!(Exit::parse("").unwrap(), Exit::Direct);
        assert_eq!(Exit::parse("direct").unwrap(), Exit::Direct);
        assert_eq!(
            Exit::parse("ssh://root@1.2.3.4:2222").unwrap(),
            Exit::Ssh { user: Some("root".into()), host: "1.2.3.4".into(), port: Some(2222) }
        );
        assert_eq!(Exit::parse("ssh://vps.example").unwrap().spec(), "ssh://vps.example");
        assert_eq!(Exit::parse("ssh://u@[::1]:22").unwrap().spec(), "ssh://u@[::1]:22");
        assert!(matches!(Exit::parse("socks5://127.0.0.1:1080").unwrap(), Exit::Proxy(_)));
        assert!(Exit::parse("ftp://x").is_err());
        assert!(Exit::parse("nonsense").is_err());
    }

    #[test]
    fn hides_proxy_password() {
        assert_eq!(Exit::parse("http://u:secret@proxy:3128").unwrap().label(), "http://u@proxy:3128");
    }

    #[test]
    fn route_keys() {
        assert_eq!(route_key("openai-compat"), "CLAUDE_BRIDGE_EXIT_OPENAI_COMPAT");
    }
}
