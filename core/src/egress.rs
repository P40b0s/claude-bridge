//! A local HTTP proxy (`CONNECT`, plus plain `http://` requests) for everything Claude Code sends
//! besides model calls: OAuth token refresh, profile and limits, MCP servers over HTTP, WebFetch.
//! Claude Code gets it as `HTTPS_PROXY`/`HTTP_PROXY`, with the bridge itself in `NO_PROXY`.
//!
//! Every connection leaves through the Anthropic exit after the Anthropic country check, so none
//! of that traffic shows the real address either. Refused connections get 451 or 502 with the
//! reason; nothing falls back to direct.
//!
//! `CLAUDE_BRIDGE_EGRESS_PORT` sets the port (the proxy's port + 1 by default); `0` turns it off.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::exits::{Exit, Exits};
use crate::{config, dns, geo, server, settings};

pub const PORT_KEY: &str = "CLAUDE_BRIDGE_EGRESS_PORT";
/// Claude Code's general traffic follows Anthropic. Browser logins also get a dedicated Codex
/// port, so the page and the token exchange leave through the same provider exit.
const DEFAULT_FAMILY: &str = "anthropic";
const BROWSER_FAMILIES: &[&str] = &["anthropic", "codex"];
const MAX_HEAD: usize = 32 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Tools Claude Code runs (npm, git) open many connections; one passed check covers this long.
/// A failed check is never reused, and model calls are checked every time regardless.
const CHECK_REUSE: Duration = Duration::from_secs(15);

/// Family + exit spec and when its country check last passed.
static PASSED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

async fn check_country(
    family: &str,
    exit: &Exit,
    client: &reqwest::Client,
) -> Result<(), crate::error::ProxyError> {
    let key = format!("{family}:{}", exit.spec());
    let fresh = PASSED
        .lock()
        .unwrap()
        .get(&key)
        .is_some_and(|at| at.elapsed() < CHECK_REUSE);
    if fresh {
        return Ok(());
    }
    if let Some(location) = geo::check(client, family, &server::family_origin(family)).await?
        && family == DEFAULT_FAMILY
    {
        crate::zone::remember(&location);
    }
    PASSED
        .lock()
        .unwrap()
        .insert(key, std::time::Instant::now());
    Ok(())
}

/// The port, or None when turned off.
pub fn port() -> Option<u16> {
    let value = settings::read()
        .remove(PORT_KEY)
        .filter(|v| !v.trim().is_empty())
        .or_else(|| config::env(PORT_KEY));
    match value.map(|v| v.trim().parse::<u16>()) {
        Some(Ok(0)) => None,
        Some(Ok(port)) => Some(port),
        _ => config::port().checked_add(1),
    }
}

/// Port for a family. Anthropic is the configured egress port; Codex is the next port.
pub fn family_port(family: &str) -> Option<u16> {
    let base = port()?;
    match family {
        DEFAULT_FAMILY => Some(base),
        "codex" => base.checked_add(1),
        _ => None,
    }
}

/// `http://127.0.0.1:<port>`, what Claude Code gets as its general Anthropic egress proxy.
pub fn url() -> Option<String> {
    family_url(DEFAULT_FAMILY)
}

/// Browser proxy for `family`, through that family's selected exit and country check.
pub fn family_url(family: &str) -> Option<String> {
    family_port(family).map(|p| format!("http://127.0.0.1:{p}"))
}

pub async fn serve(state: Arc<server::AppState>) {
    let mut tasks = tokio::task::JoinSet::new();
    for &family in BROWSER_FAMILIES {
        let Some(port) = family_port(family) else {
            continue;
        };
        let state = state.clone();
        tasks.spawn(async move { serve_family(state, family, port).await });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!("egress proxy task stopped: {e}");
        }
    }
}

async fn serve_family(state: Arc<server::AppState>, family: &'static str, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => return warn!("{family} egress proxy: cannot listen on 127.0.0.1:{port}: {e}"),
    };
    info!("{family} egress proxy on http://127.0.0.1:{port}");
    loop {
        let Ok((client, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(client, &state, family).await {
                debug!("{family} egress: {e:#}");
            }
        });
    }
}

struct Head {
    /// The bytes read so far: the request head and whatever followed it.
    buf: Vec<u8>,
    head_len: usize,
    connect: bool,
    host: String,
    port: u16,
}

async fn read_head(client: &mut TcpStream) -> anyhow::Result<Head> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    let head_len = loop {
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            bail!("client closed before the request head");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            bail!("request head too large");
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_len]).to_string();
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let (connect, host, port) = if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target, 443)?;
        (true, host, port)
    } else {
        let url =
            url::Url::parse(target).with_context(|| format!("not a proxy request: {line}"))?;
        anyhow::ensure!(
            url.scheme() == "http",
            "only http:// can be forwarded as-is; https goes through CONNECT"
        );
        let host = url
            .host_str()
            .context("no host")?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        (false, host, url.port().unwrap_or(80))
    };
    Ok(Head {
        buf,
        head_len,
        connect,
        host,
        port,
    })
}

fn split_host_port(target: &str, default: u16) -> anyhow::Result<(String, u16)> {
    if let Some(rest) = target.strip_prefix('[') {
        let (host, after) = rest.split_once(']').context("bad IPv6 target")?;
        let port = after
            .strip_prefix(':')
            .map(str::parse)
            .transpose()?
            .unwrap_or(default);
        return Ok((host.to_string(), port));
    }
    match target.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_string(), port.parse()?)),
        None => Ok((target.to_string(), default)),
    }
}

async fn refuse(client: &mut TcpStream, status: &str, message: &str) {
    let body = format!("claude-bridge: {message}\n");
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = client.write_all(reply.as_bytes()).await;
}

/// A refusal shows in the request log (and so as a warning in the UI), like a refused model call.
async fn refused(
    state: &server::AppState,
    client: &mut TcpStream,
    head: &Head,
    e: crate::error::ProxyError,
) {
    warn!(target_host = %head.host, "egress refused: {}", e.message);
    let status = format!(
        "{} {}",
        e.status.as_u16(),
        e.status.canonical_reason().unwrap_or("")
    );
    refuse(client, &status, &e.message).await;
    let tracker = state
        .log
        .begin(&format!("Claude Code → {}:{}", head.host, head.port), false);
    state.log.publish(tracker.failed(&e));
}

async fn handle(
    mut client: TcpStream,
    state: &server::AppState,
    family: &str,
) -> anyhow::Result<()> {
    let exits: &Exits = &state.exits;
    let head = read_head(&mut client).await?;
    let (exit, exit_client) = match exits.for_family(family).await {
        Ok(found) => found,
        Err(e) => {
            refused(state, &mut client, &head, e).await;
            return Ok(());
        }
    };
    // The same check as for model calls, through the same exit.
    if let Err(e) = check_country(family, &exit, &exit_client).await {
        refused(state, &mut client, &head, e).await;
        return Ok(());
    }
    let mut upstream = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        open(exits, &exit, &head.host, head.port),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            let message = format!(
                "cannot reach {}:{} through {}: {e:#}",
                head.host,
                head.port,
                exit.label()
            );
            warn!("egress: {message}");
            refuse(&mut client, "502 Bad Gateway", &message).await;
            return Ok(());
        }
        Err(_) => {
            let message = format!(
                "{}:{} through {} timed out",
                head.host,
                head.port,
                exit.label()
            );
            refuse(&mut client, "504 Gateway Timeout", &message).await;
            return Ok(());
        }
    };
    debug!(host = %head.host, port = head.port, exit = %exit.label(), "egress");
    if head.connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        upstream.write_all(&head.buf[head.head_len..]).await?;
    } else {
        // HTTP/1.1 servers accept the absolute form, so the request goes on unchanged.
        upstream.write_all(&head.buf).await?;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

/// A TCP stream to `host:port` through `exit`. Names go to the far side where there is one, and
/// over DoH when direct.
pub async fn open(exits: &Exits, exit: &Exit, host: &str, port: u16) -> anyhow::Result<TcpStream> {
    match exit {
        Exit::Direct => connect_direct(host, port).await,
        Exit::Ssh { .. } => {
            let local = exits.tunnel_port(exit).await?;
            let stream = TcpStream::connect(("127.0.0.1", local)).await?;
            socks5(stream, host, port, None).await
        }
        Exit::Proxy(spec) => {
            let url = url::Url::parse(spec)?;
            let proxy_host = url
                .host_str()
                .context("proxy has no host")?
                .trim_start_matches('[')
                .trim_end_matches(']');
            let proxy_port = url.port_or_known_default().unwrap_or(1080);
            let stream = connect_direct(proxy_host, proxy_port).await?;
            let user = (!url.username().is_empty()).then(|| {
                let decode = |s: &str| percent_decode(s);
                (
                    decode(url.username()),
                    decode(url.password().unwrap_or_default()),
                )
            });
            match url.scheme() {
                "socks5" | "socks5h" => socks5(stream, host, port, user).await,
                "http" => http_connect(stream, host, port, user).await,
                other => bail!(
                    "{other}:// proxies cannot carry Claude Code's other traffic; use socks5 or http"
                ),
            }
        }
    }
}

/// `%XX` in URL credentials; `+` stays a plus.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn resolve(host: &str, port: u16) -> anyhow::Result<Vec<SocketAddr>> {
    let ips: Vec<IpAddr> = if dns::doh_enabled() {
        dns::Doh::global().lookup(host).await?
    } else {
        tokio::net::lookup_host((host, port))
            .await?
            .map(|a| a.ip())
            .collect()
    };
    let ips: Vec<IpAddr> = ips
        .into_iter()
        .filter(|ip| ip.is_ipv4() || dns::ipv6_enabled())
        .collect();
    anyhow::ensure!(!ips.is_empty(), "{host} has no usable address");
    Ok(ips
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect())
}

async fn connect_direct(host: &str, port: u16) -> anyhow::Result<TcpStream> {
    let mut last = None;
    for addr in resolve(host, port).await? {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last = Some(e),
        }
    }
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("no address")))
}

/// SOCKS5 `CONNECT` by name (the far side resolves it), with optional user/password.
async fn socks5(
    mut s: TcpStream,
    host: &str,
    port: u16,
    auth: Option<(String, String)>,
) -> anyhow::Result<TcpStream> {
    let method = if auth.is_some() { 0x02 } else { 0x00 };
    s.write_all(&[0x05, 0x01, method]).await?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await?;
    anyhow::ensure!(
        reply[0] == 0x05 && reply[1] == method,
        "SOCKS proxy refused the login method"
    );
    if let Some((user, pass)) = auth {
        anyhow::ensure!(
            user.len() < 256 && pass.len() < 256,
            "SOCKS credentials too long"
        );
        let mut msg = vec![0x01, user.len() as u8];
        msg.extend_from_slice(user.as_bytes());
        msg.push(pass.len() as u8);
        msg.extend_from_slice(pass.as_bytes());
        s.write_all(&msg).await?;
        s.read_exact(&mut reply).await?;
        anyhow::ensure!(reply[1] == 0x00, "SOCKS proxy rejected the credentials");
    }
    let mut msg = vec![0x05, 0x01, 0x00];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            msg.push(0x01);
            msg.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            msg.push(0x04);
            msg.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            anyhow::ensure!(host.len() < 256, "host name too long");
            msg.push(0x03);
            msg.push(host.len() as u8);
            msg.extend_from_slice(host.as_bytes());
        }
    }
    msg.extend_from_slice(&port.to_be_bytes());
    s.write_all(&msg).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    anyhow::ensure!(
        head[1] == 0x00,
        "SOCKS connect to {host}:{port} failed (code {})",
        head[1]
    );
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await?;
            usize::from(len[0])
        }
        other => bail!("bad SOCKS address type {other}"),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await?;
    Ok(s)
}

async fn http_connect(
    mut s: TcpStream,
    host: &str,
    port: u16,
    auth: Option<(String, String)>,
) -> anyhow::Result<TcpStream> {
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some((user, pass)) = auth {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await?;
    // Read the reply head byte by byte so nothing of the tunnel is swallowed.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(head.len() < MAX_HEAD, "proxy reply too large");
        s.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&head)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    anyhow::ensure!(
        status.split_whitespace().nth(1) == Some("200"),
        "proxy answered {status}"
    );
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_connect_targets() {
        assert_eq!(
            split_host_port("api.anthropic.com:443", 443).unwrap(),
            ("api.anthropic.com".into(), 443)
        );
        assert_eq!(
            split_host_port("[2606:4700::1]:8443", 443).unwrap(),
            ("2606:4700::1".into(), 8443)
        );
        assert_eq!(
            split_host_port("example.com", 443).unwrap(),
            ("example.com".into(), 443)
        );
    }

    #[tokio::test]
    async fn socks5_sends_the_name_and_credentials() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let fake = tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut greet = [0u8; 3];
            s.read_exact(&mut greet).await.unwrap();
            assert_eq!(greet, [5, 1, 2]);
            s.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0u8; 2 + 1 + 1 + 2];
            s.read_exact(&mut auth).await.unwrap();
            assert_eq!(&auth, &[1, 1, b'u', 2, b'p', b'w']);
            s.write_all(&[1, 0]).await.unwrap();
            let mut req = vec![0u8; 4 + 1 + 17 + 2];
            s.read_exact(&mut req).await.unwrap();
            assert_eq!(&req[..5], &[5, 1, 0, 3, 17]);
            assert_eq!(&req[5..22], b"api.anthropic.com");
            assert_eq!(&req[22..], &443u16.to_be_bytes());
            s.write_all(&[5, 0, 0, 1, 10, 0, 0, 1, 0, 80, b'o', b'k'])
                .await
                .unwrap();
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut s = socks5(
            stream,
            "api.anthropic.com",
            443,
            Some(("u".into(), "pw".into())),
        )
        .await
        .unwrap();
        let mut tail = [0u8; 2];
        s.read_exact(&mut tail).await.unwrap();
        assert_eq!(&tail, b"ok", "bytes after the reply belong to the tunnel");
        fake.await.unwrap();
    }

    #[tokio::test]
    async fn http_connect_keeps_tunnel_bytes() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let fake = tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(
                req.starts_with("CONNECT chatgpt.com:443 HTTP/1.1\r\n"),
                "{req}"
            );
            assert!(
                req.contains("Proxy-Authorization: Basic dTpwdw==\r\n"),
                "{req}"
            );
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\nTLS").await.unwrap();
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut s = http_connect(stream, "chatgpt.com", 443, Some(("u".into(), "pw".into())))
            .await
            .unwrap();
        let mut tail = [0u8; 3];
        s.read_exact(&mut tail).await.unwrap();
        assert_eq!(&tail, b"TLS");
        fake.await.unwrap();
    }

    #[test]
    fn decodes_proxy_credentials() {
        assert_eq!(percent_decode("p%40ss%3Aword"), "p@ss:word");
        assert_eq!(percent_decode("a+b%2"), "a+b%2");
    }
}
