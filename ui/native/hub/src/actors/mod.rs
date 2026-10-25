//! Actors own the app state and talk to Dart only through signals.

mod accounts;
mod geo;
mod models;
mod notifications;
mod proxy;
mod settings;

use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::DartSignal;
use tokio::spawn;

/// Forwards every Dart signal of type `S` to the actor.
async fn forward<S, A>(mut addr: Address<A>)
where
    S: DartSignal + Send + 'static,
    A: Actor + Notifiable<S> + 'static,
{
    let receiver = S::get_dart_signal_receiver();
    while let Some(pack) = receiver.recv().await {
        if addr.notify(pack.message).await.is_err() {
            break;
        }
    }
}

pub async fn create_actors() {
    let proxy_context = Context::new();
    let proxy_actor = proxy::ProxyActor::new(proxy_context.address());
    spawn(proxy_context.run(proxy_actor));

    let accounts_context = Context::new();
    let accounts_actor = accounts::AccountsActor::new(accounts_context.address());
    spawn(accounts_context.run(accounts_actor));

    let settings_context = Context::new();
    let settings_actor = settings::SettingsActor::new(settings_context.address());
    spawn(settings_context.run(settings_actor));

    let models_context = Context::new();
    let models_actor = models::ModelsActor::new(models_context.address());
    spawn(models_context.run(models_actor));

    spawn(notifications::run());

    let geo_context = Context::new();
    let geo_actor = geo::GeoActor::new(geo_context.address());
    spawn(geo_context.run(geo_actor));
}
