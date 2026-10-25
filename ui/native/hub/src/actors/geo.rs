use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use claude_bridge_core::{auth, geo, server, settings};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::i18n::tr;
use crate::signals::{CountryInfo, ErrorSignal, GeoState, RefreshLocation, SetGeoPolicy};

const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Shows where the traffic leaves from and keeps the country allow-list the proxy enforces.
pub struct GeoActor {
    http: Option<reqwest::Client>,
    self_addr: Address<Self>,
    state: GeoState,
    _owned_tasks: JoinSet<()>,
}

impl Actor for GeoActor {}

/// Internal: look the location up now.
pub struct Poll;

/// Internal: a lookup finished.
pub struct Located(anyhow::Result<geo::Location>);

impl GeoActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<SetGeoPolicy, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<RefreshLocation, _>(self_addr.clone()));
        owned_tasks.spawn(Self::poll_timer(self_addr.clone()));
        let countries = geo::COUNTRIES
            .iter()
            .map(|(code, name)| CountryInfo {
                code: code.to_string(),
                name: name.to_string(),
            })
            .collect();
        let state = GeoState {
            ip: None,
            country_code: None,
            country: None,
            error: None,
            checking: false,
            checked_at: None,
            enabled: false,
            allowed: Vec::new(),
            countries,
        };
        Self {
            http: server::http_client().ok(),
            self_addr,
            state,
            _owned_tasks: owned_tasks,
        }
    }

    async fn poll_timer(mut self_addr: Address<Self>) {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            if self_addr.notify(Poll).await.is_err() {
                break;
            }
        }
    }

    fn send_state(&mut self) {
        let policy = geo::Policy::load();
        self.state.enabled = policy.enabled;
        self.state.allowed = policy.allowed;
        self.state.clone().send_signal_to_dart();
    }

    /// Runs the lookup in the background so policy edits are not held up by a slow service.
    fn start_lookup(&mut self) {
        if self.state.checking {
            return;
        }
        let Some(http) = self.http.clone() else {
            self.state.error = Some(
                tr(
                    "не удалось создать HTTP-клиент",
                    "cannot create an HTTP client",
                )
                .into(),
            );
            self.send_state();
            return;
        };
        self.state.checking = true;
        self.send_state();
        let mut addr = self.self_addr.clone();
        tokio::spawn(async move {
            let result = geo::lookup(&http).await;
            let _ = addr.notify(Located(result)).await;
        });
    }
}

/// The services' own errors, after core's "location lookup failed" prefix.
fn detail(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    format!(
        ": {}",
        text.strip_prefix("location lookup failed: ")
            .unwrap_or(&text)
    )
}

#[async_trait]
impl Notifiable<Poll> for GeoActor {
    async fn notify(&mut self, _: Poll, _: &Context<Self>) {
        self.start_lookup();
    }
}

#[async_trait]
impl Notifiable<RefreshLocation> for GeoActor {
    async fn notify(&mut self, _: RefreshLocation, _: &Context<Self>) {
        self.start_lookup();
    }
}

#[async_trait]
impl Notifiable<Located> for GeoActor {
    async fn notify(&mut self, Located(result): Located, _: &Context<Self>) {
        self.state.checking = false;
        match result {
            Ok(location) => {
                self.state.country = Some(location.display_country().to_string());
                self.state.ip = Some(location.ip);
                self.state.country_code = Some(location.country_code);
                self.state.checked_at = Some(auth::now_ms());
                self.state.error = None;
            }
            Err(e) => {
                self.state.error = Some(
                    tr(
                        "Не удалось определить местоположение",
                        "Location lookup failed",
                    )
                    .to_string()
                        + &detail(&e),
                )
            }
        }
        self.send_state();
    }
}

#[async_trait]
impl Notifiable<SetGeoPolicy> for GeoActor {
    async fn notify(&mut self, msg: SetGeoPolicy, _: &Context<Self>) {
        let allowed: Vec<String> = geo::COUNTRIES
            .iter()
            .map(|(code, _)| code.to_string())
            .filter(|code| msg.allowed.iter().any(|a| a.eq_ignore_ascii_case(code)))
            .collect();
        let updates = BTreeMap::from([
            (
                geo::ENABLED_KEY.to_string(),
                Some(if msg.enabled { "1" } else { "0" }.to_string()),
            ),
            (
                geo::ALLOWED_KEY.to_string(),
                Some(geo::format_list(&allowed)),
            ),
        ]);
        if let Err(e) = settings::write(&updates) {
            ErrorSignal::error(&e);
        }
        super::settings::SettingsActor::send_state();
        self.send_state();
    }
}
