# claude-bridge UI

Flutter + [rinf](https://rinf.cunarist.org) панель управления для claude-bridge. Возможности и установка описаны в [README проекта](../README.md).

## Устройство

- `native/hub` — Rust-часть на акторах (`messages`), работает поверх `claude-bridge-core`:
  - `ProxyActor` следит за фоновым прокси, запускает и останавливает его, закрепляет модель, читает журнал запросов (`/_events`), прописывает прокси в настройки Claude Code;
  - `AccountsActor` отвечает за входы в ChatGPT и Google;
  - `SettingsActor` читает и пишет `~/.claude-bridge/.env`.
- `native/hub/src/signals` — сигналы между Dart и Rust. Dart-классы генерируются в `lib/src/bindings` и в git не хранятся.
- `lib/src/services` подписываются на сигналы Rust и отправляют команды, `lib/src/providers` хранят состояние для виджетов, `lib/src/pages` и `lib/src/controls` — интерфейс.

## Запуск

```bash
cargo install rinf_cli --version 8.11.0   # один раз
rinf gen                                   # после каждого изменения сигналов
flutter run -d linux                       # или macos / windows
```

Для запуска прокси UI нужен CLI `claude-bridge`: рядом с исполняемым файлом, в `~/.cargo/bin` или в `PATH`. Для разработки хватит `cargo install --path ../cli`.

На macOS App Sandbox выключен (`macos/Runner/*.entitlements`). Иначе приложение не сможет запускать прокси, править `~/.claude/settings.json` и читать `~/.claude-bridge` и `~/.codex`.
