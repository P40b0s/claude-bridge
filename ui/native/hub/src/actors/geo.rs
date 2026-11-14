use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claude_bridge_core::exits::{self, Exit, Exits, Probe};
use claude_bridge_core::{auth, control, geo, server, settings};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::configuration;
use crate::i18n::tr;
use crate::signals::{
    CountryInfo, ErrorSignal, ExitInfo, GeoState, ProviderGeo, RefreshLocation, SetExits,
    SetGeoPolicy, SetProviderGeo,
};

const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Shows where each exit and provider comes out, and keeps the exits, routes and country
/// allow-lists the proxy enforces.
pub struct GeoActor {
    /// Used only while the proxy is down, for exits that need no SSH tunnel.
    local: Option<Arc<Exits>>,
    self_addr: Address<Self>,
    state: GeoState,
    /// Report of the last check, kept so settings edits redraw without a new check.
    report: Option<exits::Report>,
    _owned_tasks: JoinSet<()>,
}

impl Actor for GeoActor {}

/// Internal: check now.
pub struct Poll;

/// Internal: a check finished.
pub struct Checked(anyhow::Result<(exits::Report, bool)>);

impl GeoActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<SetGeoPolicy, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<SetProviderGeo, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<SetExits, _>(self_addr.clone()));
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
            exits: Vec::new(),
            providers: Vec::new(),
            proxy_running: false,
        };
        Self {
            local: server::http_client().ok().map(|http| Arc::new(Exits::new(http))),
            self_addr,
            state,
            report: None,
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

    /// Rebuilds the state from the settings and the last report, and sends it.
    fn send_state(&mut self) {
        // The shared policy: what a family without its own list gets.
        let shared = geo::Policy::shared();
        self.state.enabled = shared.enabled;
        self.state.allowed = shared.allowed;

        let probes: Vec<Probe> = self.report.as_ref().map(|r| r.exits.clone()).unwrap_or_default();
        let found = |spec: &str| probes.iter().find(|p| p.spec == spec);
        let mut all = vec![Exit::Direct];
        all.extend(exits::listed());
        self.state.exits = all
            .iter()
            .map(|exit| exit_info(exit, found(&exit.spec())))
            .collect();
        if let Some(direct) = self.state.exits.first() {
            self.state.ip = direct.ip.clone();
            self.state.country_code = direct.country_code.clone();
            self.state.country = direct.country.clone();
            if self.report.is_some() && self.state.error.is_none() {
                self.state.error = direct.error.as_ref().map(|e| {
                    format!(
                        "{}: {e}",
                        tr("Не удалось определить местоположение", "Location lookup failed")
                    )
                });
            }
        }

        let family_probes = self.report.as_ref().map(|r| r.families.clone()).unwrap_or_default();
        self.state.providers = exits::FAMILIES
            .iter()
            .filter(|f| !claude_bridge_core::families::is_disabled(f))
            .map(|family| {
                let policy = geo::Policy::load(family);
                let exit = exits::configured(family).unwrap_or(Exit::Direct);
                let probe = family_probes
                    .iter()
                    .find(|p| p.family == *family && p.probe.spec == exit.spec());
                ProviderGeo {
                    family: family.to_string(),
                    origin: probe
                        .map(|p| p.origin.clone())
                        .unwrap_or_else(|| server::family_origin(family)),
                    exit: exit_info(&exit, probe.map(|p| &p.probe)),
                    allowed: policy.allowed,
                    own: policy.own,
                }
            })
            .collect();
        self.state.clone().send_signal_to_dart();
    }

    /// Runs the check in the background so edits are not held up by a slow exit.
    fn start_check(&mut self) {
        if self.state.checking {
            return;
        }
        self.state.checking = true;
        self.send_state();
        let local = self.local.clone();
        let mut addr = self.self_addr.clone();
        tokio::spawn(async move {
            let port = configuration::port();
            let running = control::health(port).await.is_some();
            let from_proxy = match running {
                true => control::exits_report(port).await.ok(),
                false => None,
            };
            let result = if let Some(report) = from_proxy {
                Ok((report, true))
            } else {
                // Not running, or a version without /_exits: check from here (SSH exits need the proxy).
                match local {
                    Some(local) => Ok((local.report(server::family_origin, false).await, running)),
                    None => Err(anyhow::anyhow!(tr(
                        "не удалось создать HTTP-клиент",
                        "cannot create an HTTP client"
                    ))),
                }
            };
            let _ = addr.notify(Checked(result)).await;
        });
    }
}

fn exit_info(exit: &Exit, probe: Option<&Probe>) -> ExitInfo {
    let location = probe.and_then(|p| p.location.as_ref());
    ExitInfo {
        spec: exit.spec(),
        label: exit.label(),
        kind: match exit {
            Exit::Direct => "direct",
            Exit::Ssh { .. } => "ssh",
            Exit::Proxy(_) => "proxy",
        }
        .into(),
        ip: location.map(|l| l.ip.clone()),
        country_code: location.map(|l| l.country_code.clone()),
        country: location.map(|l| l.display_country().to_string()),
        error: probe.and_then(|p| p.error.clone()),
    }
}

fn save(updates: BTreeMap<String, Option<String>>) {
    if let Err(e) = settings::write(&updates) {
        ErrorSignal::error(&e);
    }
    super::settings::SettingsActor::send_state();
}

/// Keeps only codes the UI offers, in its order.
fn known_countries(codes: &[String]) -> Vec<String> {
    geo::COUNTRIES
        .iter()
        .map(|(code, _)| code.to_string())
        .filter(|code| codes.iter().any(|a| a.eq_ignore_ascii_case(code)))
        .collect()
}

#[async_trait]
impl Notifiable<Poll> for GeoActor {
    async fn notify(&mut self, _: Poll, _: &Context<Self>) {
        self.start_check();
    }
}

#[async_trait]
impl Notifiable<RefreshLocation> for GeoActor {
    async fn notify(&mut self, _: RefreshLocation, _: &Context<Self>) {
        self.start_check();
    }
}

#[async_trait]
impl Notifiable<Checked> for GeoActor {
    async fn notify(&mut self, Checked(result): Checked, _: &Context<Self>) {
        self.state.checking = false;
        match result {
            Ok((report, proxy_running)) => {
                self.report = Some(report);
                self.state.proxy_running = proxy_running;
                self.state.checked_at = Some(auth::now_ms());
                self.state.error = None;
            }
            Err(e) => {
                self.state.error = Some(format!(
                    "{}: {e:#}",
                    tr("Проверка не удалась", "Check failed")
                ))
            }
        }
        self.send_state();
    }
}

#[async_trait]
impl Notifiable<SetGeoPolicy> for GeoActor {
    async fn notify(&mut self, msg: SetGeoPolicy, _: &Context<Self>) {
        save(BTreeMap::from([
            (
                geo::ENABLED_KEY.to_string(),
                Some(if msg.enabled { "1" } else { "0" }.to_string()),
            ),
            (
                geo::ALLOWED_KEY.to_string(),
                Some(geo::format_list(&known_countries(&msg.allowed))),
            ),
        ]));
        self.send_state();
    }
}

#[async_trait]
impl Notifiable<SetProviderGeo> for GeoActor {
    async fn notify(&mut self, msg: SetProviderGeo, _: &Context<Self>) {
        if !exits::FAMILIES.contains(&msg.family.as_str()) {
            return;
        }
        let exit = match Exit::parse(&msg.exit) {
            Ok(exit) => exit,
            Err(e) => return ErrorSignal::error(&e),
        };
        let route = match exit {
            Exit::Direct => None,
            exit => Some(exit.spec()),
        };
        let changed_route = exits::configured(&msg.family).ok().map(|e| e.spec())
            != Some(route.clone().unwrap_or_else(|| "direct".into()));
        save(BTreeMap::from([
            (exits::route_key(&msg.family), route),
            (
                geo::allowed_key(&msg.family),
                msg.allowed.map(|list| geo::format_list(&known_countries(&list))),
            ),
        ]));
        self.send_state();
        if changed_route {
            self.start_check();
        }
    }
}

#[async_trait]
impl Notifiable<SetExits> for GeoActor {
    async fn notify(&mut self, msg: SetExits, _: &Context<Self>) {
        let mut list: Vec<Exit> = Vec::new();
        for spec in msg.exits.iter().filter(|s| !s.trim().is_empty()) {
            match Exit::parse(spec) {
                Ok(Exit::Direct) => {}
                Ok(exit) if !list.contains(&exit) => list.push(exit),
                Ok(_) => {}
                Err(e) => return ErrorSignal::error(&e),
            }
        }
        // Removing an exit a provider uses would quietly send that provider direct.
        for family in exits::FAMILIES {
            if let Ok(exit @ (Exit::Ssh { .. } | Exit::Proxy(_))) = exits::configured(family)
                && !list.contains(&exit)
            {
                ErrorSignal::send(
                    crate::signals::ErrorSeverity::Warning,
                    format!(
                        "{} {} {family}: {}",
                        exit.label(),
                        tr("используется провайдером", "is used by"),
                        tr(
                            "сначала выберите ему другой выход",
                            "pick another exit for it first"
                        )
                    ),
                );
                self.send_state();
                return;
            }
        }
        save(BTreeMap::from([(
            exits::EXITS_KEY.to_string(),
            Some(exits::format_list(&list)).filter(|v| !v.is_empty()),
        )]));
        self.send_state();
        self.start_check();
    }
}
