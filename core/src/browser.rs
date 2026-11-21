//! Which browser opens links (logins, claude.ai pages): the system default, or one fixed in
//! `CLAUDE_BRIDGE_BROWSER`, for example a browser whose own traffic goes out the same exit.
//!
//! The value is an id from [`installed`] (a `.desktop` file on Linux, an `.app` path on macOS, a
//! `StartMenuInternet` client on Windows), or a command: a program path, optionally with
//! arguments; `%u` in it is replaced by the link, otherwise the link goes last. A fixed browser
//! that cannot be started is an error: the link never falls back to the default one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use crate::{config, settings};

pub const KEY: &str = "CLAUDE_BRIDGE_BROWSER";
/// Non-empty: use a separate profile with this name and force it through the provider's egress.
pub const PROFILE_KEY: &str = "CLAUDE_BRIDGE_BROWSER_PROFILE";
/// Per-login override inherited by Claude Code's `BROWSER` opener.
pub const PROFILE_OVERRIDE_KEY: &str = "CLAUDE_BRIDGE_BROWSER_PROFILE_OVERRIDE";
/// `resist` standardizes Firefox's fingerprint; `exit` uses the bridge time zone and locale.
pub const FINGERPRINT_KEY: &str = "CLAUDE_BRIDGE_BROWSER_FINGERPRINT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// What `CLAUDE_BRIDGE_BROWSER` holds.
    pub id: String,
    pub name: String,
    /// Program and arguments; `%u` marks the link.
    command: Vec<String>,
}

/// The fixed browser setting: the .env file first, then the environment.
pub fn setting() -> Option<String> {
    settings::read()
        .remove(KEY)
        .or_else(|| config::env(KEY))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Opens `url` in the fixed browser, or the system default when none is set. When a profile is
/// configured, it is isolated from the user's usual browser and forced through `family`'s egress.
pub fn open(url: &str, family: &str) -> anyhow::Result<()> {
    open_with_profile(url, family, None)
}

pub fn open_with_profile(url: &str, family: &str, profile: Option<&str>) -> anyhow::Result<()> {
    let choice = setting();
    let mut command = match &choice {
        Some(choice) => resolve(choice)?,
        None => default_command(),
    };
    let account_profile = profile
        .map(str::to_string)
        .or_else(|| config::env(PROFILE_OVERRIDE_KEY));
    let configured_profile = profile_setting();
    if choice.is_none() && configured_profile.is_some() {
        bail!("{PROFILE_KEY} needs a browser selected in {KEY}");
    }
    // A named account can still log in with the system browser when no fixed browser was chosen;
    // it simply cannot get automatic profile isolation or browser proxying in that case.
    if let Some(profile) = account_profile.or(configured_profile)
        && choice.is_some()
    {
        isolate(&mut command, family, &profile)?;
    }
    spawn(&command, url)
}

fn profile_setting() -> Option<String> {
    settings::read()
        .remove(PROFILE_KEY)
        .or_else(|| config::env(PROFILE_KEY))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn fingerprint_setting() -> String {
    settings::read()
        .remove(FINGERPRINT_KEY)
        .or_else(|| config::env(FINGERPRINT_KEY))
        .filter(|v| v == "exit" || v == "resist")
        .unwrap_or_else(|| "resist".into())
}

fn safe_name(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if value.trim_matches(['.', '_']).is_empty() {
        "default".into()
    } else {
        value
    }
}

fn insert_before_url(command: &mut Vec<String>, args: impl IntoIterator<Item = String>) {
    let at = command
        .iter()
        .position(|a| a.contains("%u") || a.contains("%1"))
        .unwrap_or(command.len());
    command.splice(at..at, args);
}

fn isolate(command: &mut Vec<String>, family: &str, profile: &str) -> anyhow::Result<()> {
    let proxy = crate::egress::family_url(family)
        .with_context(|| format!("no browser egress proxy for provider family {family}"))?;
    let port = crate::egress::family_port(family).context("browser egress proxy is disabled")?;
    let joined = command.join(" ").to_ascii_lowercase();
    let root = config::data_dir()
        .join("browser")
        .join(safe_name(profile))
        .join(family);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating browser profile {}", root.display()))?;

    let fingerprint = fingerprint_setting();
    let locale = crate::claude_settings::language();
    if joined.contains("firefox")
        || joined.contains("librewolf")
        || joined.contains("waterfox")
        || joined.contains("zen")
    {
        write_firefox_prefs(&root, port, fingerprint == "resist", locale.as_deref())?;
        insert_before_url(
            command,
            [
                "--no-remote".into(),
                "--profile".into(),
                root.display().to_string(),
            ],
        );
        return Ok(());
    }
    if [
        "chromium", "chrome", "brave", "edge", "vivaldi", "opera", "thorium", "yandex",
    ]
    .iter()
    .any(|name| joined.contains(name))
    {
        let mut args = vec![
            format!("--user-data-dir={}", root.display()),
            format!("--proxy-server={proxy}"),
            "--proxy-bypass-list=localhost;127.0.0.1;[::1]".into(),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-sync".into(),
            "--disable-background-networking".into(),
        ];
        if fingerprint == "exit"
            && let Some(locale) = locale
        {
            args.push(format!("--lang={}", browser_locale(&locale)));
        }
        insert_before_url(command, args);
        return Ok(());
    }
    bail!(
        "the selected browser cannot be isolated automatically; use Firefox, Chromium, Chrome, Brave, Edge, Vivaldi or Opera"
    )
}

fn browser_locale(locale: &str) -> String {
    locale.split('.').next().unwrap_or(locale).replace('_', "-")
}

fn write_firefox_prefs(
    profile: &Path,
    port: u16,
    resist: bool,
    locale: Option<&str>,
) -> anyhow::Result<()> {
    let locale = locale.map(browser_locale).unwrap_or_else(|| "en-US".into());
    let privacy = if resist {
        "user_pref(\"privacy.resistFingerprinting\", true);\nuser_pref(\"privacy.resistFingerprinting.letterboxing\", true);"
    } else {
        "user_pref(\"privacy.resistFingerprinting\", false);"
    };
    let prefs = format!(
        r#"// Managed by claude-bridge. This profile is intentionally separate from the normal browser.
user_pref("network.proxy.type", 1);
user_pref("network.proxy.http", "127.0.0.1");
user_pref("network.proxy.http_port", {port});
user_pref("network.proxy.ssl", "127.0.0.1");
user_pref("network.proxy.ssl_port", {port});
user_pref("network.proxy.no_proxies_on", "localhost, 127.0.0.1, ::1");
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("datareporting.healthreport.uploadEnabled", false);
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("toolkit.telemetry.enabled", false);
user_pref("intl.locale.requested", "{locale}");
user_pref("intl.accept_languages", "{locale}, en-US, en");
{privacy}
"#,
    );
    std::fs::write(profile.join("user.js"), prefs)
        .with_context(|| format!("writing Firefox profile {}", profile.display()))
}

/// A tiny opener Claude Code can call through its standard `BROWSER` variable. This keeps `/login`
/// on the same browser/profile policy as links opened by the bridge UI.
pub fn claude_code_opener() -> anyhow::Result<Option<String>> {
    if setting().is_none() {
        return Ok(None);
    }
    let cli =
        crate::control::find_cli().context("cannot find claude-bridge for the browser opener")?;
    let dir = config::data_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(windows)]
    {
        let path = dir.join("browser-open.cmd");
        let cli = cli.display().to_string().replace('"', "\"\"");
        std::fs::write(
            &path,
            format!("@\"{cli}\" browser --family anthropic %*\r\n"),
        )?;
        return Ok(Some(path.display().to_string()));
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("browser-open");
        let quoted = format!("'{}'", cli.display().to_string().replace('\'', "'\\''"));
        std::fs::write(
            &path,
            format!("#!/bin/sh\nexec {quoted} browser --family anthropic \"$@\"\n"),
        )?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        Ok(Some(path.display().to_string()))
    }
}

fn resolve(choice: &str) -> anyhow::Result<Vec<String>> {
    if let Some(b) = installed().into_iter().find(|b| b.id == choice) {
        return Ok(b.command);
    }
    #[cfg(target_os = "macos")]
    if choice.ends_with(".app") && Path::new(choice).is_dir() {
        return Ok(vec!["open".into(), "-a".into(), choice.into(), "%u".into()]);
    }
    if choice.ends_with(".desktop") {
        bail!("{KEY}={choice}: no such browser installed");
    }
    // A whole path with spaces in it, or a command line.
    let command = if Path::new(choice).is_file() {
        vec![choice.to_string()]
    } else {
        split_command(choice)
    };
    anyhow::ensure!(!command.is_empty(), "{KEY} is empty");
    Ok(command)
}

fn default_command() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec!["open".into(), "%u".into()]
    } else if cfg!(windows) {
        // `start` treats the first quoted argument as a window title.
        vec![
            "cmd".into(),
            "/C".into(),
            "start".into(),
            String::new(),
            "%u".into(),
        ]
    } else {
        vec!["xdg-open".into(), "%u".into()]
    }
}

fn spawn(command: &[String], url: &str) -> anyhow::Result<()> {
    let mut args: Vec<String> = Vec::with_capacity(command.len() + 1);
    let mut placed = false;
    for arg in command {
        // `%1` is how Windows registry commands mark it.
        if arg.contains("%u") || arg.contains("%1") {
            args.push(arg.replace("%u", url).replace("%1", url));
            placed = true;
        } else {
            args.push(arg.clone());
        }
    }
    if !placed {
        args.push(url.to_string());
    }
    let (program, rest) = args.split_first().context("empty browser command")?;
    let mut cmd = Command::new(program);
    cmd.args(rest);
    // In `exit` fingerprint mode these affect the browser's process-level locale/time zone. Firefox
    // RFP intentionally overrides the visible JS time zone in `resist` mode.
    if fingerprint_setting() == "exit" {
        if let Some(tz) = crate::claude_settings::time_zone() {
            cmd.env("TZ", tz);
        }
        if let Some(locale) = crate::claude_settings::language() {
            cmd.env("LANG", &locale).env("LC_ALL", locale);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting browser {program}"))?;
    // Reaped in the background, so a long-running browser leaves no zombie behind.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Splits a command line on spaces, keeping "quoted parts" together.
fn split_command(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            // Only \" and \\ are escapes, so Windows paths keep their backslashes.
            '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => cur.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Browsers installed on this system, sorted by name.
pub fn installed() -> Vec<Browser> {
    let mut list = platform_browsers();
    list.sort_by_key(|b| b.name.to_lowercase());
    list
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_browsers() -> Vec<Browser> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| n.ends_with(".desktop"))
            else {
                continue;
            };
            // The first directory holding an id wins, as for the desktop itself.
            if !seen.insert(id.to_string()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(b) = parse_desktop(id, &text) {
                out.push(b);
            }
        }
    }
    out
}

/// `$XDG_DATA_HOME` first, then `$XDG_DATA_DIRS`, plus Flatpak's exports in case the session
/// left them out.
#[cfg(all(unix, not(target_os = "macos")))]
fn application_dirs() -> Vec<PathBuf> {
    let home = config::home_dir();
    let data_home = config::env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs =
        config::env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs = vec![data_home];
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs.push(home.join(".local/share/flatpak/exports/share"));
    dirs.push("/var/lib/flatpak/exports/share".into());
    dirs.push("/var/lib/snapd/desktop".into());
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs.into_iter().map(|d| d.join("applications")) {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// A browser entry: in the `WebBrowser` category or handling https links, shown in menus.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn parse_desktop(id: &str, text: &str) -> Option<Browser> {
    let mut in_entry = false;
    let (mut name, mut exec, mut categories, mut mime) = (None, None, "", "");
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Type" if value != "Application" => return None,
            "NoDisplay" | "Hidden" if value == "true" => return None,
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value),
            "Categories" => categories = value,
            "MimeType" => mime = value,
            _ => {}
        }
    }
    let is_browser = categories.split(';').any(|c| c == "WebBrowser")
        || mime.split(';').any(|m| m == "x-scheme-handler/https");
    if !is_browser {
        return None;
    }
    let command = exec_command(exec?, name.as_deref().unwrap_or(id));
    if command.is_empty() {
        return None;
    }
    Some(Browser {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.trim_end_matches(".desktop").to_string()),
        command,
    })
}

/// The `Exec` line as arguments: field codes for files and links become `%u`, the rest go away.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn exec_command(exec: &str, name: &str) -> Vec<String> {
    // Desktop files escape backslashes once more than the command line does.
    let exec = exec.replace("\\\\", "\\");
    let mut out = Vec::new();
    for arg in split_command(&exec) {
        match arg.as_str() {
            "%u" | "%U" | "%f" | "%F" => out.push("%u".into()),
            "%i" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" | "%k" => {}
            // Flatpak's file forwarding markers.
            "@@u" | "@@" | "@@f" => {}
            _ => {
                let arg = arg.replace("%c", name).replace("%%", "%");
                out.push(arg);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn platform_browsers() -> Vec<Browser> {
    const KNOWN: &[&str] = &[
        "Safari",
        "Google Chrome",
        "Google Chrome Canary",
        "Firefox",
        "Firefox Developer Edition",
        "Firefox Nightly",
        "Brave Browser",
        "Microsoft Edge",
        "Arc",
        "Opera",
        "Opera GX",
        "Vivaldi",
        "Chromium",
        "Yandex",
        "Orion",
        "Zen",
        "Zen Browser",
        "LibreWolf",
        "Tor Browser",
        "Waterfox",
        "Mullvad Browser",
        "Dia",
        "Thorium",
    ];
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        config::home_dir().join("Applications"),
    ];
    dirs.push("/System/Volumes/Preboot/Cryptexes/App/System/Applications".into());
    let mut out: Vec<Browser> = Vec::new();
    for name in KNOWN {
        let Some(path) = dirs
            .iter()
            .map(|d| d.join(format!("{name}.app")))
            .find(|p| p.is_dir())
        else {
            continue;
        };
        if out.iter().any(|b| b.name == *name) {
            continue;
        }
        let id = path.display().to_string();
        out.push(Browser {
            command: vec!["open".into(), "-a".into(), id.clone(), "%u".into()],
            id,
            name: name.to_string(),
        });
    }
    out
}

#[cfg(windows)]
fn platform_browsers() -> Vec<Browser> {
    let mut out: Vec<Browser> = Vec::new();
    for root in ["HKCU", "HKLM", r"HKLM\SOFTWARE\WOW6432Node"] {
        let root = if root.starts_with("HKLM\\") {
            root.to_string()
        } else {
            format!(r"{root}\SOFTWARE")
        };
        let key = format!(r"{root}\Clients\StartMenuInternet");
        let Ok(output) = Command::new("reg")
            .args(["query", &key, "/s"])
            .stdin(Stdio::null())
            .output()
        else {
            continue;
        };
        for b in parse_reg(&String::from_utf8_lossy(&output.stdout)) {
            if !out.iter().any(|o| o.id == b.id) {
                out.push(b);
            }
        }
    }
    out
}

/// `reg query …\StartMenuInternet /s`: each client's display name and `shell\open\command`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg(text: &str) -> Vec<Browser> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut commands: Vec<(String, String)> = Vec::new();
    let mut key = String::new();
    for line in text.lines() {
        if line.starts_with("HKEY_") {
            key = line.trim().to_string();
            continue;
        }
        let Some((_, value)) = line.split_once("REG_SZ") else {
            continue;
        };
        if !line.trim_start().starts_with("(") {
            continue; // Only default values.
        }
        let value = value.trim().to_string();
        let Some((_, rest)) = key.split_once(r"StartMenuInternet\") else {
            continue;
        };
        match rest.split_once('\\') {
            None => names.push((rest.to_string(), value)),
            Some((client, path)) if path.eq_ignore_ascii_case(r"shell\open\command") => {
                commands.push((client.to_string(), value))
            }
            _ => {}
        }
    }
    commands
        .into_iter()
        .filter_map(|(client, line)| {
            let command = split_command(&line);
            if command.is_empty() {
                return None;
            }
            let name = names
                .iter()
                .find(|(c, _)| *c == client)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| client.clone());
            Some(Browser {
                id: client,
                name,
                command,
            })
        })
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn platform_browsers() -> Vec<Browser> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_is_parsed() {
        let text = "[Desktop Entry]\nType=Application\nName=Firefox\nName[ru]=Файрфокс\nExec=firefox --name %c %u\n\
                    Categories=Network;WebBrowser;\n[Desktop Action new-window]\nExec=firefox --new-window %u\n";
        let b = parse_desktop("firefox.desktop", text).unwrap();
        assert_eq!(b.name, "Firefox");
        assert_eq!(b.command, ["firefox", "--name", "Firefox", "%u"]);
    }

    #[test]
    fn flatpak_markers_and_non_browsers() {
        let text = "[Desktop Entry]\nName=Brave\nExec=/usr/bin/flatpak run --command=brave com.brave.Browser @@u %U @@\n\
                    MimeType=text/html;x-scheme-handler/https;\n";
        let b = parse_desktop("com.brave.Browser.desktop", text).unwrap();
        assert_eq!(
            b.command,
            [
                "/usr/bin/flatpak",
                "run",
                "--command=brave",
                "com.brave.Browser",
                "%u"
            ]
        );
        assert!(
            parse_desktop(
                "x.desktop",
                "[Desktop Entry]\nName=Editor\nExec=ed %f\nCategories=Utility;\n"
            )
            .is_none()
        );
        assert!(
            parse_desktop(
                "y.desktop",
                "[Desktop Entry]\nName=B\nExec=b\nNoDisplay=true\nCategories=WebBrowser;\n"
            )
            .is_none()
        );
    }

    #[test]
    fn quoted_commands_split() {
        assert_eq!(
            split_command(r#""C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url "%1""#),
            [
                r"C:\Program Files\Mozilla Firefox\firefox.exe",
                "-osint",
                "-url",
                "%1"
            ]
        );
        assert_eq!(
            split_command("chromium --profile-directory=Work %u"),
            ["chromium", "--profile-directory=Work", "%u"]
        );
    }

    #[test]
    fn reg_output_is_parsed() {
        let text = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Clients\\StartMenuInternet\\FIREFOX.EXE\r\n    (Default)    REG_SZ    Mozilla Firefox\r\n\r\n\
                    HKEY_LOCAL_MACHINE\\SOFTWARE\\Clients\\StartMenuInternet\\FIREFOX.EXE\\shell\\open\\command\r\n    (Default)    REG_SZ    \"C:\\Program Files\\Mozilla Firefox\\firefox.exe\"\r\n";
        let list = parse_reg(text);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "FIREFOX.EXE");
        assert_eq!(list[0].name, "Mozilla Firefox");
        assert_eq!(
            list[0].command,
            [r"C:\Program Files\Mozilla Firefox\firefox.exe"]
        );
    }
}
