//! Resolves the `model` field Claude Code sends (`/model` shortcuts, `provider:model`,
//! `@effort` suffixes) to a provider and an upstream model slug.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    CodexOauth,
    Openai,
    OpenaiCompat,
    GeminiOauth,
    Gemini,
    Anthropic,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "codex-oauth" => Provider::CodexOauth,
            "openai" => Provider::Openai,
            "openai-compat" => Provider::OpenaiCompat,
            "gemini-oauth" => Provider::GeminiOauth,
            "gemini" => Provider::Gemini,
            "anthropic" => Provider::Anthropic,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::CodexOauth => "codex-oauth",
            Provider::Openai => "openai",
            Provider::OpenaiCompat => "openai-compat",
            Provider::GeminiOauth => "gemini-oauth",
            Provider::Gemini => "gemini",
            Provider::Anthropic => "anthropic",
        }
    }

    /// The group the UI turns on and off as one: a login and an API key of the same vendor
    /// count separately only where they are separate products (Codex vs the OpenAI API).
    pub fn family(self) -> &'static str {
        match self {
            Provider::CodexOauth => "codex",
            Provider::Openai => "openai",
            Provider::OpenaiCompat => "openai-compat",
            Provider::GeminiOauth | Provider::Gemini => "gemini",
            Provider::Anthropic => "anthropic",
        }
    }

    /// Stand-in for Claude Code's internal haiku/sonnet calls (titles, Explore subagent, quota probes).
    fn fast_model(self) -> Option<&'static str> {
        match self {
            Provider::CodexOauth => Some("gpt-5.6-luna"),
            Provider::Openai => Some("gpt-5-mini"),
            Provider::OpenaiCompat => None,
            Provider::GeminiOauth | Provider::Gemini => Some("gemini-3-flash-preview"),
            Provider::Anthropic => None,
        }
    }

    /// Stand-in for Claude Code's internal opus calls.
    fn main_model(self) -> Option<&'static str> {
        match self {
            Provider::CodexOauth => Some("gpt-5.6-sol"),
            Provider::Openai => Some("gpt-5.5"),
            Provider::OpenaiCompat => None,
            Provider::GeminiOauth | Provider::Gemini => Some("gemini-3.1-pro-preview"),
            Provider::Anthropic => None,
        }
    }

    /// Providers with server-side web search, which replaces Claude Code's WebSearch/WebFetch.
    pub fn has_native_search(self) -> bool {
        !matches!(self, Provider::Anthropic | Provider::OpenaiCompat)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reasoning {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Reasoning {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "none" => Reasoning::None,
            "minimal" => Reasoning::Minimal,
            "low" => Reasoning::Low,
            "medium" => Reasoning::Medium,
            "high" => Reasoning::High,
            "xhigh" => Reasoning::Xhigh,
            "max" => Reasoning::Max,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Reasoning::None => "none",
            Reasoning::Minimal => "minimal",
            Reasoning::Low => "low",
            Reasoning::Medium => "medium",
            Reasoning::High => "high",
            Reasoning::Xhigh => "xhigh",
            Reasoning::Max => "max",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub provider: Provider,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

impl Target {
    pub fn new(provider: Provider, model: impl Into<String>, reasoning: Option<Reasoning>) -> Self {
        Self {
            provider,
            model: model.into(),
            reasoning,
        }
    }
}

pub struct Preset {
    /// Shortcut accepted by `/model`, the CLI and the UI.
    pub alias: &'static str,
    pub provider: Provider,
    pub description: &'static str,
}

/// The main shortcuts, in the order the UI and `claude-bridge model --list` show them.
pub const PRESETS: &[Preset] = &[
    Preset {
        alias: "opus",
        provider: Provider::Anthropic,
        description: "Claude Opus 5.5",
    },
    Preset {
        alias: "sonnet",
        provider: Provider::Anthropic,
        description: "Claude Sonnet 5.5",
    },
    Preset {
        alias: "haiku",
        provider: Provider::Anthropic,
        description: "Claude Haiku 5.5",
    },
    Preset {
        alias: "sol",
        provider: Provider::CodexOauth,
        description: "GPT-5.6 Sol, frontier (High effort)",
    },
    Preset {
        alias: "terra",
        provider: Provider::CodexOauth,
        description: "GPT-5.6 Terra, balanced",
    },
    Preset {
        alias: "luna",
        provider: Provider::CodexOauth,
        description: "GPT-5.6 Luna, fast",
    },
    Preset {
        alias: "gpt55",
        provider: Provider::CodexOauth,
        description: "GPT-5.5, previous frontier",
    },
    Preset {
        alias: "gemini",
        provider: Provider::GeminiOauth,
        description: "Gemini 3.1 Pro",
    },
    Preset {
        alias: "gemini-flash",
        provider: Provider::GeminiOauth,
        description: "Gemini 3 Flash",
    },
    Preset {
        alias: "gemini-3p",
        provider: Provider::GeminiOauth,
        description: "Gemini 3 Pro",
    },
    Preset {
        alias: "gemini-25p",
        provider: Provider::GeminiOauth,
        description: "Gemini 2.5 Pro",
    },
    Preset {
        alias: "gemini-25f",
        provider: Provider::GeminiOauth,
        description: "Gemini 2.5 Flash",
    },
];

/// Efforts offered as `@suffix`, weakest first.
pub const EFFORTS: &[Reasoning] = &[
    Reasoning::Minimal,
    Reasoning::Low,
    Reasoning::Medium,
    Reasoning::High,
    Reasoning::Xhigh,
    Reasoning::Max,
];

/// `/model` shortcuts. Every Codex model also accepts an `@low|medium|high|xhigh|max` suffix.
fn shortcut(name: &str) -> Option<&'static str> {
    Some(match name {
        // Anthropic passthrough. In /model Claude Code turns opus/sonnet/haiku into bare
        // claude-* names, which count as internal calls; the prefixed forms reach the proxy as-is.
        "opus" | "anthropic:opus" => "anthropic:claude-opus-5-5",
        "sonnet" | "anthropic:sonnet" => "anthropic:claude-sonnet-5-5",
        "haiku" | "anthropic:haiku" => "anthropic:claude-haiku-5-5",
        // Gemini via Google login
        "gemini" | "gemini-pro" | "gemini-31p" | "gp" => "gemini-oauth:gemini-3.1-pro-preview",
        "gemini-flash" | "gf" => "gemini-oauth:gemini-3-flash-preview",
        "gemini-3p" => "gemini-oauth:gemini-3-pro-preview",
        "gemini-31f" => "gemini-oauth:gemini-3.1-flash-preview",
        "gemini-25p" => "gemini-oauth:gemini-2.5-pro",
        "gemini-25f" => "gemini-oauth:gemini-2.5-flash",
        // Codex: GPT-5.6 Sol (frontier) / Terra (balanced) / Luna (fast), GPT-5.5
        "codex" | "cx" | "sol" | "gpt56" | "gpt-5.6-sol" => "codex-oauth:gpt-5.6-sol@high",
        "terra" | "gpt-5.6-terra" => "codex-oauth:gpt-5.6-terra",
        "luna" | "gpt-5.6-luna" => "codex-oauth:gpt-5.6-luna",
        "gpt55" | "gpt-5.5" => "codex-oauth:gpt-5.5",
        // Effort ladder on Sol
        "fast" => "codex-oauth:gpt-5.6-sol@low",
        "smart" => "codex-oauth:gpt-5.6-sol@medium",
        "deep" => "codex-oauth:gpt-5.6-sol@high",
        "xhigh" | "extra" => "codex-oauth:gpt-5.6-sol@xhigh",
        "max" | "think" => "codex-oauth:gpt-5.6-sol@max",
        _ => return None,
    })
}

/// Splits a trailing `@effort`; an `@` that is not a known effort stays part of the name.
fn split_reasoning(s: &str) -> (&str, Option<Reasoning>) {
    match s.rfind('@') {
        Some(at) if at > 0 => match Reasoning::parse(&s[at + 1..]) {
            Some(r) => (&s[..at], Some(r)),
            None => (s, None),
        },
        _ => (s, None),
    }
}

/// `defaults` is the provider currently in use. It decides where bare model names and
/// Claude Code's internal `claude-*` calls go.
pub fn resolve(model_field: &str, defaults: Option<&Target>) -> Result<Target, String> {
    let model_field = model_field.trim();
    if model_field.is_empty() {
        return defaults
            .cloned()
            .ok_or_else(|| "missing 'model' in request".into());
    }

    let (raw, typed_reasoning) = split_reasoning(model_field);
    let expanded = shortcut(&raw.to_ascii_lowercase()).unwrap_or(raw);
    // A shortcut may bake in an effort; one typed explicitly wins.
    let (expanded, baked_reasoning) = split_reasoning(expanded);
    let reasoning = typed_reasoning.or(baked_reasoning);

    if expanded.to_ascii_lowercase().starts_with("claude-") {
        return Ok(match defaults {
            Some(d) if d.provider != Provider::Anthropic => {
                let fallback = if expanded.to_ascii_lowercase().contains("opus") {
                    d.provider.main_model()
                } else {
                    d.provider.fast_model()
                };
                Target::new(d.provider, fallback.unwrap_or(&d.model), reasoning)
            }
            _ => Target::new(Provider::Anthropic, expanded, reasoning),
        });
    }

    let split = expanded
        .split_once(':')
        .or_else(|| expanded.split_once('/'));
    if let Some((prefix, model)) = split
        && let Some(provider) = Provider::parse(&prefix.to_ascii_lowercase())
    {
        return Ok(Target::new(provider, model, reasoning));
    }

    // A bare or unknown-prefixed name goes to the current provider as-is.
    match defaults {
        Some(d) => Ok(Target::new(d.provider, expanded, reasoning.or(d.reasoning))),
        None => Err(format!(
            "cannot tell which provider serves model \"{model_field}\"; use provider:model \
             (codex-oauth, openai, openai-compat, gemini-oauth, gemini, anthropic)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(field: &str) -> Target {
        resolve(field, None).unwrap()
    }

    fn codex(model: &str, reasoning: Option<Reasoning>) -> Target {
        Target::new(Provider::CodexOauth, model, reasoning)
    }

    #[test]
    fn codex_defaults_to_sol_high() {
        assert_eq!(r("codex"), codex("gpt-5.6-sol", Some(Reasoning::High)));
        assert_eq!(r("cx"), codex("gpt-5.6-sol", Some(Reasoning::High)));
    }

    #[test]
    fn family_shortcuts() {
        assert_eq!(r("sol"), codex("gpt-5.6-sol", Some(Reasoning::High)));
        assert_eq!(r("terra"), codex("gpt-5.6-terra", None));
        assert_eq!(r("luna"), codex("gpt-5.6-luna", None));
        assert_eq!(r("gpt55"), codex("gpt-5.5", None));
        assert_eq!(
            r("GPT-5.6-Sol"),
            codex("gpt-5.6-sol", Some(Reasoning::High))
        );
    }

    #[test]
    fn effort_shortcuts() {
        assert_eq!(r("fast"), codex("gpt-5.6-sol", Some(Reasoning::Low)));
        assert_eq!(r("smart"), codex("gpt-5.6-sol", Some(Reasoning::Medium)));
        assert_eq!(r("deep"), codex("gpt-5.6-sol", Some(Reasoning::High)));
        assert_eq!(r("xhigh"), codex("gpt-5.6-sol", Some(Reasoning::Xhigh)));
        assert_eq!(r("extra"), codex("gpt-5.6-sol", Some(Reasoning::Xhigh)));
        assert_eq!(r("max"), codex("gpt-5.6-sol", Some(Reasoning::Max)));
        assert_eq!(r("think"), codex("gpt-5.6-sol", Some(Reasoning::Max)));
    }

    #[test]
    fn explicit_effort_overrides_baked_one() {
        assert_eq!(r("fast@high"), codex("gpt-5.6-sol", Some(Reasoning::High)));
        assert_eq!(r("sol@low"), codex("gpt-5.6-sol", Some(Reasoning::Low)));
    }

    #[test]
    fn verbatim_model_at_level() {
        assert_eq!(r("terra@max"), codex("gpt-5.6-terra", Some(Reasoning::Max)));
        assert_eq!(r("luna@high"), codex("gpt-5.6-luna", Some(Reasoning::High)));
        assert_eq!(
            r("codex@xhigh"),
            codex("gpt-5.6-sol", Some(Reasoning::Xhigh))
        );
        assert_eq!(
            r("codex-oauth:gpt-5.5@medium"),
            codex("gpt-5.5", Some(Reasoning::Medium))
        );
    }

    #[test]
    fn unknown_suffix_stays_in_name() {
        assert_eq!(
            r("openai:ft:model@team"),
            Target::new(Provider::Openai, "ft:model@team", None)
        );
    }

    #[test]
    fn provider_prefixes() {
        assert_eq!(
            r("openai:gpt-5.5"),
            Target::new(Provider::Openai, "gpt-5.5", None)
        );
        assert_eq!(
            r("openai-compat:llama3.3"),
            Target::new(Provider::OpenaiCompat, "llama3.3", None)
        );
        assert_eq!(
            r("gemini/gemini-2.5-pro"),
            Target::new(Provider::Gemini, "gemini-2.5-pro", None)
        );
        assert_eq!(
            r("gemini-flash@low"),
            Target::new(
                Provider::GeminiOauth,
                "gemini-3-flash-preview",
                Some(Reasoning::Low)
            )
        );
    }

    #[test]
    fn explicit_claude_shortcuts() {
        assert_eq!(
            r("anthropic:opus"),
            Target::new(Provider::Anthropic, "claude-opus-5-5", None)
        );
        let active = codex("gpt-5.6-sol", None);
        assert_eq!(
            resolve("anthropic:haiku", Some(&active)).unwrap().provider,
            Provider::Anthropic
        );
    }

    #[test]
    fn claude_models_follow_active_provider() {
        let active = codex("gpt-5.6-sol", None);
        assert_eq!(
            resolve("claude-haiku-5-5", Some(&active)).unwrap(),
            codex("gpt-5.6-luna", None)
        );
        assert_eq!(
            resolve("claude-opus-5-5", Some(&active)).unwrap(),
            codex("gpt-5.6-sol", None)
        );
        assert_eq!(
            r("claude-sonnet-5-5"),
            Target::new(Provider::Anthropic, "claude-sonnet-5-5", None)
        );
        let anthropic = Target::new(Provider::Anthropic, "claude-sonnet-5-5", None);
        assert_eq!(
            resolve("claude-haiku-5-5", Some(&anthropic))
                .unwrap()
                .provider,
            Provider::Anthropic
        );
    }

    #[test]
    fn presets_resolve_to_their_provider() {
        for p in PRESETS {
            assert_eq!(r(p.alias).provider, p.provider, "{}", p.alias);
        }
    }

    #[test]
    fn bare_names_use_active_provider() {
        let active = codex("gpt-5.6-sol", Some(Reasoning::Medium));
        assert_eq!(
            resolve("gpt-5.4", Some(&active)).unwrap(),
            codex("gpt-5.4", Some(Reasoning::Medium))
        );
        assert!(resolve("gpt-5.4", None).is_err());
    }
}
