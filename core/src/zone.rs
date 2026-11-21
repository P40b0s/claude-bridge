//! `CLAUDE_BRIDGE_TZ=auto`: Claude Code and the commands it runs get the time zone of the country
//! the Anthropic exit shows, so git commit times and `date` match the address the traffic leaves
//! from. The proxy remembers that country after each check; an unknown country gives `UTC`.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::{config, exits, geo, server};

const FAMILY: &str = "anthropic";
/// With the country restriction off nothing is checked per request, so the proxy looks again
/// this often.
const RELOOK: Duration = Duration::from_secs(10 * 60);

/// The zone for a country: the capital's where a country has several.
pub fn for_country(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_uppercase().as_str() {
        "DE" => "Europe/Berlin",
        "NL" => "Europe/Amsterdam",
        "SE" => "Europe/Stockholm",
        "EE" => "Europe/Tallinn",
        "LV" => "Europe/Riga",
        "LT" => "Europe/Vilnius",
        "GB" => "Europe/London",
        "IE" => "Europe/Dublin",
        "FR" => "Europe/Paris",
        "CH" => "Europe/Zurich",
        "PL" => "Europe/Warsaw",
        "FI" => "Europe/Helsinki",
        "NO" => "Europe/Oslo",
        "DK" => "Europe/Copenhagen",
        "AT" => "Europe/Vienna",
        "CZ" => "Europe/Prague",
        "SK" => "Europe/Bratislava",
        "BE" => "Europe/Brussels",
        "LU" => "Europe/Luxembourg",
        "ES" => "Europe/Madrid",
        "PT" => "Europe/Lisbon",
        "IT" => "Europe/Rome",
        "HU" => "Europe/Budapest",
        "RO" => "Europe/Bucharest",
        "BG" => "Europe/Sofia",
        "RS" => "Europe/Belgrade",
        "GR" => "Europe/Athens",
        "MD" => "Europe/Chisinau",
        "UA" => "Europe/Kyiv",
        "TR" => "Europe/Istanbul",
        "GE" => "Asia/Tbilisi",
        "AM" => "Asia/Yerevan",
        "KZ" => "Asia/Almaty",
        "AE" => "Asia/Dubai",
        "IL" => "Asia/Jerusalem",
        "JP" => "Asia/Tokyo",
        "SG" => "Asia/Singapore",
        "HK" => "Asia/Hong_Kong",
        "KR" => "Asia/Seoul",
        "IN" => "Asia/Kolkata",
        "US" => "America/New_York",
        "CA" => "America/Toronto",
        "BR" => "America/Sao_Paulo",
        "AU" => "Australia/Sydney",
        _ => return None,
    })
}

fn path() -> std::path::PathBuf {
    config::data_dir().join("exit-country")
}

/// The Anthropic exit's country as last seen.
pub fn remembered() -> Option<String> {
    std::fs::read_to_string(path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// What `auto` turns into.
pub fn auto() -> String {
    remembered()
        .as_deref()
        .and_then(for_country)
        .unwrap_or("UTC")
        .to_string()
}

fn wanted() -> bool {
    crate::claude_settings::time_zone_setting().is_some_and(|v| v.eq_ignore_ascii_case("auto"))
}

/// Records the country the Anthropic exit shows. On a change, a connected `~/.claude/settings.json`
/// gets the new zone for the next Claude Code started.
pub fn remember(location: &geo::Location) {
    if !wanted() || remembered().as_deref() == Some(location.country_code.as_str()) {
        return;
    }
    if let Err(e) = std::fs::write(path(), &location.country_code) {
        return warn!("cannot save the exit country: {e}");
    }
    debug!(country = %location.country_code, "time zone follows the exit");
    if let Err(e) = crate::claude_settings::sync_auth(config::port()) {
        warn!("{e:#}");
    }
}

/// The country of a family's exit, whether or not the restriction is on. For the launcher.
pub async fn locate_outside() -> anyhow::Result<geo::Location> {
    let client = exits::outside_client_unchecked(FAMILY).await?;
    geo::locate(&client, &server::family_origin(FAMILY)).await
}

/// In the proxy: keeps the country current while nothing else checks it.
pub async fn watch(state: Arc<server::AppState>) {
    loop {
        if wanted() {
            match state.exits.for_family(FAMILY).await {
                Ok((_, client)) => match geo::locate(&client, &server::family_origin(FAMILY)).await
                {
                    Ok(location) => remember(&location),
                    Err(e) => debug!("time zone: exit country unknown: {e:#}"),
                },
                Err(e) => debug!("time zone: {}", e.message),
            }
        }
        tokio::time::sleep(RELOOK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_country_has_a_zone() {
        for (code, _) in geo::COUNTRIES {
            assert!(for_country(code).is_some(), "{code}");
        }
        assert_eq!(for_country("de"), Some("Europe/Berlin"));
        assert_eq!(for_country("ZZ"), None);
    }
}
