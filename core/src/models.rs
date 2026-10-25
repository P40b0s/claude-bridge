//! Persistent user model registry (`~/.claude-bridge/models.json`) combined with built-in presets.

use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::routing::{self, Provider, Reasoning, Target};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomModel {
    pub id: String,
    pub name: String,
    pub provider: Provider,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

impl CustomModel {
    pub fn target(&self) -> Target {
        Target::new(self.provider, self.model.clone(), self.reasoning)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ModelsFile {
    #[serde(default)]
    models: Vec<CustomModel>,
    /// Model selected in the Home page dropdown (`builtin:<alias>` or a custom UUID).
    #[serde(default)]
    selected: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryModel {
    pub id: String,
    pub name: String,
    pub target: Target,
    pub builtin: bool,
    pub selected: bool,
}

pub fn file_path() -> PathBuf {
    config::data_dir().join("models.json")
}

fn read_file() -> ModelsFile {
    std::fs::read(file_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_file(file: &ModelsFile) -> anyhow::Result<()> {
    let path = file_path();
    std::fs::create_dir_all(config::data_dir())?;
    let mut text = serde_json::to_string_pretty(file)?;
    text.push('\n');
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&tmp)?.write_all(text.as_bytes())?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
}

pub fn list() -> Vec<RegistryModel> {
    let file = read_file();
    let mut out = Vec::new();
    for preset in routing::PRESETS {
        if let Ok(target) = routing::resolve(preset.alias, None) {
            let id = format!("builtin:{}", preset.alias);
            out.push(RegistryModel {
                selected: file.selected.as_deref() == Some(&id),
                id,
                name: preset.description.into(),
                target,
                builtin: true,
            });
        }
    }
    for model in file.models {
        out.push(RegistryModel {
            selected: file.selected.as_deref() == Some(&model.id),
            id: model.id.clone(),
            name: model.name.clone(),
            target: model.target(),
            builtin: false,
        });
    }
    out
}

pub fn save(
    id: Option<String>,
    name: String,
    provider: &str,
    model: String,
    reasoning: Option<String>,
) -> anyhow::Result<CustomModel> {
    let name = name.trim();
    let model = model.trim();
    if name.is_empty() || model.is_empty() {
        bail!("model name and model ID are required");
    }
    let provider = Provider::parse(provider).ok_or_else(|| anyhow::anyhow!("unknown provider: {provider}"))?;
    let reasoning = match reasoning.as_deref().filter(|value| !value.trim().is_empty()) {
        Some(value) => Some(Reasoning::parse(value).ok_or_else(|| anyhow::anyhow!("unknown reasoning effort: {value}"))?),
        None => None,
    };
    let mut file = read_file();
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if id.starts_with("builtin:") {
        bail!("built-in models cannot be edited");
    }
    let entry = CustomModel { id: id.clone(), name: name.into(), provider, model: model.into(), reasoning };
    if let Some(existing) = file.models.iter_mut().find(|m| m.id == id) {
        *existing = entry.clone();
    } else {
        file.models.push(entry.clone());
    }
    write_file(&file)?;
    Ok(entry)
}

pub fn delete(id: &str) -> anyhow::Result<()> {
    if id.starts_with("builtin:") {
        bail!("built-in models cannot be deleted");
    }
    let mut file = read_file();
    let before = file.models.len();
    file.models.retain(|model| model.id != id);
    if file.models.len() == before {
        bail!("model not found");
    }
    if file.selected.as_deref() == Some(id) {
        file.selected = None;
    }
    write_file(&file)
}

pub fn select(id: &str) -> anyhow::Result<Target> {
    let model = list().into_iter().find(|model| model.id == id).ok_or_else(|| anyhow::anyhow!("model not found"))?;
    let mut file = read_file();
    file.selected = Some(id.into());
    write_file(&file)?;
    Ok(model.target)
}

pub fn selected() -> Option<Target> {
    list().into_iter().find(|model| model.selected).map(|model| model.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_always_present() {
        assert!(list().iter().any(|model| model.id == "builtin:opus"));
        assert!(list().iter().any(|model| model.id == "builtin:sol"));
    }
}
