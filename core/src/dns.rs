//! DNS over HTTPS, so the names of the providers (and of the SSH exits) never reach the system
//! resolver, the router or the ISP in clear text.
//!
//! Only direct connections need it: behind a SOCKS or SSH exit names are resolved on the far side
//! (`socks5h`), and an HTTP proxy gets the name in `CONNECT`. The resolvers are asked by IP, so
//! asking them needs no DNS itself. When none answers, the lookup fails; there is no quiet fallback
//! to the system resolver. `CLAUDE_BRIDGE_DNS=system` turns this off; `CLAUDE_BRIDGE_DOH_SERVERS`
//! picks the resolvers.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;
use tracing::debug;

use crate::{config, settings};

pub const MODE_KEY: &str = "CLAUDE_BRIDGE_DNS";
pub const SERVERS_KEY: &str = "CLAUDE_BRIDGE_DOH_SERVERS";
pub const IPV6_KEY: &str = "CLAUDE_BRIDGE_IPV6";

/// JSON DoH endpoints (`?name=…&type=A|AAAA`, `application/dns-json`), tried in order.
const DEFAULT_SERVERS: &str = "https://1.1.1.1/dns-query,https://8.8.8.8/resolve";
const TIMEOUT: Duration = Duration::from_secs(5);
const MIN_TTL: u64 = 30;
const MAX_TTL: u64 = 600;

/// DoH unless `CLAUDE_BRIDGE_DNS=system`. Read when a client is built, so a change applies after a
/// restart.
pub fn doh_enabled() -> bool {
    !setting(MODE_KEY).is_some_and(|v| v.trim().eq_ignore_ascii_case("system"))
}

/// `CLAUDE_BRIDGE_IPV6=0`: connect over IPv4 only, for a VPN that tunnels IPv4 and lets IPv6 go
/// around it.
pub fn ipv6_enabled() -> bool {
    setting(IPV6_KEY).is_none_or(|v| v.trim() != "0")
}

fn setting(key: &str) -> Option<String> {
    settings::read().remove(key).filter(|v| !v.trim().is_empty()).or_else(|| config::env(key))
}

/// The resolvers from `CLAUDE_BRIDGE_DOH_SERVERS`, comma-separated. Each must name its host by IP:
/// reaching a resolver by name would need the very DNS this avoids.
pub fn servers() -> anyhow::Result<Vec<url::Url>> {
    parse_servers(&setting(SERVERS_KEY).unwrap_or_else(|| DEFAULT_SERVERS.into()))
}

fn parse_servers(list: &str) -> anyhow::Result<Vec<url::Url>> {
    let mut out = Vec::new();
    for spec in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let url = url::Url::parse(spec).map_err(|e| anyhow::anyhow!("{SERVERS_KEY}: bad URL {spec:?}: {e}"))?;
        anyhow::ensure!(url.scheme() == "https", "{SERVERS_KEY}: {spec} is not https");
        anyhow::ensure!(
            matches!(url.host(), Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))),
            "{SERVERS_KEY}: {spec} must name its host by IP (https://1.1.1.1/dns-query), not by name"
        );
        out.push(url);
    }
    anyhow::ensure!(!out.is_empty(), "{SERVERS_KEY} lists no servers");
    Ok(out)
}

pub struct Doh {
    http: reqwest::Client,
    /// Read once per process; a bad setting makes every lookup fail with its message.
    servers: Result<Vec<url::Url>, String>,
    cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
}

impl Doh {
    /// One per process, so the cache is shared by every client.
    pub fn global() -> Arc<Doh> {
        static DOH: OnceLock<Arc<Doh>> = OnceLock::new();
        DOH.get_or_init(|| {
            // The endpoints are IPs: this client never resolves anything.
            let http = reqwest::Client::builder().no_proxy().timeout(TIMEOUT).build().unwrap_or_default();
            Arc::new(Doh { http, servers: servers().map_err(|e| format!("{e:#}")), cache: Mutex::default() })
        })
        .clone()
    }

    /// IPv4 first: a broken IPv6 route is more common than a broken IPv4 one.
    pub async fn lookup(&self, name: &str) -> anyhow::Result<Vec<IpAddr>> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if let Ok(ip) = name.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        if name == "localhost" {
            return Ok(vec![IpAddr::from([127, 0, 0, 1])]);
        }
        let cached = self.cache.lock().unwrap().get(&name).filter(|(_, until)| Instant::now() < *until).map(|(ips, _)| ips.clone());
        let ips = match cached {
            Some(ips) => ips,
            None => self.lookup_all(&name).await?,
        };
        let ips: Vec<IpAddr> = ips.into_iter().filter(|ip| ip.is_ipv4() || ipv6_enabled()).collect();
        anyhow::ensure!(!ips.is_empty(), "{name} has no IPv4 address ({IPV6_KEY}=0)");
        Ok(ips)
    }

    async fn lookup_all(&self, name: &str) -> anyhow::Result<Vec<IpAddr>> {
        let name = name.to_string();
        let servers = self.servers.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut errors = Vec::new();
        for server in servers {
            let (v4, v6) = tokio::join!(self.query(server, &name, "A"), self.query(server, &name, "AAAA"));
            match (v4, v6) {
                (Err(e), Err(_)) => errors.push(format!("{server}: {e:#}")),
                (v4, v6) => {
                    let (mut ips, mut ttl) = v4.unwrap_or_default();
                    let (more, ttl6) = v6.unwrap_or_default();
                    ips.extend(more);
                    if ips.is_empty() {
                        anyhow::bail!("DNS over HTTPS: {name} has no address");
                    }
                    if ttl == 0 {
                        ttl = ttl6;
                    }
                    let ttl = Duration::from_secs(ttl.clamp(MIN_TTL, MAX_TTL));
                    debug!(%name, ?ips, "resolved over https");
                    self.cache.lock().unwrap().insert(name, (ips.clone(), Instant::now() + ttl));
                    return Ok(ips);
                }
            }
        }
        anyhow::bail!(
            "DNS over HTTPS failed for {name} ({}); set {MODE_KEY}=system to use the system resolver",
            errors.join("; ")
        )
    }

    /// Addresses of one record type and the smallest TTL among them.
    async fn query(&self, server: &url::Url, name: &str, kind: &str) -> anyhow::Result<(Vec<IpAddr>, u64)> {
        let mut url = server.clone();
        url.query_pairs_mut().append_pair("name", name).append_pair("type", kind);
        let body: Value = self
            .http
            .get(url)
            .header("accept", "application/dns-json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(body["Status"].as_i64() == Some(0) || body["Status"].as_i64() == Some(3), "status {}", body["Status"]);
        Ok(parse_answers(&body, if kind == "A" { 1 } else { 28 }))
    }
}

fn parse_answers(body: &Value, record: i64) -> (Vec<IpAddr>, u64) {
    let mut ttl = u64::MAX;
    let ips: Vec<IpAddr> = body["Answer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["type"].as_i64() == Some(record))
        .filter_map(|a| {
            let ip = a["data"].as_str()?.parse().ok()?;
            ttl = ttl.min(a["TTL"].as_u64().unwrap_or(MIN_TTL));
            Some(ip)
        })
        .collect();
    (ips, if ttl == u64::MAX { 0 } else { ttl })
}

/// reqwest's hook: every name a client connects to goes through `Doh`.
pub struct DohResolver(pub Arc<Doh>);

impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let doh = self.0.clone();
        let name = name.as_str().to_string();
        Box::pin(async move {
            let ips = doh.lookup(&name).await?;
            Ok(Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0))) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn servers_must_be_https_by_ip() {
        assert_eq!(parse_servers(DEFAULT_SERVERS).unwrap().len(), 2);
        assert!(parse_servers("https://[2606:4700:4700::1111]/dns-query").is_ok());
        assert!(parse_servers("https://dns.google/resolve").is_err());
        assert!(parse_servers("http://1.1.1.1/dns-query").is_err());
        assert!(parse_servers(" , ").is_err());
    }

    #[test]
    fn reads_answers_of_the_asked_type() {
        let body = json!({ "Status": 0, "Answer": [
            { "type": 5, "TTL": 60, "data": "edge.example." },
            { "type": 1, "TTL": 247, "data": "160.79.104.10" },
            { "type": 1, "TTL": 120, "data": "160.79.104.11" }
        ]});
        let (ips, ttl) = parse_answers(&body, 1);
        assert_eq!(ips, vec!["160.79.104.10".parse::<IpAddr>().unwrap(), "160.79.104.11".parse().unwrap()]);
        assert_eq!(ttl, 120);
        assert_eq!(parse_answers(&body, 28), (vec![], 0));
    }
}
