# TODO

## Account selection and browser proxy follow-ups

Issues found while reviewing commit `039e862` (`add account selection + browser proxy`).

### Authentication and account switching

- [x] Remove an inherited `ANTHROPIC_AUTH_TOKEN=claude-bridge` placeholder before launching Claude Code or starting `claude auth login`. Otherwise it shadows valid OAuth credentials and the proxy returns `424 No Anthropic credentials`.
  - `core/src/launcher.rs`
  - `ui/native/hub/src/actors/accounts.rs`

- [ ] Do not treat unreadable or malformed existing Claude Code JSON as an empty object during account activation.
  - `core/src/auth/claude_code.rs:120-145`
  - Distinguish a missing file from read and parse failures.
  - Refuse activation when an existing credentials or configuration file cannot be read and parsed.
  - Otherwise activation can overwrite unrelated Claude Code configuration and credentials.

- [ ] Make Claude account activation transactional across `.credentials.json` and `.claude.json`.
  - `core/src/auth/claude_code.rs:120-145`
  - Prepare and validate both resulting documents before writing either one.
  - If the second replacement fails, restore the first file or use another rollback-safe strategy.
  - A partial write currently can leave OAuth credentials and account metadata belonging to different accounts.

- [ ] Prevent cancelled and replacement login tasks from racing with account restoration.
  - `ui/native/hub/src/actors/accounts.rs:162-183`
  - Wait for an aborted task to finish cleanup before starting another login, or serialize login transitions.
  - Overlapping Claude logins can restore stale shared credential files.

- [ ] Prevent profile-name collisions on case-insensitive filesystems.
  - `core/src/auth/profiles.rs:23`
  - Canonicalize stored profile IDs or reject a new name when its case-folded form already exists.
  - Names such as `work` and `Work` currently refer to the same path on Windows and default macOS filesystems.

### Browser isolation and proxying

- [ ] Check that the browser egress proxy is running before opening an isolated login profile.
  - `core/src/browser.rs:114`
  - Start or health-check the proxy first, or return an actionable error.
  - Currently the browser is configured to use a local port even when nothing is listening there.

- [ ] Preserve browser isolation when no fixed browser is selected.
  - `core/src/browser.rs:61`
  - Resolve the system browser to an executable that supports the required profile and proxy arguments, or require explicit browser selection for named-account login.
  - The current fallback can reuse the ordinary browser profile, share cookies, and bypass the configured egress route.

- [ ] Fix isolated-browser arguments on macOS.
  - `core/src/browser.rs:536`
  - Commands represented as `open -a <app> %u` need `--args` before Chromium or Firefox profile/proxy flags, or should invoke the application executable directly.
  - The current flags are passed to `open` itself rather than to the browser.

- [ ] Reject an egress base port above `65534`, or configure the Codex browser proxy port independently.
  - `core/src/egress.rs:84`
  - Port `65535` is valid for the Anthropic listener but leaves no adjacent port for the Codex browser proxy.
