use async_trait::async_trait;
use claude_bridge_core::stats::{self, Tokens};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::signals::{ErrorSignal, StatsDay, StatsModel, StatsReport, StatsRequest, StatsTokens};

pub struct StatsActor {
    _owned_tasks: JoinSet<()>,
}

impl Actor for StatsActor {}

impl StatsActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<StatsRequest, _>(self_addr));
        Self { _owned_tasks: owned_tasks }
    }
}

fn tokens(t: &Tokens) -> StatsTokens {
    let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    StatsTokens {
        requests: n(t.requests),
        input: n(t.input),
        cache_read: n(t.cache_read),
        cache_write: n(t.cache_write),
        output: n(t.output),
    }
}

fn all(t: &Tokens) -> u64 {
    t.input + t.cache_read + t.cache_write + t.output
}

#[async_trait]
impl Notifiable<StatsRequest> for StatsActor {
    async fn notify(&mut self, request: StatsRequest, _: &Context<Self>) {
        let parse = |s: &str| s.parse::<jiff::civil::Date>().map_err(|e| anyhow::anyhow!("{s}: {e}"));
        let (from, to) = match (parse(&request.from), parse(&request.to)) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(e), _) | (_, Err(e)) => return ErrorSignal::error(&e),
        };
        let days = tokio::task::spawn_blocking(move || stats::range(from, to)).await.unwrap_or_default();
        let mut total = Tokens::default();
        let mut models = std::collections::BTreeMap::<String, Tokens>::new();
        for (_, day) in &days {
            for (model, t) in &day.models {
                models.entry(model.clone()).or_default().add(t);
                total.add(t);
            }
        }
        let mut models: Vec<_> = models.into_iter().collect();
        models.sort_by_key(|(_, t)| std::cmp::Reverse(all(t)));
        StatsReport {
            from: request.from,
            to: request.to,
            total: tokens(&total),
            days: days.iter().map(|(date, day)| StatsDay { date: date.to_string(), tokens: tokens(&day.total()) }).collect(),
            models: models.iter().map(|(model, t)| StatsModel { model: model.clone(), tokens: tokens(t) }).collect(),
            dir: stats::dir().display().to_string(),
        }
        .send_signal_to_dart();
    }
}
