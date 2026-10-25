//! ChatGPT/Codex OAuth. Uses the Codex CLI client, so tokens from `~/.codex/auth.json`
//! work as-is and our own login mirrors what `codex login` does.

use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

use super::{REFRESH_SKEW_MS, jwt_claims, now_ms, read_json, write_secret_json};
use crate::config;
use crate::error::ProxyError;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str = "openid profile email offline_access";
/// The Codex client only accepts this redirect, so login must listen on this port.
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Unix milliseconds.
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Bridge,
    CodexCli,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Bridge => "claude-bridge",
            Source::CodexCli => "codex-cli",
        }
    }
}

fn bridge_file() -> PathBuf {
    config::data_dir().join("codex-oauth.json")
}

fn codex_cli_file() -> PathBuf {
    config::home_dir().join(".codex").join("auth.json")
}

impl CodexTokens {
    /// When the paid ChatGPT plan ends (Unix ms), from the ID token; None on the free plan.
    pub fn subscription_until(&self) -> Option<i64> {
        let claims = jwt_claims(self.id_token.as_deref()?)?;
        let until = claims["https://api.openai.com/auth"]["chatgpt_subscription_active_until"].as_str()?;
        crate::usage::parse_rfc3339_ms(until)
    }

    fn from_token_response(access_token: String, refresh_token: String, id_token: Option<String>, expires_in: Option<i64>) -> Self {
        let access = jwt_claims(&access_token);
        let id = id_token.as_deref().and_then(jwt_claims);
        let auth = access.as_ref().map(|c| &c["https://api.openai.com/auth"]);
        let expires_at = access
            .as_ref()
            .and_then(|c| c["exp"].as_i64())
            .map(|exp| exp * 1000)
            .unwrap_or_else(|| now_ms() + expires_in.unwrap_or(3600) * 1000);
        let email = id
            .as_ref()
            .and_then(|c| c["email"].as_str())
            .or_else(|| access.as_ref().and_then(|c| c["https://api.openai.com/profile"]["email"].as_str()))
            .map(String::from);
        Self {
            expires_at,
            email,
            plan: auth.and_then(|a| a["chatgpt_plan_type"].as_str()).map(String::from),
            account_id: auth.and_then(|a| a["chatgpt_account_id"].as_str()).map(String::from),
            access_token,
            refresh_token,
            id_token,
        }
    }
}

fn load_codex_cli() -> Option<CodexTokens> {
    let file: Value = read_json(&codex_cli_file())?;
    let t = &file["tokens"];
    let mut tokens = CodexTokens::from_token_response(
        t["access_token"].as_str()?.to_string(),
        t["refresh_token"].as_str()?.to_string(),
        t["id_token"].as_str().map(String::from),
        None,
    );
    if let Some(account) = t["account_id"].as_str() {
        tokens.account_id = Some(account.into());
    }
    Some(tokens)
}

/// Our own login wins over the Codex CLI's.
pub fn load() -> Option<(CodexTokens, Source)> {
    read_json::<CodexTokens>(&bridge_file())
        .filter(|t| !t.access_token.is_empty() && !t.refresh_token.is_empty())
        .map(|t| (t, Source::Bridge))
        .or_else(|| load_codex_cli().map(|t| (t, Source::CodexCli)))
}

fn save(tokens: &CodexTokens, source: Source) -> anyhow::Result<()> {
    match source {
        Source::Bridge => write_secret_json(&bridge_file(), tokens),
        // Refresh tokens rotate: write back to the Codex CLI file so `codex` keeps working too.
        Source::CodexCli => {
            let path = codex_cli_file();
            let mut file: Value = read_json(&path).unwrap_or_else(|| json!({}));
            let t = &mut file["tokens"];
            t["access_token"] = json!(tokens.access_token);
            t["refresh_token"] = json!(tokens.refresh_token);
            if let Some(id) = &tokens.id_token {
                t["id_token"] = json!(id);
            }
            write_secret_json(&path, &file)
        }
    }
}

pub fn logout() -> bool {
    super::remove_file(&bridge_file())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

pub struct Credentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

/// Serializes refreshes: refresh tokens are single-use, so two concurrent refreshes would
/// leave one request with a revoked token.
#[derive(Default)]
pub struct OpenAiAuth {
    lock: Mutex<()>,
}

impl OpenAiAuth {
    pub async fn credentials(&self, http: &reqwest::Client) -> Result<Credentials, ProxyError> {
        let _guard = self.lock.lock().await;
        let (mut tokens, source) = load().ok_or_else(|| {
            ProxyError::unauthorized("Not logged in to ChatGPT. Run `claude-bridge login codex` (or `codex login`).")
        })?;

        if now_ms() > tokens.expires_at - REFRESH_SKEW_MS {
            info!("codex access token expired, refreshing");
            let resp = http
                .post(TOKEN_ENDPOINT)
                .json(&json!({
                    "client_id": CLIENT_ID,
                    "grant_type": "refresh_token",
                    "refresh_token": tokens.refresh_token,
                    "scope": "openid profile email",
                }))
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(ProxyError::unauthorized(format!(
                    "ChatGPT token refresh failed ({status}): {}. Run `claude-bridge login codex`.",
                    crate::error::truncate(&body, 300)
                )));
            }
            let r: TokenResponse = resp.json().await?;
            let mut refreshed = CodexTokens::from_token_response(
                r.access_token,
                r.refresh_token.unwrap_or(tokens.refresh_token),
                r.id_token.or(tokens.id_token),
                r.expires_in,
            );
            refreshed.email = refreshed.email.or(tokens.email);
            refreshed.plan = refreshed.plan.or(tokens.plan);
            refreshed.account_id = refreshed.account_id.or(tokens.account_id);
            tokens = refreshed;
            save(&tokens, source).map_err(|e| ProxyError::internal(format!("saving refreshed token: {e:#}")))?;
        }

        Ok(Credentials { access_token: tokens.access_token, account_id: tokens.account_id })
    }
}

/// Browser PKCE login with a loopback callback on the port the Codex client allows.
/// `on_url` gets the authorization URL and is expected to open it.
pub async fn login(http: &reqwest::Client, on_url: impl FnOnce(&str)) -> anyhow::Result<CodexTokens> {
    let mut listeners = vec![
        TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
            .await
            .with_context(|| format!("port {CALLBACK_PORT} is busy (is `codex login` running?)"))?,
    ];
    // The redirect goes to `localhost`, which browsers may resolve to ::1 first.
    if let Ok(v6) = TcpListener::bind(("::1", CALLBACK_PORT)).await {
        listeners.push(v6);
    }
    let redirect_uri = format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}");
    let pkce = super::Pkce::new();
    let state = super::random_urlsafe(32);

    let mut url = url::Url::parse(AUTH_ENDPOINT)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", &state)
        .append_pair("originator", "codex_cli_rs");
    on_url(url.as_str());

    let code = super::wait_for_code(listeners, CALLBACK_PATH, state).await?;

    let resp = http
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", pkce.verifier.as_str()),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        bail!("token exchange failed ({status}): {}", resp.text().await.unwrap_or_default());
    }
    let r: TokenResponse = resp.json().await?;
    let Some(refresh_token) = r.refresh_token else {
        bail!("token response has no refresh_token");
    };
    let tokens = CodexTokens::from_token_response(r.access_token, refresh_token, r.id_token, r.expires_in);
    write_secret_json(&bridge_file(), &tokens)?;
    Ok(tokens)
}
