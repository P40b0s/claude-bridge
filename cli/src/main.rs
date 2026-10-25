use std::io::IsTerminal;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use tracing_subscriber::fmt::writer::BoxMakeWriter;

use claude_bridge_core::auth::{self, google, now_ms, openai};
use claude_bridge_core::launcher::{self, ContextWindow, Launch};
use claude_bridge_core::{claude_settings, config, control, models, routing, server, usage};

#[derive(Parser)]
#[command(name = "claude-bridge", version, about = "Run Claude Code on GPT (Codex), Gemini and other models")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Claude Code on GPT-5.6 via your ChatGPT login
    Codex(LaunchArgs),
    /// Claude Code on Gemini via Google login (or GEMINI_API_KEY)
    Gemini(LaunchArgs),
    /// Claude Code on the OpenAI API (OPENAI_API_KEY)
    Openai(LaunchArgs),
    /// Claude Code through the proxy to Claude (Claude Code's own login or ANTHROPIC_API_KEY)
    Anthropic(LaunchArgs),
    /// Run the proxy in the foreground
    Serve {
        /// provider:model used until the first request picks one, e.g. codex-oauth:gpt-5.6-sol
        #[arg(long)]
        default: Option<String>,
    },
    /// Log in through the browser
    Login {
        provider: AuthProvider,
        /// Google account slot; account 2 takes over when account 1 is rate limited
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
        account: u8,
    },
    /// Remove stored credentials
    Logout {
        provider: AuthProvider,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
        account: u8,
    },
    /// Switch the model of every running session, e.g. `terra@max` or `gemini-flash`
    Model {
        /// /model spec; omit to show the current one
        model: Option<String>,
        /// Follow Claude Code's /model choice again
        #[arg(long, conflicts_with = "model")]
        clear: bool,
        /// List the shortcuts
        #[arg(long, conflicts_with_all = ["model", "clear"])]
        list: bool,
    },
    /// Show proxy and login status
    Status,
    /// Show subscription limits: Claude, ChatGPT (Codex), Gemini
    Usage,
    /// Send every `claude` through the proxy (adds it to ~/.claude/settings.json)
    Connect,
    /// Take the proxy back out of ~/.claude/settings.json
    Disconnect,
    /// Stop the background proxy
    Stop,
}

#[derive(Args)]
struct LaunchArgs {
    /// Pass --dangerously-skip-permissions to Claude Code
    #[arg(short = 'd', long = "dangerously-skip-permissions")]
    skip_permissions: bool,
    /// Restart the proxy (needed after upgrading claude-bridge)
    #[arg(long)]
    restart: bool,
    /// Extra arguments for Claude Code
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    claude_args: Vec<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum AuthProvider {
    Codex,
    Google,
}

#[tokio::main]
async fn main() {
    config::load_env_file();
    let cli = Cli::parse();
    // The background proxy writes its own log file so it can rotate it; stderr (still pointed at
    // the same file) only catches panics.
    let log_file = std::env::var_os(control::LOG_FILE_ENV).and_then(|path| control::RotatingLog::open(path.into()).ok());
    let (writer, ansi) = match log_file {
        Some(log) => (BoxMakeWriter::new(move || log.clone()), false),
        None => (BoxMakeWriter::new(std::io::stderr), std::io::stderr().is_terminal()),
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("CLAUDE_BRIDGE_LOG").unwrap_or_else(|_| "claude_bridge=info".into()),
        )
        .with_ansi(ansi)
        .with_writer(writer)
        .init();

    if let Err(e) = run(cli.command).await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Serve { default } => {
            let default = match default {
                Some(spec) => Some(routing::resolve(&spec, None).map_err(anyhow::Error::msg)?),
                None => None,
            };
            server::serve(config::port(), default).await
        }
        Command::Codex(args) => launch_codex(args).await,
        Command::Gemini(args) => launch_gemini(args).await,
        Command::Openai(args) => {
            require_env("OPENAI_API_KEY")?;
            launcher::launch(args.into_launch("openai:gpt-5.5", Some("openai:gpt-5.5"), None)).await
        }
        Command::Anthropic(args) => {
            if auth::claude_code::account().is_none() {
                require_env("ANTHROPIC_API_KEY").context("or log in to Claude Code first (`claude`, then /login)")?;
            }
            // Prefixed so the proxy sees an explicit choice, not one of Claude Code's internal calls.
            launcher::launch(args.into_launch("anthropic:claude-opus-5-5", Some("anthropic:opus"), None)).await
        }
        Command::Login { provider, account } => login(provider, account).await,
        Command::Logout { provider, account } => {
            let removed = match provider {
                AuthProvider::Codex => openai::logout(),
                AuthProvider::Google => google::logout(account),
            };
            eprintln!("{}", if removed { "Logged out." } else { "Not logged in." });
            Ok(())
        }
        Command::Status => status().await,
        Command::Usage => usage_command().await,
        Command::Connect => {
            claude_settings::connect(config::port())?;
            println!("Every `claude` now goes through {} ({}).", control::base_url(config::port()), claude_settings::settings_file().display());
            println!("It needs the proxy running: claude-bridge serve, or start it in the UI.");
            Ok(())
        }
        Command::Disconnect => {
            claude_settings::disconnect()?;
            println!("`claude` talks to Anthropic directly again ({}).", claude_settings::settings_file().display());
            Ok(())
        }
        Command::Model { model, clear, list } => model_command(model, clear, list).await,
        Command::Stop => {
            let stopped = control::stop(config::port()).await?;
            eprintln!("{}", if stopped { "Proxy stopped." } else { "No proxy running." });
            Ok(())
        }
    }
}

impl LaunchArgs {
    fn into_launch(self, proxy_default: &str, claude_model: Option<&str>, context: Option<ContextWindow>) -> Launch {
        Launch {
            proxy_default: proxy_default.into(),
            claude_model: claude_model.map(String::from),
            context,
            skip_permissions: self.skip_permissions,
            restart: self.restart,
            claude_args: self.claude_args,
        }
    }
}

fn require_env(name: &str) -> anyhow::Result<()> {
    if config::env(name).is_none() {
        bail!("{name} is not set. Export it or add it to {}", config::data_dir().join(".env").display());
    }
    Ok(())
}

fn env_u64(name: &str) -> Option<u64> {
    config::env(name).and_then(|v| v.parse().ok()).filter(|&v| v > 0)
}

fn expires_in(expires_at: i64) -> String {
    let minutes = (expires_at - now_ms()) / 60_000;
    if minutes > 0 { format!("expires in {minutes} min") } else { "expired (refreshes automatically)".into() }
}

async fn launch_codex(args: LaunchArgs) -> anyhow::Result<()> {
    let tokens = match openai::load() {
        Some((tokens, _)) => tokens,
        None => openai::login(&server::http_client()?, auth::print_login_url).await?,
    };
    eprintln!(
        "  ChatGPT: {} ({})",
        tokens.email.as_deref().unwrap_or("unknown"),
        tokens.plan.as_deref().unwrap_or("unknown plan")
    );
    eprintln!("  Models: /model sol | terra | luna | gpt55, effort: fast | smart | deep | xhigh | max or <model>@<effort>");

    // GPT-5.6 family window; compact a little before the hard limit.
    let tokens = env_u64("CODEX_CONTEXT_WINDOW_TOKENS").unwrap_or(372_000);
    let compact_at = env_u64("CODEX_AUTO_COMPACT_WINDOW_TOKENS").unwrap_or(tokens * 95 / 100).min(tokens);
    let context = ContextWindow {
        tokens,
        auto_compact_at: compact_at,
        disable_compact: config::env("CODEX_DISABLE_COMPACT").as_deref() == Some("1"),
    };
    launcher::launch(args.into_launch("codex-oauth:gpt-5.6-sol", Some("codex"), Some(context))).await
}

async fn launch_gemini(args: LaunchArgs) -> anyhow::Result<()> {
    let (default, model) = if config::env("GEMINI_API_KEY").is_some() && google::load(1).is_none() {
        eprintln!("  Gemini: API key");
        ("gemini:gemini-3.1-pro-preview", "gemini:gemini-3.1-pro-preview")
    } else {
        let tokens = match google::load(1) {
            Some(tokens) => tokens,
            None => google::login(&server::http_client()?, 1, auth::print_login_url).await?,
        };
        eprintln!(
            "  Google: {} ({})",
            tokens.email.as_deref().unwrap_or("unknown"),
            tokens.project_id.as_deref().map(|p| format!("Code Assist: {p}")).unwrap_or_else(|| "standard API".into())
        );
        ("gemini-oauth:gemini-3.1-pro-preview", "gemini")
    };
    eprintln!("  Models: /model gemini | gemini-flash | gemini-3p | gemini-25p | gemini-25f, thinking: <model>@low|medium|high");
    let context = ContextWindow { tokens: 1_000_000, auto_compact_at: 1_000_000, disable_compact: true };
    launcher::launch(args.into_launch(default, Some(model), Some(context))).await
}

async fn login(provider: AuthProvider, account: u8) -> anyhow::Result<()> {
    let http = server::http_client()?;
    match provider {
        AuthProvider::Codex => {
            let t = openai::login(&http, auth::print_login_url).await?;
            eprintln!("Logged in to ChatGPT as {} ({}).", t.email.as_deref().unwrap_or("unknown"), t.plan.as_deref().unwrap_or("unknown plan"));
        }
        AuthProvider::Google => {
            let t = google::login(&http, account, auth::print_login_url).await?;
            let mode = t.project_id.as_deref().map(|p| format!("Code Assist project {p}")).unwrap_or_else(|| "standard Gemini API".into());
            eprintln!("Logged in to Google (account {account}) as {}, using {mode}.", t.email.as_deref().unwrap_or("unknown"));
        }
    }
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let port = config::port();
    match control::health(port).await {
        Some(h) => {
            println!("Proxy:    running on 127.0.0.1:{port}, PID {}, model {}", h.pid, describe(h.active.as_ref()));
            if let Some(pinned) = &h.pinned {
                println!("Pinned:   {} (claude-bridge model --clear to follow /model)", describe(Some(pinned)));
            }
        }
        None => println!("Proxy:    not running"),
    }
    match auth::claude_code::account() {
        Some(a) => {
            let plan = a.plan.as_deref().unwrap_or("no subscription");
            let since = a.subscription_since.map(|ms| format!(" since {}", format_date(ms))).unwrap_or_default();
            let billing = a.billing.map(|b| format!(", billed via {b}")).unwrap_or_default();
            println!("Claude:   {} ({plan}{since}{billing}), Claude Code's own login", a.email.as_deref().unwrap_or("unknown"));
        }
        None => println!("Claude:   Claude Code is not logged in"),
    }
    match openai::load() {
        Some((t, source)) => println!(
            "ChatGPT:  {} ({}{}), via {}, token {}",
            t.email.as_deref().unwrap_or("unknown"),
            t.plan.as_deref().unwrap_or("unknown plan"),
            t.subscription_until().map(|ms| format!(" until {}", format_date(ms))).unwrap_or_default(),
            source.as_str(),
            expires_in(t.expires_at)
        ),
        None => println!("ChatGPT:  not logged in (claude-bridge login codex)"),
    }
    for account in 1..=2u8 {
        match google::load(account) {
            Some(t) => println!(
                "Google {account}: {}, {}, token {}",
                t.email.as_deref().unwrap_or("unknown"),
                t.project_id.as_deref().map(|p| format!("Code Assist {p}")).unwrap_or_else(|| "standard API".into()),
                expires_in(t.expires_at)
            ),
            None if account == 1 => println!("Google 1: not logged in (claude-bridge login google)"),
            None => println!("Google 2: not linked (optional failover: claude-bridge login google --account 2)"),
        }
    }
    for key in ["OPENAI_API_KEY", "GEMINI_API_KEY", "ANTHROPIC_API_KEY"] {
        println!("{key:<18}{}", if config::env(key).is_some() { "set" } else { "not set" });
    }
    let claude = match claude_settings::connection(port) {
        claude_settings::Connection::Connected => "goes through the proxy (claude-bridge disconnect to undo)".to_string(),
        claude_settings::Connection::Other(url) => format!("goes to {url}"),
        claude_settings::Connection::Disconnected => "direct; only `claude-bridge <route>` uses the proxy (claude-bridge connect)".into(),
    };
    println!("claude:   {claude}");
    println!("Config:   {}", config::data_dir().display());
    Ok(())
}

async fn usage_command() -> anyhow::Result<()> {
    let report = usage::collect(&server::http_client()?, &google::GoogleAuth::default()).await;
    match &report.claude {
        Some(u) => print_usage("Claude", u, Some("from the last reply relayed by the proxy")),
        None if auth::claude_code::account().is_some() => {
            println!("Claude:   no data yet; it appears after a Claude request through the proxy")
        }
        None => println!("Claude:   Claude Code is not logged in"),
    }
    match &report.codex {
        Some(u) => print_usage("ChatGPT", u, None),
        None => println!("ChatGPT:  not logged in"),
    }
    for u in &report.gemini {
        print_usage(&format!("Google {}", u.account.unwrap_or(1)), u, None);
    }
    Ok(())
}

fn print_usage(name: &str, u: &usage::Usage, source: Option<&str>) {
    let label = format!("{name}:");
    if let Some(error) = &u.error {
        println!("{label:<10}{error}");
        return;
    }
    let age = (now_ms() - u.observed_at) / 60_000;
    let note = [source.map(String::from), (age > 0).then(|| format!("{age} min ago"))].into_iter().flatten().collect::<Vec<_>>();
    let limited = if u.limited { "  LIMIT REACHED" } else { "" };
    println!("{label:<10}{}{limited}", if note.is_empty() { String::new() } else { format!("({})", note.join(", ")) });
    for w in &u.windows {
        let span = w.window_minutes.map(format_minutes).unwrap_or_else(|| w.id.clone());
        let span = match (&w.model, w.window_minutes) {
            (Some(model), Some(_)) => format!("{span} {model}"),
            _ => span,
        };
        let reset = w.resets_at.map(|at| format!(", resets in {}", format_minutes((at - now_ms()).max(0) / 60_000))).unwrap_or_default();
        println!("  {span:<24}{:>5.1}% used{reset}", w.used_percent);
    }
}

/// Unix ms → `2026-11-25` (UTC).
fn format_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02}")
}

fn format_minutes(minutes: i64) -> String {
    match minutes {
        m if m >= 24 * 60 && m % (24 * 60) == 0 => format!("{}d", m / (24 * 60)),
        m if m >= 24 * 60 => format!("{}d {}h", m / (24 * 60), m % (24 * 60) / 60),
        m if m >= 60 && m % 60 == 0 => format!("{}h", m / 60),
        m if m >= 60 => format!("{}h {}m", m / 60, m % 60),
        m => format!("{m}m"),
    }
}

fn describe(target: Option<&routing::Target>) -> String {
    match target {
        Some(t) => {
            let effort = t.reasoning.map(|r| format!("@{}", r.as_str())).unwrap_or_default();
            format!("{}:{}{effort}", t.provider.as_str(), t.model)
        }
        None => "-".into(),
    }
}

async fn model_command(model: Option<String>, clear: bool, list: bool) -> anyhow::Result<()> {
    if list {
        for model in models::list() {
            let marker = if model.selected { "*" } else { " " };
            let effort = model.target.reasoning.map(|r| format!("@{}", r.as_str())).unwrap_or_default();
            let spec = format!("{}:{}{effort}", model.target.provider.as_str(), model.target.model);
            println!("{marker} {:<28} {}", spec, model.name);
        }
        let efforts: Vec<&str> = routing::EFFORTS.iter().map(|r| r.as_str()).collect();
        println!("\n  Add @effort to any model: {}. Any provider:model works too.", efforts.join(" | "));
        return Ok(());
    }
    let port = config::port();
    if clear {
        control::unpin_model(port).await?;
        eprintln!("Unpinned: sessions follow Claude Code's /model again.");
    } else if let Some(model) = model {
        let target = control::pin_model(port, &model).await?;
        eprintln!("All sessions now use {}.", describe(Some(&target)));
    } else {
        let h = control::health(port).await.ok_or_else(|| anyhow::anyhow!("proxy is not running"))?;
        match h.pinned {
            Some(p) => println!("pinned {}", describe(Some(&p))),
            None => println!("following /model, last used {}", describe(h.active.as_ref())),
        }
    }
    Ok(())
}
