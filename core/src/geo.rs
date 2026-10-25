//! Where the proxy's traffic leaves from, and the country allow-list that gates it.
//!
//! The policy lives in `~/.claude-bridge/.env` and is read on every check, so changes apply
//! without a restart.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

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

/// A fresh answer is reused this long.
const FRESH: Duration = Duration::from_secs(60);
/// When the lookup fails, an older answer is still trusted this long.
const STALE: Duration = Duration::from_secs(10 * 60);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub ip: String,
    /// Upper-case ISO 3166-1 alpha-2.
    pub country_code: String,
    /// English name from the lookup service.
    pub country: String,
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
}

impl Policy {
    pub fn load() -> Self {
        let file = settings::read();
        let get = |key: &str| file.get(key).cloned().filter(|v| !v.trim().is_empty()).or_else(|| config::env(key));
        let enabled = matches!(get(ENABLED_KEY).as_deref(), Some("1" | "true"));
        let allowed = match file.get(ALLOWED_KEY) {
            // Written by the UI; an explicit "-" means an empty list.
            Some(list) => parse_list(list),
            None => match config::env(ALLOWED_KEY) {
                Some(list) => parse_list(&list),
                None => COUNTRIES.iter().map(|(c, _)| c.to_string()).collect(),
            },
        };
        Self { enabled, allowed }
    }

    pub fn permits(&self, code: &str) -> bool {
        self.allowed.iter().any(|c| c.eq_ignore_ascii_case(code))
    }
}

fn parse_list(list: &str) -> Vec<String> {
    list.split(',').map(|c| c.trim().to_ascii_uppercase()).filter(|c| c.len() == 2).collect()
}

/// The value to store in `ALLOWED_KEY`. An empty list needs a marker: an empty value removes the key,
/// and a missing key means "all offered countries".
pub fn format_list(codes: &[String]) -> String {
    if codes.is_empty() { "-".into() } else { codes.join(",") }
}

#[derive(Deserialize)]
struct IpWho {
    ip: String,
    #[serde(default = "yes")]
    success: bool,
    #[serde(default)]
    country: String,
    #[serde(default)]
    country_code: String,
}

#[derive(Deserialize)]
struct CountryIs {
    ip: String,
    country: String,
}

fn yes() -> bool {
    true
}

/// Asks public geo-IP services where this machine's traffic appears to come from. Goes out the same
/// way the proxy's provider calls do, so a system or environment proxy is honored.
pub async fn lookup(http: &reqwest::Client) -> anyhow::Result<Location> {
    let first = async {
        let r: IpWho = http.get("https://ipwho.is/").timeout(LOOKUP_TIMEOUT).send().await?.error_for_status()?.json().await?;
        anyhow::ensure!(r.success && r.country_code.len() == 2, "ipwho.is gave no country");
        anyhow::Ok(Location { ip: r.ip, country_code: r.country_code.to_ascii_uppercase(), country: r.country })
    }
    .await;
    let first_err = match first {
        Ok(location) => return Ok(location),
        Err(e) => e,
    };
    let second = async {
        let r: CountryIs = http.get("https://api.country.is/").timeout(LOOKUP_TIMEOUT).send().await?.error_for_status()?.json().await?;
        anyhow::ensure!(r.country.len() == 2, "api.country.is gave no country");
        let code = r.country.to_ascii_uppercase();
        let country = country_name(&code).unwrap_or(&code).to_string();
        anyhow::Ok(Location { ip: r.ip, country_code: code, country })
    }
    .await;
    second.map_err(|e| anyhow::anyhow!("location lookup failed: {first_err:#}; {e:#}"))
}

/// Blocks requests while the machine is outside the allowed countries.
#[derive(Default)]
pub struct Guard {
    cache: Mutex<Option<(Location, Instant)>>,
}

impl Guard {
    /// Current location, cached for a minute; a failed lookup falls back to a recent answer.
    pub async fn locate(&self, http: &reqwest::Client) -> anyhow::Result<Location> {
        let mut cache = self.cache.lock().await;
        if let Some((location, at)) = cache.as_ref()
            && at.elapsed() < FRESH
        {
            return Ok(location.clone());
        }
        match lookup(http).await {
            Ok(location) => {
                *cache = Some((location.clone(), Instant::now()));
                Ok(location)
            }
            Err(e) => match cache.as_ref() {
                Some((location, at)) if at.elapsed() < STALE => Ok(location.clone()),
                _ => Err(e),
            },
        }
    }

    pub async fn check(&self, http: &reqwest::Client) -> Result<(), ProxyError> {
        let policy = Policy::load();
        if !policy.enabled {
            return Ok(());
        }
        let location = self.locate(http).await.map_err(|e| {
            ProxyError::new(StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, format!("country restriction is on, but the country is unknown: {e:#}"))
        })?;
        if policy.permits(&location.country_code) {
            return Ok(());
        }
        Err(ProxyError::new(
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            format!(
                "request not sent: the current IP {} is in {} ({}), which is not an allowed country",
                location.ip,
                location.display_country(),
                location.country_code
            ),
        ))
    }
}
