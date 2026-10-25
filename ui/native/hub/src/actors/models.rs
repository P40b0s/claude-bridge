use async_trait::async_trait;
use claude_bridge_core::models;
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::i18n::tr;
use crate::signals::{
    DeleteModel, ErrorSeverity, ErrorSignal, ModelsState, SaveModel, SavedModelInfo, SelectModel,
};

pub struct ModelsActor {
    _owned_tasks: JoinSet<()>,
}

impl Actor for ModelsActor {}

impl ModelsActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<SaveModel, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<DeleteModel, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<SelectModel, _>(self_addr));
        Self::send_state();
        Self {
            _owned_tasks: owned_tasks,
        }
    }

    fn send_state() {
        ModelsState {
            models: models::list()
                .into_iter()
                .map(|model| SavedModelInfo {
                    id: model.id,
                    name: model.name,
                    provider: model.target.provider.as_str().into(),
                    model: model.target.model,
                    reasoning: model.target.reasoning.map(|r| r.as_str().into()),
                    builtin: model.builtin,
                    selected: model.selected,
                })
                .collect(),
            file_path: models::file_path().display().to_string(),
        }
        .send_signal_to_dart();
    }

    fn report(result: anyhow::Result<()>, success: &str) {
        match result {
            Ok(()) => {
                ErrorSignal::send(ErrorSeverity::Success, success);
                Self::send_state();
            }
            Err(error) => ErrorSignal::error(&error),
        }
    }
}

#[async_trait]
impl Notifiable<SaveModel> for ModelsActor {
    async fn notify(&mut self, message: SaveModel, _: &Context<Self>) {
        let result = models::save(
            message.id,
            message.name,
            &message.provider,
            message.model,
            message.reasoning,
        )
        .map(|_| ());
        Self::report(result, tr("Модель сохранена", "Model saved"));
    }
}

#[async_trait]
impl Notifiable<DeleteModel> for ModelsActor {
    async fn notify(&mut self, message: DeleteModel, _: &Context<Self>) {
        Self::report(
            models::delete(&message.id),
            tr("Модель удалена", "Model deleted"),
        );
    }
}

#[async_trait]
impl Notifiable<SelectModel> for ModelsActor {
    async fn notify(&mut self, message: SelectModel, _: &Context<Self>) {
        // No toast: the dropdown already shows the choice, and the Home page
        // also saves it silently on startup.
        match models::select(&message.id) {
            Ok(_) => Self::send_state(),
            Err(error) => ErrorSignal::error(&error),
        }
    }
}
