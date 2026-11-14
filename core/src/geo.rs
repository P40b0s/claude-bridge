//! Where each provider's traffic leaves from, and the per-provider country allow-lists that gate it.
//! The country comes from Cloudflare's `/cdn-cgi/trace` on the provider's own host.
//!
//! The policy lives in `~/.claude-bridge/.env` and is read on every check, so changes apply
//! without a restart.

use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::error::ProxyError;
use crate::{config, settings};

pub const ENABLED_KEY: &str = "GEO_GUARD_ENABLED";
pub const ALLOWED_KEY: &str = "GEO_ALLOWED_COUNTRIES";

/// Countries the UI offers, with ISO 3166-1 alpha-2 codes and English names (the UI translates
/// them). All are allowed until the list is edited.
pub const COUNTRIES: &[(&str, &str)] = &[
    ("DE", "Germany"),
    ("NL", "Netherlands"),
    ("SE", "Sweden"),
    ("EE", "Estonia"),
    ("LV", "Latvia"),
    ("GB", "United Kingdom"),
    ("FR", "France"),
    ("LT", "Lithuania"),
    ("CH", "Switzerland"),
    ("PL", "Poland"),
    ("US", "United States"),
];

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub ip: String,
    /// Upper-case ISO 3166-1 alpha-2.
    pub country_code: String,
    /// English name.
    pub country: String,
    /// The host whose trace page answered.
    #[serde(default)]
    pub origin: String,
}

impl Location {
    /// Name from `COUNTRIES`, the service's name otherwise.
    pub fn display_country(&self) -> &str {
        country_name(&self.country_code).unwrap_or(&self.country)
    }
}

pub fn country_name(code: &str) -> Option<&'static str> {
    COUNTRIES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map(|(_, n)| *n)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    pub enabled: bool,
    /// Upper-case codes.
    pub allowed: Vec<String>,
    /// The family has its own list (`GEO_ALLOWED_<FAMILY>`) rather than the shared one.
    pub own: bool,
}

/// `GEO_ALLOWED_OPENAI_COMPAT` for `openai-compat`.
pub fn allowed_key(family: &str) -> String {
    format!("GEO_ALLOWED_{}", family.to_ascii_uppercase().replace('-', "_"))
}

impl Policy {
    /// The policy for one provider family: its own list if set, the shared one otherwise.
    pub fn load(family: &str) -> Self {
        Self::read(Some(family))
    }

    /// The shared list, for families without their own.
    pub fn shared() -> Self {
        Self::read(None)
    }

    fn read(family: Option<&str>) -> Self {
        let file = settings::read();
        let get = |key: &str| file.get(key).cloned().filter(|v| !v.trim().is_empty()).or_else(|| config::env(key));
        let enabled = matches!(get(ENABLED_KEY).as_deref(), Some("1" | "true"));
        // Written by the UI; an explicit "-" means an empty list.
        let list = |key: &str| file.get(key).cloned().or_else(|| config::env(key));
        let (allowed, own) = match family.and_then(|f| list(&allowed_key(f))) {
            Some(own) => (parse_list(&own), true),
            None => match list(ALLOWED_KEY) {
                Some(shared) => (parse_list(&shared), false),
                None => (COUNTRIES.iter().map(|(c, _)| c.to_string()).collect(), false),
            },
        };
        Self { enabled, allowed, own }
    }

    pub fn permits(&self, code: &str) -> bool {
        self.allowed.iter().any(|c| c.eq_ignore_ascii_case(code))
    }
}

fn parse_list(list: &str) -> Vec<String> {
    list.split(',').map(|c| c.trim().to_ascii_uppercase()).filter(|c| c.len() == 2).collect()
}

/// The value to store in an allow-list key. An empty list needs a marker: an empty value removes
/// the key, and a missing key means "the shared list" (or every offered country).
pub fn format_list(codes: &[String]) -> String {
    if codes.is_empty() { "-".into() } else { codes.join(",") }
}

/// Neutral Cloudflare host, for exits checked without a provider (and hosts without a trace page).
pub const NEUTRAL_ORIGIN: &str = "https://www.cloudflare.com";

/// Asks a Cloudflare-fronted host where the request comes from, through its `/cdn-cgi/trace`
/// page (`ip=…`, `loc=XX`). Asked of the provider's own host, it goes the same way as the real
/// call, so domain-based routing rules (VPN client, router) apply to it too. `None` when the host
/// has no such page.
pub async fn trace(http: &reqwest::Client, origin: &str) -> anyhow::Result<Option<Location>> {
    let resp = http.get(format!("{}/cdn-cgi/trace", origin.trim_end_matches('/'))).timeout(LOOKUP_TIMEOUT).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    Ok(parse_trace(&resp.text().await?))
}

fn parse_trace(body: &str) -> Option<Location> {
    let field = |key: &str| body.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim);
    let ip = field("ip")?.to_string();
    let code = field("loc").filter(|c| c.len() == 2)?.to_ascii_uppercase();
    let country = country_name(&code).unwrap_or(&code).to_string();
    Some(Location { ip, country_code: code, country, origin: String::new() })
}

/// Where a request to `origin` leaves from: the host's own trace page, or Cloudflare's through the
/// same client when the host has none (exact behind an SSH or proxy exit, approximate when direct).
pub async fn locate(http: &reqwest::Client, origin: &str) -> anyhow::Result<Location> {
    let hosts = if origin == NEUTRAL_ORIGIN { vec![origin] } else { vec![origin, NEUTRAL_ORIGIN] };
    for host in hosts {
        match trace(http, host).await {
            Ok(Some(location)) => return Ok(Location { origin: host.to_string(), ..location }),
            Ok(None) => {}
            Err(e) => anyhow::bail!("{host} is unreachable: {e:#}"),
        }
    }
    anyhow::bail!("{NEUTRAL_ORIGIN}/cdn-cgi/trace gave no country")
}

/// Checks the country before every request, with no caching: the route can change at any moment.
/// Off unless the policy is enabled.
pub async fn check(http: &reqwest::Client, family: &str, origin: &str) -> Result<Option<Location>, ProxyError> {
    let policy = Policy::load(family);
    if !policy.enabled {
        return Ok(None);
    }
    let location = locate(http, origin).await.map_err(|e| {
        ProxyError::new(StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, format!("country restriction is on, but the country is unknown: {e:#}"))
    })?;
    if policy.permits(&location.country_code) {
        return Ok(Some(location));
    }
    Err(ProxyError::new(
        StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
        format!(
            "request not sent: {} sees IP {} in {} ({}), which is not allowed for {family}",
            location.origin,
            location.ip,
            location.display_country(),
            location.country_code
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cloudflare_trace() {
        let body = "fl=1\nh=api.anthropic.com\nip=185.191.118.89\nts=1\ncolo=FRA\nloc=de\ntls=TLSv1.3\n";
        let l = parse_trace(body).unwrap();
        assert_eq!((l.ip.as_str(), l.country_code.as_str(), l.country.as_str()), ("185.191.118.89", "DE", "Germany"));
    }

    #[test]
    fn rejects_page_without_country() {
        assert!(parse_trace("<html>not found</html>").is_none());
        assert!(parse_trace("ip=1.2.3.4\nloc=XX1\n").is_none());
    }
}
