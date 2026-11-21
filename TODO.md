# TODO

## Account selection and browser proxy follow-ups

Issues found while reviewing commit `039e862` (`add account selection + browser proxy`).

### Authentication and account switching

- [x] Remove an inherited `ANTHROPIC_AUTH_TOKEN=claude-bridge` placeholder before launching Claude Code or starting `claude auth login`. Otherwise it shadows valid OAuth credentials and the proxy returns `424 No Anthropic credentials`.
  - `core/src/launcher.rs`
  - `ui/native/hub/src/actors/accounts.rs`

- [x] Do not treat unreadable or malformed existing Claude Code JSON as an empty object during account activation.
  - `core/src/auth/claude_code.rs:120-145`
  - Distinguish a missing file from read and parse failures.
  - Refuse activation when an existing credentials or configuration file cannot be read and parsed.
  - Otherwise activation can overwrite unrelated Claude Code configuration and credentials.

- [x] Make Claude account activation transactional across `.credentials.json` and `.claude.json`.
  - `core/src/auth/claude_code.rs:120-145`
  - Prepare and validate both resulting documents before writing either one.
  - If the second replacement fails, restore the first file or use another rollback-safe strategy.
  - A partial write currently can leave OAuth credentials and account metadata belonging to different accounts.

- [x] Prevent cancelled and replacement login tasks from racing with account restoration.
  - `ui/native/hub/src/actors/accounts.rs:162-183`
  - Wait for an aborted task to finish cleanup before starting another login, or serialize login transitions.
  - Overlapping Claude logins can restore stale shared credential files.

- [x] Prevent profile-name collisions on case-insensitive filesystems.
  - `core/src/auth/profiles.rs:23`
  - Canonicalize stored profile IDs or reject a new name when its case-folded form already exists.
  - Names such as `work` and `Work` currently refer to the same path on Windows and default macOS filesystems.

### Browser isolation and proxying

- [x] Check that the browser egress proxy is running before opening an isolated login profile.
  - `core/src/browser.rs:114`
  - Start or health-check the proxy first, or return an actionable error.
  - Currently the browser is configured to use a local port even when nothing is listening there.

- [x] Preserve browser isolation when no fixed browser is selected.
  - `core/src/browser.rs:61`
  - Resolve the system browser to an executable that supports the required profile and proxy arguments, or require explicit browser selection for named-account login.
  - The current fallback can reuse the ordinary browser profile, share cookies, and bypass the configured egress route.

- [x] Fix isolated-browser arguments on macOS.
  - `core/src/browser.rs:536`
  - Commands represented as `open -a <app> %u` need `--args` before Chromium or Firefox profile/proxy flags, or should invoke the application executable directly.
  - The current flags are passed to `open` itself rather than to the browser.

- [x] Reject an egress base port above `65534`, or configure the Codex browser proxy port independently.
  - `core/src/egress.rs:84`
  - Port `65535` is valid for the Anthropic listener but leaves no adjacent port for the Codex browser proxy.

## Additional findings

Found while re-checking the open items after `760ea23` (`fix accounts`). All open items above are still present in the code.

### Authentication and account switching

- [x] Detect an account change made outside the bridge before saving the "current" snapshot.
  - `core/src/auth/claude_code.rs:164-174` (`activate`), `:184-192` (`prepare_login`)
  - If the user runs `claude auth login` / `/login` directly, the shared profile holds account B while the active profile is still `work` (account A). The next switch calls `save_current("work")` and overwrites A's snapshot with B's tokens.
  - Compare `oauthAccount.accountUuid` (or the email) with the saved snapshot first. On a mismatch, save under a new profile name or ask the user.

- [x] `clear_active` can fail after a successful `claude auth logout`.
  - `core/src/auth/claude_code.rs:245-256`
  - `remove()` resets the active profile to `default`, then `activate(next)` calls `save_current("default")` whenever `account()` is still `Some`. If logout removed the credentials but left `oauthAccount`, `current_snapshot()` fails and the next account is never activated. If logout left both, it overwrites the `default` snapshot.
  - Skip saving the current snapshot in this path.

- [x] Ignore a stale `LoginFinished` from a replaced login.
  - `ui/native/hub/src/actors/accounts.rs:370-372`
  - `self.login = None` runs without checking that the message belongs to the current login. A message queued just before `abort()` clears the new login's handle, so `CancelLogin` can no longer stop it. Add a login generation id.

### Browser isolation and proxying

- [x] Windows: `cmd /C start "" <url>` breaks OAuth URLs at `&`.
  - `core/src/browser.rs:264-279`, `spawn`
  - Rust does not quote an argument that has no spaces, so `cmd` treats `&` as a command separator. The browser opens a truncated URL without `state` / `code_challenge`. Escape the URL for `cmd` (`^&`), use `raw_arg` with quotes, or call `ShellExecuteW` / `rundll32 url.dll,FileProtocolHandler`.

- [x] Plain-HTTP keep-alive connections through the egress proxy go to the wrong host.
  - `core/src/egress.rs:292-296`
  - Only the first request head is parsed. After that the connection is piped to that host with `copy_bidirectional`. Browsers reuse proxy connections, so a later `http://other-host/` request (OCSP, for example) goes to the first host's server. Parse each request or force `Connection: close`.

- [x] An isolated browser profile still leaks the real IP through WebRTC.
  - `core/src/browser.rs:149-157` (Chromium), `:187-203` (Firefox)
  - An HTTP proxy does not cover WebRTC UDP. Add `--force-webrtc-ip-handling-policy=disable_non_proxied_udp` for Chromium and `media.peerconnection.ice.proxy_only=true` (or `media.peerconnection.enabled=false`) for Firefox. Also consider `network.trr.mode=5` and `network.dns.disablePrefetch`.

- [x] A second link in an already open isolated Firefox profile fails.
  - `core/src/browser.rs:133-140`
  - With `--no-remote`, Firefox reports "already running" when the profile is locked, for example when opening the usage page while the login window is still open. Drop `--no-remote` and keep `--profile`, which routes the link to the running instance of that profile.

- [x] Snap/Flatpak browsers may be unable to open the profile directory.
  - `core/src/browser.rs:118-123`
  - Snap Firefox (the default on Ubuntu) cannot access hidden directories in `$HOME` such as `~/.local/share/...`. Flatpak needs a filesystem permission. Detect these cases and report an error, or put the profile in an allowed location (`~/snap/<name>/common/...`).

- [x] Browser type detection by substring is unreliable.
  - `core/src/browser.rs:117-147`
  - `joined.contains("zen" | "opera" | "edge" | ...)` matches any part of the path and arguments, such as a user name or directory. Match on the executable file name or the `.desktop` id instead.
