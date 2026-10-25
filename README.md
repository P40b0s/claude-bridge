# claude-bridge

**Русский** | [English](README.en.md)

Локальный прокси с API Anthropic Messages. Через него Claude Code работает на других моделях:

- **Codex** — GPT-5.6 Sol / Terra / Luna и GPT-5.5 по подписке ChatGPT (вход через браузер или токены Codex CLI);
- **Gemini** — через вход в Google (Code Assist) или по `GEMINI_API_KEY`;
- **OpenAI API** — по `OPENAI_API_KEY`;
- **Anthropic API** — по `ANTHROPIC_API_KEY`.

Claude Code шлёт запросы на `http://127.0.0.1:17870/v1/messages`. Прокси переводит их в формат провайдера и возвращает ответ в виде SSE в формате Anthropic. Поддерживаются стриминг и обычные ответы, tool use, картинки, блоки thinking и встроенный веб-поиск провайдера. Модель можно сменить посреди сессии через `/model`, а для всех сессий сразу — из CLI или UI. Ненужных провайдеров можно отключить, а проксирование — разрешить только из выбранных стран.


## Установка

Готовые сборки лежат в [Releases](../../releases). Для каждой версии там есть:

| Файл | Что внутри |
| --- | --- |
| `claude-bridge-<версия>-linux-x64.tar.gz` | UI и CLI |
| `claude-bridge-<версия>-macos-universal.zip` | `claude_bridge_ui.app`, CLI внутри `Contents/MacOS` |
| `claude-bridge-<версия>-windows-x64.zip` | UI и `claude-bridge.exe` |
| `claude-bridge-cli-<версия>-<платформа>.*` | только CLI |
| `SHA256SUMS.txt` | контрольные суммы |

Нужен установленный [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude` в `PATH`).

Сборки не подписаны:
- **macOS.** При первом запуске откройте приложение через правый клик → «Открыть» или снимите карантин: `xattr -dr com.apple.quarantine claude_bridge_ui.app`.

Только CLI из исходников:

```bash
cargo install --path cli
```

## CLI

```bash
claude-bridge codex          # Claude Code на GPT-5.6 Sol (вход в ChatGPT при первом запуске)
claude-bridge gemini         # Claude Code на Gemini 3.1 Pro (вход в Google или GEMINI_API_KEY)
claude-bridge openai         # OpenAI API, нужен OPENAI_API_KEY
claude-bridge anthropic      # Claude Opus: вход Claude Code или ANTHROPIC_API_KEY
```

Команда поднимает прокси в фоне (или берёт уже запущенный) и запускает `claude`, направленный на него. Флаги:
- `-d` — передать в Claude Code `--dangerously-skip-permissions` (по умолчанию выключено: Claude Code спрашивает разрешения как обычно). Флаг действует только на сессию, которую запускает эта команда в терминале. Расширение для IDE и обычный `claude` запускаются без claude-bridge, поэтому режим разрешений для них выбирается в самом расширении или через `permissions.defaultMode` в `~/.claude/settings.json`;
- `--restart` — перезапустить прокси;
- всё после имени маршрута уходит в `claude` как есть.

Прокси — отдельный процесс, общий для всех сессий. Он работает и после выхода из Claude Code.

| Команда | Что делает |
| --- | --- |
| `claude-bridge status` | состояние прокси, входов и ключей |
| `claude-bridge connect` / `disconnect` | прописать прокси в `~/.claude/settings.json`, чтобы через него шёл любой `claude`, или убрать его оттуда |
| `claude-bridge usage` | [лимиты подписок](#лимиты-подписок): Claude, ChatGPT, Gemini |
| `claude-bridge stop` | остановить прокси |
| `claude-bridge login codex` / `login google [--account 2]` | войти через браузер; Google-аккаунт 2 подхватывается, когда первый упирается в лимит (429) |
| `claude-bridge logout codex` / `logout google [--account 2]` | удалить сохранённые токены |
| `claude-bridge model terra@max` | переключить все запущенные сессии на модель |
| `claude-bridge model --clear` | вернуть выбор модели через `/model` |
| `claude-bridge model --list` | список коротких имён |
| `claude-bridge serve` | запустить прокси на переднем плане |

## Модели

В Claude Code модель выбирается через `/model <имя>`. Короткие имена:

| Имя | Модель |
| --- | --- |
| `sol` (`codex`, `cx`) | `codex-oauth:gpt-5.6-sol`, усилие high |
| `terra`, `luna`, `gpt55` | GPT-5.6 Terra, GPT-5.6 Luna, GPT-5.5 |
| `fast` / `smart` / `deep` / `xhigh` / `max` | Sol с усилием low / medium / high / xhigh / max |
| `gemini`, `gemini-flash`, `gemini-3p`, `gemini-25p`, `gemini-25f` | Gemini через вход в Google |
| `opus`, `anthropic:sonnet`, `anthropic:haiku` | Claude: через вход Claude Code или по `ANTHROPIC_API_KEY` |

Ещё варианты:
- **Суффикс усилия.** `@low|medium|high|xhigh|max` работает с любой моделью, например `/model terra@max` или `/model gemini-flash@low`.
- **Явный провайдер.** `провайдер:модель`, где провайдер — `codex-oauth`, `openai`, `gemini-oauth`, `gemini` или `anthropic`.
- **Имя без провайдера** уходит текущему провайдеру как есть.

Claude Code сам делает служебные запросы к `claude-haiku-*` и `claude-sonnet-*`: заголовки, сабагенты, проверки. Прокси отправляет их провайдеру основной модели сессии (на его быструю модель), поэтому ключ Anthropic для них не нужен. В Claude они идут, только если основная модель сессии — Claude (`anthropic:…`).

### Claude и другие модели в одной сессии

Прокси сам в Claude по подписке не входит: условия Anthropic разрешают токены подписки только в её собственных приложениях. Вместо этого Claude Code пользуется своим входом, а прокси только пересылает его запросы.

- Если Claude Code вошёл (`/login`, по подписке или по ключу), `claude-bridge` его вход не трогает. Иначе Claude Code получает заглушку `ANTHROPIC_AUTH_TOKEN=claude-bridge`: с ней он не просит войти, весь трафик идёт к другим провайдерам, а модели Claude работают по `ANTHROPIC_API_KEY`.
- Claude выбирается явно: `/model opus` (`claude-bridge` направляет его на `anthropic:claude-opus-5-5`), `anthropic:sonnet` или `anthropic:haiku`. Запросы уходят прямо в Anthropic с входом Claude Code. Прокси пересылает их как есть и ничего не сохраняет.
- `/model sol`, `gemini` и другие модели работают через прокси, как обычно.
- Служебные запросы Claude Code (`claude-haiku-*`, `claude-sonnet-*` и т. п.) идут туда же, куда основная модель **этой** сессии. Пока выбрана не Claude, в Anthropic из сессии не уходит ни одного запроса. Сессии различаются по заголовку `X-Claude-Code-Session-Id` и друг на друга не влияют.
- Закреплённая модель («Модель для всех сессий») важнее всего.

Чтобы через прокси шёл и обычный `claude` (и расширения для IDE), выполните `claude-bridge connect` или включите «Любой claude через прокси» на главной в UI. В `~/.claude/settings.json` добавится:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:17870",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "anthropic:claude-opus-5-5",
    "ANTHROPIC_AUTH_TOKEN": "claude-bridge"
  }
}
```

`ANTHROPIC_AUTH_TOKEN` добавляется, только если Claude Code не вошёл сам: без него Claude Code и расширение для IDE просят войти, даже когда работают на GPT. Токен из `env` важнее подписки, поэтому после `/login` заглушка убирается, а после выхода — возвращается (UI следит за этим постоянно, CLI — при каждом запуске `claude-bridge <маршрут>`). Свой `ANTHROPIC_AUTH_TOKEN` не трогается.

Остальное содержимое файла не меняется. `claude-bridge disconnect` (или тот же переключатель) убирает только добавленные строки. Пока прокси не запущен, такой `claude` работать не будет.

## Провайдеры

На вкладке «Провайдеры» каждую группу можно отключить: Anthropic (Claude), ChatGPT / Codex, OpenAI API, Google (Gemini, оба аккаунта и `GEMINI_API_KEY`). Отключённый провайдер:
- пропадает из UI: аккаунтов, лимитов, списка моделей, настроек и журнала запросов; его лимиты не запрашиваются;
- не выбирается моделью по умолчанию, когда UI запускает прокси;
- блокируется прокси: на запрос к нему (например, `/model gemini`) приходит ошибка 400 с подсказкой.

Входы и ключи при этом сохраняются. Список хранится в `CLAUDE_BRIDGE_DISABLED_PROVIDERS` (`anthropic`, `codex`, `openai`, `gemini` через запятую) и применяется сразу, без перезапуска.

## Местоположение и ограничение по стране

На главной показаны текущий внешний IP, страна и её флаг. Их определяет [ipwho.is](https://ipwho.is), запасной вариант — [api.country.is](https://api.country.is). Запрос идёт тем же путём, что и трафик к провайдерам, так что учитывается системный прокси или VPN.

На вкладке «Страны» можно разрешить проксирование только из отмеченных стран: Германия, Нидерланды, Швеция, Эстония, Латвия, Великобритания, Франция, Литва, Швейцария, Польша, США. Когда ограничение включено, прокси перед запросом проверяет страну (ответ кэшируется на минуту) и, если её нет в списке, ничего не отправляет провайдеру, а возвращает ошибку 451 с IP и страной. Если страну определить не удалось, запрос тоже блокируется; исключение — прошлый ответ не старше 10 минут.

Настройки — `GEO_GUARD_ENABLED` и `GEO_ALLOWED_COUNTRIES`, применяются сразу. Список стран задан в `COUNTRIES` в [core/src/geo.rs](core/src/geo.rs).

## Лимиты подписок

`claude-bridge usage` и вкладка «Аккаунты» показывают, сколько израсходовано в каждом окне лимита и когда оно сбросится:

| Провайдер | Откуда данные |
| --- | --- |
| Claude | заголовки `anthropic-ratelimit-unified-*` из ответов Anthropic на запросы Claude Code; прокси запоминает последние в `~/.claude-bridge/claude-usage.json`. Появляются после первого запроса к Claude через прокси |
| ChatGPT / Codex | `chatgpt.com/backend-api/wham/usage`, тот же запрос, что делает `/status` в Codex CLI |
| Gemini | `retrieveUserQuota` в Code Assist. У стандартного Gemini API лимитов нет |

UI обновляет лимиты раз в минуту и по кнопке.

Там же видны сведения о подписках. Для ChatGPT это план и дата окончания платной подписки из ID-токена. Для Claude — план, дата начала и способ оплаты из `~/.claude.json`: даты окончания Claude Code не хранит. У Gemini таких сведений нет. В CLI это показывает `claude-bridge status`.

## UI

Приложение на Flutter — панель управления тем же фоновым прокси:
- **Главная**: текущее местоположение (IP, страна, флаг), запуск и остановка прокси (по умолчанию он запускается вместе с приложением), подключение любого `claude` к прокси, модель для всех сессий;
- **Провайдеры**: какие провайдеры используются ([подробнее](#провайдеры));
- **Аккаунты**: все провайдеры в одном месте: вход Claude Code, вход через браузер для ChatGPT и двух аккаунтов Google, API-ключи OpenAI, Gemini и Anthropic, лимиты подписок;
- **Запросы**: живой журнал запросов с моделью, временем, токенами (вход / из кэша / выход) и ошибками;
- **Страны**: ограничение проксирования по стране ([подробнее](#местоположение-и-ограничение-по-стране));
- **Настройки**: редактор `~/.claude-bridge/.env`.

Уведомления появляются тостами в правом нижнем углу, а их история открывается кнопкой в шапке: фильтр по типу, копирование текста.

Язык интерфейса — русский или английский — переключается кнопкой `RU` / `EN` в шапке. По умолчанию берётся язык системы.

UI ищет CLI в таком порядке: рядом со своим исполняемым файлом, в `~/.cargo/bin`, в `PATH`. Путь можно задать в настройках.

Обычный порядок работы: включить на главной «Любой claude через прокси», затем запускать `claude` в любом терминале или IDE и переключать модели через `/model` или закрепление.

## Настройки

Всё хранится в `~/.claude-bridge/` (другую папку можно задать через `CLAUDE_BRIDGE_HOME`):
- `.env` — ключи и параметры;
- `codex-oauth.json`, `google-oauth*.json` — токены, права `0600`;
- `proxy.log` — лог прокси.

Переменные окружения важнее `.env`. После изменения настроек перезапустите прокси: `claude-bridge stop` или кнопка в UI. Исключение — провайдеры и ограничение по стране: прокси перечитывает их на каждом запросе.

| Переменная | Назначение |
| --- | --- |
| `OPENAI_API_KEY`, `GEMINI_API_KEY`, `ANTHROPIC_API_KEY` | ключи для маршрутов по API |
| `CLAUDE_BRIDGE_PORT` | порт прокси, по умолчанию `17870` |
| `CODEX_REASONING_EFFORT` | усилие Codex, если не указан `@суффикс`; по умолчанию `high` |
| `CODEX_REASONING_SUMMARY` | `auto` / `concise` / `detailed` / `none` |
| `CODEX_TEXT_VERBOSITY` | `low` / `medium` / `high` |
| `CODEX_CONTEXT_WINDOW_TOKENS`, `CODEX_AUTO_COMPACT_WINDOW_TOKENS`, `CODEX_DISABLE_COMPACT` | окно контекста, которое `claude-bridge codex` сообщает Claude Code |
| `CODEX_DISABLE_REASONING_CACHE` | `1` — не передавать зашифрованные рассуждения в следующий ход |
| `ANTHROPIC_UPSTREAM_URL`, `GEMINI_BASE_URL` | другие адреса API |
| `CLAUDE_BRIDGE_BIN` | путь к CLI для UI |
| `CLAUDE_BRIDGE_AUTOSTART` | `0` — UI не запускает прокси при открытии |
| `CLAUDE_BRIDGE_LANG` | язык UI: `ru` или `en` |
| `CLAUDE_BRIDGE_DISABLED_PROVIDERS` | отключённые провайдеры: `anthropic`, `codex`, `openai`, `gemini` через запятую |
| `GEO_GUARD_ENABLED` | `1` — проксировать только из разрешённых стран |
| `GEO_ALLOWED_COUNTRIES` | разрешённые страны, коды ISO через запятую (`DE,NL,SE`); `-` — ни одной; без значения — все из списка |
| `CLAUDE_BRIDGE_LOG` | фильтр логов, например `claude_bridge_core=debug` |

**Вход в ChatGPT.**
- Если у вас уже есть `~/.codex/auth.json` от Codex CLI, прокси использует эти токены. Обновлённые токены записываются обратно туда же, чтобы Codex CLI продолжал работать.
- Собственный вход слушает `localhost:1455`, как и Codex CLI, поэтому одновременно с `codex login` он не запустится.

## Безопасность

- Прокси слушает только `127.0.0.1`.
- Запросы, у которых заголовок `Host` не равен `127.0.0.1:<порт>` или `localhost:<порт>`, отклоняются. Это защита от DNS rebinding: веб-страница не может пользоваться вашими подписками через прокси.
- Управляющие эндпоинты (`/_model`, `/_shutdown`) требуют заголовок `x-claude-bridge`. Браузер не отправит его с чужого сайта без CORS preflight.
- Для карточки местоположения UI раз в минуту обращается к ipwho.is (или api.country.is), а прокси — только пока включено ограничение по стране. Эти сервисы видят ваш внешний IP.

## Разработка

```
core/           прокси, провайдеры, OAuth, роутинг, журнал запросов, геопроверка
cli/            бинарник claude-bridge
ui/             Flutter-приложение
ui/native/hub   Rust-часть UI на rinf (акторы поверх core)
```

```bash
cargo test --workspace
cargo clippy --workspace --all-targets

cargo install rinf_cli --version 8.11.0   # один раз
cd ui && rinf gen                          # Dart-биндинги не хранятся в git, их генерирует rinf
flutter run -d linux                       # или macos / windows
```

Если поменялись сигналы в `ui/native/hub/src/signals`, заново запустите `rinf gen`.

## Релизы

[`.github/workflows/release.yml`](.github/workflows/release.yml) собирает CLI и UI под Linux x64, macOS (universal) и Windows x64 и публикует GitHub Release с именем, равным версии.

1. Поднимите версию в `cli/Cargo.toml`.
2. Создайте и отправьте тег:
   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

Тег должен совпадать с версией в `cli/Cargo.toml`, иначе сборка остановится. Пересобрать существующий тег можно через «Run workflow» во вкладке Actions.
