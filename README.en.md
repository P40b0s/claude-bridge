# claude-bridge

[Русский](README.md) | **English**

A local proxy that speaks the Anthropic Messages API, so Claude Code can run on other models:

- **Codex**: GPT-5.6 Sol / Terra / Luna and GPT-5.5 on a ChatGPT subscription (browser login or Codex CLI tokens);
- **Gemini**: through a Google login (Code Assist) or `GEMINI_API_KEY`;
- **OpenAI API**: with `OPENAI_API_KEY`;
- **OpenAI-compatible**: OpenRouter, Ollama, vLLM, LM Studio, DeepSeek and other Chat Completions APIs;
- **Anthropic API**: with `ANTHROPIC_API_KEY`.

Claude Code sends requests to `http://127.0.0.1:17870/v1/messages`. The proxy translates them into the provider's format and returns the answer as Anthropic-style SSE. Streaming and non-streaming responses, tool use, images, thinking blocks and the provider's built-in web search are supported. You can switch models mid-session with `/model`, or for every session at once from the CLI or the UI. Providers you don't need can be turned off, and proxying can be limited to selected countries.


## Installation

Prebuilt packages are in [Releases](../../releases). Each version has:

| File | Contents |
| --- | --- |
| `claude-bridge-<version>-linux-x64.tar.gz` | UI and CLI |
| `claude-bridge-<version>-macos-universal.zip` | `claude_bridge_ui.app`, CLI inside `Contents/MacOS` |
| `claude-bridge-<version>-windows-x64.zip` | UI and `claude-bridge.exe` |
| `claude-bridge-cli-<version>-<platform>.*` | CLI only |
| `SHA256SUMS.txt` | checksums |

[Claude Code](https://docs.claude.com/en/docs/claude-code) must be installed (`claude` in `PATH`).

The builds are not signed:
- **macOS.** On first launch, open the app via right-click → "Open", or remove the quarantine flag: `xattr -dr com.apple.quarantine claude_bridge_ui.app`.

CLI only, from source:

```bash
cargo install --path cli
```

## CLI

```bash
claude-bridge codex          # Claude Code on GPT-5.6 Sol (ChatGPT login on first run)
claude-bridge gemini         # Claude Code on Gemini 3.1 Pro (Google login or GEMINI_API_KEY)
claude-bridge openai         # OpenAI API, needs OPENAI_API_KEY
claude-bridge anthropic      # Claude Opus: Claude Code's login or ANTHROPIC_API_KEY
```

The command starts the proxy in the background (or reuses a running one) and launches `claude` pointed at it. Flags:
- `-d` passes `--dangerously-skip-permissions` to Claude Code (off by default: Claude Code asks for permissions as usual). It only affects the session this command starts in the terminal. IDE extensions and a plain `claude` are started without claude-bridge, so their permission mode is set in the extension itself or with `permissions.defaultMode` in `~/.claude/settings.json`;
- `--restart` restarts the proxy;
- everything after the route name goes to `claude` as is.

The proxy is a separate process shared by all sessions. It keeps running after Claude Code exits.

| Command | What it does |
| --- | --- |
| `claude-bridge status` | state of the proxy, logins and keys |
| `claude-bridge connect` / `disconnect` | add the proxy to `~/.claude/settings.json` so that every `claude` goes through it, or remove it |
| `claude-bridge usage` | [subscription limits](#subscription-limits): Claude, ChatGPT, Gemini |
| `claude-bridge stop` | stop the proxy |
| `claude-bridge login codex` / `login google [--account 2]` | log in through the browser; Google account 2 takes over when the first one hits its limit (429) |
| `claude-bridge logout codex` / `logout google [--account 2]` | delete the saved tokens |
| `claude-bridge model terra@max` | switch every running session to a model |
| `claude-bridge model --clear` | let `/model` choose the model again |
| `claude-bridge model --list` | list the short names |
| `claude-bridge serve` | run the proxy in the foreground |

## Models

In Claude Code the model is picked with `/model <name>`. Short names:

| Name | Model |
| --- | --- |
| `sol` (`codex`, `cx`) | `codex-oauth:gpt-5.6-sol`, high effort |
| `terra`, `luna`, `gpt55` | GPT-5.6 Terra, GPT-5.6 Luna, GPT-5.5 |
| `fast` / `smart` / `deep` / `xhigh` / `max` | Sol with low / medium / high / xhigh / max effort |
| `gemini`, `gemini-flash`, `gemini-3p`, `gemini-25p`, `gemini-25f` | Gemini through a Google login |
| `opus`, `anthropic:sonnet`, `anthropic:haiku` | Claude: through Claude Code's login or `ANTHROPIC_API_KEY` |

The **Models** tab shows built-in and custom models together. A new Claude, GPT, Gemini or OpenAI-compatible model can be added without recompiling: set its name, provider, Model ID and optional effort. Custom models and the model selected on the Home page are stored in `~/.claude-bridge/models.json`. Disabling a provider hides its models from the lists without deleting them.

More options:
- **Effort suffix.** `@low|medium|high|xhigh|max` works with any model, e.g. `/model terra@max` or `/model gemini-flash@low`.
- **Explicit provider.** `provider:model`, where the provider is `codex-oauth`, `openai`, `openai-compat`, `gemini-oauth`, `gemini` or `anthropic`. For a compatible API: `/model openai-compat:llama3.3`.
- **A name without a provider** goes to the current provider as is.

Claude Code makes its own background calls to `claude-haiku-*` and `claude-sonnet-*`: titles, subagents, checks. The proxy sends them to the provider of the session's main model (to its fast model), so they need no Anthropic key. They reach Claude only when the session's main model is Claude (`anthropic:…`).

### Claude and other models in one session

The proxy never logs in to a Claude subscription itself: Anthropic's terms allow subscription tokens only in Anthropic's own apps. Instead, Claude Code uses its own login and the proxy just relays its requests.

- If Claude Code is logged in (`/login`, with a subscription or a key), `claude-bridge` leaves that login alone. Otherwise Claude Code gets the placeholder `ANTHROPIC_AUTH_TOKEN=claude-bridge`: with it, Claude Code does not ask you to log in, all traffic goes to the other providers, and Claude models use `ANTHROPIC_API_KEY`.
- Claude is picked explicitly: `/model opus` (`claude-bridge` maps it to `anthropic:claude-opus-5-5`), `anthropic:sonnet` or `anthropic:haiku`. Those requests go straight to Anthropic with Claude Code's login. The proxy forwards them untouched and stores nothing.
- `/model sol`, `gemini` and the other models go through the proxy as usual.
- Claude Code's background calls (`claude-haiku-*`, `claude-sonnet-*` and so on) go wherever **this** session's main model goes. While a non-Claude model is selected, the session sends nothing to Anthropic. Sessions are told apart by the `X-Claude-Code-Session-Id` header and don't affect each other.
- A pinned model ("Model for all sessions") overrides everything.

To route a plain `claude` (and IDE extensions) through the proxy too, run `claude-bridge connect` or turn on "Any claude through the proxy" on the UI's home page. This is added to `~/.claude/settings.json`:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:17870",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "anthropic:claude-opus-5-5",
    "ANTHROPIC_AUTH_TOKEN": "claude-bridge"
  }
}
```

`ANTHROPIC_AUTH_TOKEN` is added only when Claude Code has no login of its own: without it, Claude Code and the IDE extension ask you to log in even when they run on GPT. A token in `env` takes precedence over the subscription, so the placeholder is removed after `/login` and comes back after you log out (the UI keeps it in sync all the time, the CLI on every `claude-bridge <route>`). A token of your own in `ANTHROPIC_AUTH_TOKEN` is left alone.

Nothing else in the file changes. `claude-bridge disconnect` (or the same switch) removes only the lines it added. While the proxy is not running, such a `claude` will not work.

## Providers

On the "Providers" tab, each group can be turned off: Anthropic (Claude), ChatGPT / Codex, OpenAI API, OpenAI-compatible, Google (Gemini, both accounts and `GEMINI_API_KEY`). A disabled provider:
- disappears from the UI: accounts, limits, the model list, settings and the request log; its limits are not fetched;
- cannot be the selected model the UI starts the proxy with;
- is blocked by the proxy: a request to it (say, `/model gemini`) gets a 400 error with a hint.

Logins and keys are kept. The list is stored in `CLAUDE_BRIDGE_DISABLED_PROVIDERS` (comma-separated `anthropic`, `codex`, `openai`, `openai-compat`, `gemini`) and applies immediately, no restart needed.

## Location and country restriction

The home page shows your current public IP, its country and flag. They come from [ipwho.is](https://ipwho.is), with [api.country.is](https://api.country.is) as a fallback. The lookup goes out the same way as provider traffic, so a system proxy or VPN is taken into account.

On the "Countries" tab you can allow proxying only from the checked countries: Germany, the Netherlands, Sweden, Estonia, Latvia, the United Kingdom, France, Lithuania, Switzerland, Poland, the USA. With the restriction on, the proxy checks the country before each request (the answer is cached for a minute). If the country is not on the list, nothing is sent to the provider and the proxy returns a 451 error naming the IP and country. If the country cannot be determined, the request is blocked too, unless a previous answer is less than 10 minutes old.

The settings are `GEO_GUARD_ENABLED` and `GEO_ALLOWED_COUNTRIES`; they apply immediately. The country list is `COUNTRIES` in [core/src/geo.rs](core/src/geo.rs).

## Subscription limits

`claude-bridge usage` and the "Accounts" tab show how much of each limit window is used and when it resets:

| Provider | Data source |
| --- | --- |
| Claude | `anthropic-ratelimit-unified-*` headers in Anthropic's replies to Claude Code; the proxy keeps the latest in `~/.claude-bridge/claude-usage.json`. They appear after the first Claude request through the proxy |
| ChatGPT / Codex | `chatgpt.com/backend-api/wham/usage`, the same request as `/status` in Codex CLI |
| Gemini | `retrieveUserQuota` in Code Assist. The standard Gemini API reports no limits |

The UI refreshes limits every minute and on demand.

When ChatGPT answers 429 `usage_limit_reached`, the proxy remembers the reset time and answers on its own until then instead of sending the conversation upstream; Claude Code gets `x-should-retry: false` and does not retry. The UI warns with a notification and the tray icon when a limit is reached, when Claude's or Codex's 5-hour window is 80% used, and when at the current pace it will run out before the reset.

Subscription details are shown there as well. For ChatGPT: the plan and the paid plan's end date from the ID token. For Claude: the plan, start date and billing type from `~/.claude.json`; Claude Code does not store an end date. Gemini has no such details. In the CLI, `claude-bridge status` shows them.

## UI

A Flutter app that controls the same background proxy:
- **Home**: current location (IP, country, flag), starting and stopping the proxy (by default it starts with the app), routing any `claude` through the proxy, the model for all sessions;
- **Providers**: which providers are in use ([details](#providers));
- **Accounts**: every provider in one place: Claude Code's login, browser login for ChatGPT and two Google accounts, OpenAI, Gemini and Anthropic API keys, subscription limits;
- **Requests**: a live request log with model, timing, tokens (input / cached / output) and errors;
- **Countries**: the country restriction ([details](#location-and-country-restriction));
- **Settings**: an editor for `~/.claude-bridge/.env`.

Notifications pop up as toasts in the bottom-right corner; their history opens from the button in the header, with a filter by type and copying of the text.

The UI looks for the CLI next to its own executable, then in `~/.cargo/bin`, then in `PATH`. The path can be set in the settings.

The usual workflow: turn on "Any claude through the proxy" on the home page, then run `claude` in any terminal or IDE and switch models with `/model` or by pinning.

The UI is in English or Russian: switch with the `RU` / `EN` button in the header. The system language is used by default.

## Settings

Everything lives in `~/.claude-bridge/` (another folder can be set with `CLAUDE_BRIDGE_HOME`):
- `.env`: keys and options;
- `codex-oauth.json`, `google-oauth*.json`: tokens, mode `0600`;
- `proxy.log`: the proxy log.

Environment variables take precedence over `.env`. After changing settings, restart the proxy: `claude-bridge stop` or the button in the UI. The exception is providers and the country restriction: the proxy re-reads them on every request.

| Variable | Purpose |
| --- | --- |
| `OPENAI_API_KEY`, `GEMINI_API_KEY`, `ANTHROPIC_API_KEY` | keys for the API routes |
| `OPENAI_COMPAT_BASE_URL` | OpenAI-compatible API URL; `/chat/completions` is appended automatically |
| `OPENAI_COMPAT_API_KEY` | optional OpenAI-compatible API key |
| `~/.claude-bridge/models.json` | custom models and the model selected in the UI |
| `CLAUDE_BRIDGE_PORT` | proxy port, `17870` by default |
| `CLAUDE_BRIDGE_CONTEXT_WARN_TOKENS` | context size per request at which the UI warns with a tray icon and a notification; default `150000`, `0` turns it off |
| `CODEX_REASONING_EFFORT` | Codex effort when no `@suffix` is given; `high` by default |
| `CODEX_REASONING_SUMMARY` | `auto` / `concise` / `detailed` / `none` |
| `CODEX_TEXT_VERBOSITY` | `low` / `medium` / `high` |
| `CODEX_CONTEXT_WINDOW_TOKENS`, `CODEX_AUTO_COMPACT_WINDOW_TOKENS`, `CODEX_DISABLE_COMPACT` | the context window `claude-bridge codex` reports to Claude Code |
| `CODEX_DISABLE_REASONING_CACHE` | `1`: don't pass encrypted reasoning on to the next turn |
| `ANTHROPIC_UPSTREAM_URL`, `GEMINI_BASE_URL` | alternative API addresses |
| `CLAUDE_BRIDGE_BIN` | CLI path for the UI |
| `CLAUDE_BRIDGE_AUTOSTART` | `0`: the UI doesn't start the proxy when it opens |
| `CLAUDE_BRIDGE_LANG` | UI language: `ru` or `en` |
| `CLAUDE_BRIDGE_DISABLED_PROVIDERS` | disabled providers: comma-separated `anthropic`, `codex`, `openai`, `gemini` |
| `GEO_GUARD_ENABLED` | `1`: proxy only from the allowed countries |
| `GEO_ALLOWED_COUNTRIES` | allowed countries, comma-separated ISO codes (`DE,NL,SE`); `-` for none; unset means every country on the list |
| `CLAUDE_BRIDGE_LOG` | log filter, e.g. `claude_bridge_core=debug` |

**ChatGPT login.**
- If you already have `~/.codex/auth.json` from Codex CLI, the proxy uses those tokens. Refreshed tokens are written back there, so Codex CLI keeps working.
- The built-in login listens on `localhost:1455`, like Codex CLI, so it can't run at the same time as `codex login`.

## Security

- The proxy listens on `127.0.0.1` only.
- Requests whose `Host` header is not `127.0.0.1:<port>` or `localhost:<port>` are rejected. This defeats DNS rebinding: a web page can't use your subscriptions through the proxy.
- The control endpoints (`/_model`, `/_shutdown`) require the `x-claude-bridge` header. A browser won't send it from another site without a CORS preflight.
- For the location card, the UI queries ipwho.is (or api.country.is) once a minute; the proxy does so only while the country restriction is on. These services see your public IP.

## Development

```
core/           proxy, providers, OAuth, routing, request log, geo check
cli/            the claude-bridge binary
ui/             Flutter app
ui/native/hub   the UI's Rust side on rinf (actors on top of core)
```

```bash
cargo test --workspace
cargo clippy --workspace --all-targets

cargo install rinf_cli --version 8.11.0   # once
cd ui && rinf gen                          # Dart bindings are not in git; rinf generates them
flutter run -d linux                       # or macos / windows
```

If the signals in `ui/native/hub/src/signals` change, run `rinf gen` again.

## Releases

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds the CLI and UI for Linux x64, macOS (universal) and Windows x64 and publishes a GitHub Release named after the version.

1. Commit your changes and run:
   ```bash
   scripts/release.sh 0.2.0
   ```
   The script bumps `cli/Cargo.toml`, commits it and pushes the commit together with the tag `v0.2.0`, so the tag always sits on the commit that carries the version.

The tag must match the version in `cli/Cargo.toml`, or the build stops. An existing tag can be rebuilt with "Run workflow" on the Actions tab.
