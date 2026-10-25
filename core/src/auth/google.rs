//! Google OAuth for Gemini, using the Gemini CLI's installed-app client. After login the
//! account is enrolled in Code Assist when possible; otherwise the standard
//! Generative Language API is used with the same token.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::{REFRESH_SKEW_MS, now_ms, read_json, write_secret_json};
use crate::config;
use crate::error::ProxyError;

// Public "installed application" credentials of the Gemini CLI.
const CLIENT_ID: &str = "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform \
                      https://www.googleapis.com/auth/userinfo.email \
                      https://www.googleapis.com/auth/userinfo.profile";
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
pub const CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const CALLBACK_PATH: &str = "/oauth2callback";
/// How often to retry Code Assist setup for an account that has no project.
const CODE_ASSIST_RETRY: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix milliseconds.
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Code Assist project; absent means the standard Generative Language API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

fn token_file(account: u8) -> PathBuf {
    let name = if account == 1 { "google-oauth.json".to_string() } else { format!("google-oauth-{account}.json") };
    config::data_dir().join(name)
}

pub fn load(account: u8) -> Option<GoogleTokens> {
    read_json::<GoogleTokens>(&token_file(account)).filter(|t| !t.refresh_token.is_empty())
}

pub fn logout(account: u8) -> bool {
    super::remove_file(&token_file(account))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Default)]
pub struct GoogleAuth {
    /// Last Code Assist setup attempt per account.
    lock: Mutex<HashMap<u8, Instant>>,
}

impl GoogleAuth {
    /// Tokens for `account` with a fresh access token.
    pub async fn credentials(&self, http: &reqwest::Client, account: u8) -> Result<GoogleTokens, ProxyError> {
        let mut attempts = self.lock.lock().await;
        let mut tokens = load(account).ok_or_else(|| {
            let flag = if account == 1 { String::new() } else { format!(" --account {account}") };
            ProxyError::unauthorized(format!("Not logged in to Google. Run `claude-bridge login google{flag}`."))
        })?;
        if now_ms() > tokens.expires_at - REFRESH_SKEW_MS {
            self.refresh(http, account, &mut tokens).await?;
        }

        // Setup at login can fail transiently, and the standard API rejects these tokens.
        let due = attempts.get(&account).is_none_or(|t| t.elapsed() >= CODE_ASSIST_RETRY);
        if tokens.project_id.is_none() && due {
            attempts.insert(account, Instant::now());
            info!(account, "no Code Assist project, retrying setup");
            if let Some(project) = setup_code_assist(http, &tokens.access_token).await {
                info!(account, project, "Code Assist project found");
                tokens.project_id = Some(project);
                write_secret_json(&token_file(account), &tokens)
                    .map_err(|e| ProxyError::internal(format!("saving Code Assist project: {e:#}")))?;
            }
        }
        Ok(tokens)
    }

    async fn refresh(&self, http: &reqwest::Client, account: u8, tokens: &mut GoogleTokens) -> Result<(), ProxyError> {

        info!(account, "google access token expired, refreshing");
        let resp = http
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
                ("refresh_token", tokens.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ProxyError::unauthorized(format!(
                "Google token refresh failed ({status}): {}. Run `claude-bridge login google`.",
                crate::error::truncate(&body, 300)
            )));
        }
        let r: TokenResponse = resp.json().await?;
        tokens.access_token = r.access_token;
        tokens.expires_at = now_ms() + r.expires_in.unwrap_or(3600) * 1000;
        if let Some(refresh) = r.refresh_token {
            tokens.refresh_token = refresh;
        }
        write_secret_json(&token_file(account), &*tokens)
            .map_err(|e| ProxyError::internal(format!("saving refreshed token: {e:#}")))?;
        Ok(())
    }
}

/// `on_url` gets the authorization URL and is expected to open it.
pub async fn login(http: &reqwest::Client, account: u8, on_url: impl FnOnce(&str)) -> anyhow::Result<GoogleTokens> {
    // Installed-app clients accept any loopback port.
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let pkce = super::Pkce::new();
    let state = super::random_urlsafe(32);

    let mut url = url::Url::parse(AUTH_ENDPOINT)?;
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &pkce.challenge);
    on_url(url.as_str());

    let code = super::wait_for_code(vec![listener], CALLBACK_PATH, state).await?;

    let resp = http
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", code.as_str()),
            ("code_verifier", pkce.verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
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

    let email = fetch_email(http, &r.access_token).await;
    let project_id = setup_code_assist(http, &r.access_token).await;
    let tokens = GoogleTokens {
        expires_at: now_ms() + r.expires_in.unwrap_or(3600) * 1000,
        access_token: r.access_token,
        refresh_token,
        email,
        project_id,
    };
    write_secret_json(&token_file(account), &tokens)?;
    Ok(tokens)
}

async fn fetch_email(http: &reqwest::Client, access_token: &str) -> Option<String> {
    let resp = http.get(USERINFO_ENDPOINT).bearer_auth(access_token).send().await.ok()?;
    let body: Value = resp.error_for_status().ok()?.json().await.ok()?;
    body["email"].as_str().map(String::from)
}

fn client_metadata() -> Value {
    json!({ "ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI" })
}

async fn post_code_assist(http: &reqwest::Client, token: &str, method: &str, body: Value) -> anyhow::Result<Value> {
    let resp = http.post(format!("{CODE_ASSIST_ENDPOINT}:{method}")).bearer_auth(token).json(&body).send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("{method} returned {status}");
    }
    Ok(resp.json().await?)
}

/// Finds or provisions the Code Assist project. Failure is not fatal: the standard API is used.
async fn setup_code_assist(http: &reqwest::Client, token: &str) -> Option<String> {
    match try_setup_code_assist(http, token).await {
        Ok(project) => project,
        Err(e) => {
            warn!("Code Assist setup failed, using the standard Gemini API: {e:#}");
            None
        }
    }
}

async fn try_setup_code_assist(http: &reqwest::Client, token: &str) -> anyhow::Result<Option<String>> {
    let loaded = post_code_assist(http, token, "loadCodeAssist", json!({ "metadata": client_metadata() })).await?;
    if let Some(project) = loaded["cloudaicompanionProject"].as_str() {
        return Ok(Some(project.into()));
    }

    let tiers = loaded["allowedTiers"].as_array().or_else(|| loaded["availableTiers"].as_array());
    let tier = [&loaded["paidTier"], &loaded["currentTier"]]
        .into_iter()
        .find(|t| t.is_object())
        .or_else(|| tiers.and_then(|ts| ts.iter().find(|t| t["id"] == "STANDARD" || t["id"] == "FREE")))
        .or_else(|| tiers.and_then(|ts| ts.first()));
    let Some(tier_id) = tier.and_then(|t| t["id"].as_str()) else {
        return Ok(None);
    };

    info!(tier = tier_id, "onboarding to Code Assist");
    let mut op = post_code_assist(http, token, "onboardUser", json!({ "tierId": tier_id, "metadata": client_metadata() })).await?;
    for _ in 0..12 {
        if op["done"].as_bool() == Some(true) {
            break;
        }
        let Some(name) = op["name"].as_str() else { break };
        tokio::time::sleep(Duration::from_secs(5)).await;
        op = http
            .get(format!("{CODE_ASSIST_ENDPOINT}/operations/{name}"))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
    }
    Ok(op["response"]["cloudaicompanionProject"]["id"].as_str().map(String::from))
}
