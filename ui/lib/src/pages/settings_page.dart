import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:claude_bridge_ui/src/services/context_watch.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

enum _Kind { text, secret, number, choice, flag }

class _Field {
  const _Field(
    this.key,
    this.label, {
    this.kind = _Kind.text,
    this.hint,
    this.options = const [],
    this.family,
  });
  final String key;

  ///Поле прячется, когда эта группа провайдеров отключена
  final String? family;
  final String label;
  final _Kind kind;
  final String? hint;

  ///Для choice: допустимые значения, пустая строка - значение по умолчанию
  final List<String> options;
}

class _Group {
  const _Group(this.title, this.icon, this.fields, {this.family});
  final String title;
  final String? family;
  final IconData icon;
  final List<_Field> fields;
}

List<_Group> get _groups => [
  //API-ключи задаются на странице «Аккаунты»
  _Group('Codex', Icons.auto_awesome, [
    _Field(
      'CODEX_REASONING_SUMMARY',
      tr('Сводка рассуждений', 'Reasoning summary'),
      kind: _Kind.choice,
      options: ['', 'auto', 'concise', 'detailed', 'none'],
      hint: tr(
        'Что показывать в блоке thinking; по умолчанию auto',
        'What the thinking block shows; auto by default',
      ),
    ),
    _Field(
      'CODEX_TEXT_VERBOSITY',
      tr('Подробность ответов', 'Answer verbosity'),
      kind: _Kind.choice,
      options: ['', 'low', 'medium', 'high'],
    ),
    _Field(
      'CODEX_CONTEXT_WINDOW_TOKENS',
      tr('Окно контекста, токенов', 'Context window, tokens'),
      kind: _Kind.number,
      hint: tr('По умолчанию 372000', 'Default 372000'),
    ),
    _Field(
      'CODEX_AUTO_COMPACT_WINDOW_TOKENS',
      tr('Автосжатие с, токенов', 'Auto-compact at, tokens'),
      kind: _Kind.number,
      hint: tr(
        'По умолчанию 200000. Каждый вызов инструмента заново отправляет весь контекст, поэтому длинная сессия дорогая. '
        'Для «любого claude через прокси» значение пишется в ~/.claude/settings.json',
        'Default 200000. Every tool call re-sends the whole context, so a long session is expensive. '
        'For "any claude through the proxy" the value goes into ~/.claude/settings.json',
      ),
    ),
    _Field(
      'CODEX_DISABLE_COMPACT',
      tr('Отключить автосжатие', 'Disable auto-compact'),
      kind: _Kind.flag,
    ),
    _Field(
      'CODEX_PARALLEL_TOOL_CALLS',
      tr('Параллельные вызовы инструментов', 'Parallel tool calls'),
      kind: _Kind.choice,
      options: ['', '1', '0'],
      hint: tr(
        'По умолчанию включены: несколько инструментов за ход, меньше ходов и повторных отправок контекста. 0 - по одному',
        'On by default: several tools per turn, fewer turns and context re-sends. 0 - one at a time',
      ),
    ),
    _Field(
      'CODEX_DISABLE_REASONING_CACHE',
      tr(
        'Не переиспользовать рассуждения между ходами',
        "Don't reuse reasoning between turns",
      ),
      kind: _Kind.flag,
    ),
  ], family: 'codex'),
  _Group(tr('Прокси', 'Proxy'), Icons.hub_outlined, [
    _Field(
      'CLAUDE_BRIDGE_PORT',
      tr('Порт', 'Port'),
      kind: _Kind.number,
      hint: tr('По умолчанию 17870', 'Default 17870'),
    ),
    _Field(
      'CLAUDE_BRIDGE_DNS',
      tr('DNS', 'DNS'),
      kind: _Kind.choice,
      options: ['', 'doh', 'system'],
      hint: tr(
        'По умолчанию doh: имена провайдеров и SSH-серверов резолвятся через DNS over HTTPS (серверы ниже), '
            'системный DNS их не видит. system - обычный резолвер (нужен, если VPN-клиент работает в режиме fake-ip). '
            'Применяется после перезапуска прокси',
        'doh by default: provider and SSH server names are resolved over DNS over HTTPS (servers below), '
            'the system DNS never sees them. system - the usual resolver (needed if a VPN client runs in fake-ip mode). '
            'Applies after a proxy restart',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_IPV6',
      tr('IPv6', 'IPv6'),
      kind: _Kind.choice,
      options: ['', '1', '0'],
      hint: tr(
        '0 - соединяться только по IPv4: на случай VPN, который туннелирует IPv4, а IPv6 пускает мимо себя. '
            'Применяется после перезапуска прокси',
        '0 - connect over IPv4 only: for a VPN that tunnels IPv4 and lets IPv6 go around it. Applies after a proxy restart',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_EGRESS_PORT',
      tr('Порт для прочего трафика Claude Code', "Port for Claude Code's other traffic"),
      kind: _Kind.number,
      hint: tr(
        'Всё, что Claude Code шлёт помимо запросов к модели (OAuth, MCP, WebFetch, запускаемые им команды), идёт через '
            'этот порт, выход и проверку страны Anthropic. По умолчанию порт прокси + 1; 0 - отключить',
        'Everything Claude Code sends besides model calls (OAuth, MCP, WebFetch, the commands it runs) goes through this '
            'port, the Anthropic exit and its country check. Default: the proxy port + 1; 0 turns it off',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_NO_PROXY',
      tr('Прямые адреса Claude Code', 'Claude Code direct hosts'),
      hint: tr(
        'Домены, хосты или IP-адреса через запятую, которые Claude Code и запускаемые им процессы открывают напрямую, '
            'мимо выходного прокси. Нужно для внутренних адресов, которых нет в публичном DNS: например .sminex.com. '
            'Локальные адреса моста добавляются автоматически. Применяется к новым сессиям claude',
        'Comma-separated domains, hosts, or IP addresses that Claude Code and its child processes reach directly, '
            'bypassing the egress proxy. Use for private addresses absent from public DNS, for example .sminex.com. '
            'The bridge local addresses are added automatically. Applies to new claude sessions',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_TZ',
      tr('Часовой пояс для Claude Code', 'Time zone for Claude Code'),
      hint: tr(
        'Передаётся как TZ в Claude Code и запускаемые им команды: время в git-коммитах, вывод date. '
            'auto - пояс страны выхода Anthropic (если страна неизвестна - UTC); или явно: UTC, Europe/Berlin. '
            'Пусто - системный. Применяется к новым сессиям claude',
        'Passed as TZ to Claude Code and the commands it runs: git commit times, date output. '
            'auto - the zone of the Anthropic exit\'s country (UTC while it is unknown); or a name: UTC, Europe/Berlin. '
            'Empty - the system one. Applies to new claude sessions',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_LOCALE',
      tr('Язык системы для Claude Code', 'System language for Claude Code'),
      kind: _Kind.choice,
      options: ['', 'en_US.UTF-8', 'C.UTF-8', 'de_DE.UTF-8', 'fr_FR.UTF-8', 'ru_RU.UTF-8'],
      hint: tr(
        'Передаётся как LANG и LC_ALL в Claude Code и запускаемые им команды: их вывод (даты, ошибки, git) '
            'уходит модели на этом языке. Локаль должна быть установлена в системе. Применяется к новым сессиям claude',
        'Passed as LANG and LC_ALL to Claude Code and the commands it runs: their output (dates, errors, git) reaches '
            'the model in this language. The locale must be installed. Applies to new claude sessions',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_DOH_SERVERS',
      tr('Серверы DNS over HTTPS', 'DNS over HTTPS servers'),
      hint: tr(
        'Через запятую, по порядку; JSON API (application/dns-json), хост - только IP. '
            'По умолчанию https://1.1.1.1/dns-query,https://8.8.8.8/resolve',
        'Comma-separated, tried in order; JSON API (application/dns-json), host by IP only. '
            'Default https://1.1.1.1/dns-query,https://8.8.8.8/resolve',
      ),
    ),
    _Field(
      'ANTHROPIC_UPSTREAM_URL',
      tr('Адрес Anthropic API', 'Anthropic API address'),
      hint: tr(
        'По умолчанию https://api.anthropic.com',
        'Default https://api.anthropic.com',
      ),
      family: 'anthropic',
    ),
    _Field(
      'OPENAI_COMPAT_BASE_URL',
      tr('Адрес OpenAI-compatible API', 'OpenAI-compatible API address'),
      hint: tr(
        'Например http://localhost:11434/v1; /chat/completions добавляется автоматически',
        'For example http://localhost:11434/v1; /chat/completions is appended automatically',
      ),
      family: 'openai-compat',
    ),
  ]),
  _Group(tr('Приложение', 'App'), Icons.desktop_windows_outlined, [
    _Field(
      contextWarnKey,
      tr(
        'Предупреждать о контексте от, токенов',
        'Warn about context from, tokens',
      ),
      kind: _Kind.number,
      hint: tr(
        'По умолчанию 150000; 0 отключает. Значок в трее и уведомление',
        'Default 150000; 0 turns it off. Tray icon and a notification',
      ),
    ),
    _Field(
      'CLAUDE_BRIDGE_BIN',
      tr('Путь к claude-bridge', 'Path to claude-bridge'),
      hint: tr(
        'Пусто: рядом с приложением, ~/.cargo/bin или PATH',
        'Empty: next to the app, ~/.cargo/bin or PATH',
      ),
    ),
  ]),
];

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  final Map<String, TextEditingController> _controllers = {};
  final Map<String, String> _choices = {};
  final Set<String> _revealed = {};
  bool _loaded = false;

  @override
  void dispose() {
    for (final c in _controllers.values) {
      c.dispose();
    }
    super.dispose();
  }

  ///Заполняет форму сохраненными значениями
  void _reset() {
    final settings = context.appServices.settingsService.provider;
    for (final group in _groups) {
      for (final f in group.fields) {
        final value = settings.value(f.key);
        if (f.kind == _Kind.choice || f.kind == _Kind.flag) {
          _choices[f.key] = value;
        } else {
          (_controllers[f.key] ??= TextEditingController()).text = value;
        }
      }
    }
  }

  String _current(_Field f) =>
      _controllers[f.key]?.text.trim() ?? _choices[f.key] ?? '';

  Map<String, String> _changes() {
    final settings = context.appServices.settingsService.provider;
    return {
      for (final group in _groups)
        for (final f in group.fields)
          if (_current(f) != settings.value(f.key)) f.key: _current(f),
    };
  }

  @override
  Widget build(BuildContext context) {
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: settings,
      builder: (context, _) {
        //Форма заполняется, как только Rust прислал настройки
        if (!_loaded && settings.state != null) {
          _loaded = true;
          _reset();
        }
        final changes = _changes();
        return Column(
          children: [
            Expanded(
              child: ListView(
                padding: const EdgeInsets.all(24),
                children: [
                  for (final group in _groups)
                    if (group.family == null ||
                        settings.isEnabled(group.family!))
                      SectionCard(
                        title: group.title,
                        icon: group.icon,
                        child: Column(
                          children: [
                            for (final f in group.fields)
                              if (f.family == null ||
                                  settings.isEnabled(f.family!))
                                _buildField(f, settings.fromEnvironment(f.key)),
                          ],
                        ),
                      ),
                  if (settings.state != null)
                    Text(
                      tr(
                        'Файл: ${settings.state!.filePath}',
                        'File: ${settings.state!.filePath}',
                      ),
                      style: TextStyle(color: Theme.of(context).hintColor),
                    ),
                ],
              ),
            ),
            const Divider(height: 1),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      changes.isEmpty
                          ? tr('Изменений нет', 'No changes')
                          : tr(
                              'Изменено: ${changes.length}',
                              'Changed: ${changes.length}',
                            ),
                    ),
                  ),
                  TextButton(
                    onPressed: changes.isEmpty ? null : () => setState(_reset),
                    child: Text(tr(tr('Отменить', 'Revert'), 'Revert')),
                  ),
                  const SizedBox(width: 8),
                  FilledButton.icon(
                    onPressed: changes.isEmpty
                        ? null
                        : () =>
                              context.appServices.settingsService.save(changes),
                    icon: const Icon(Icons.save_outlined),
                    label: Text(tr(tr('Сохранить', 'Save'), 'Save')),
                  ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }

  Widget _buildField(_Field f, bool fromEnvironment) {
    final envHint = fromEnvironment
        ? tr(
            'Задано в переменных окружения; значение здесь его не заменит',
            'Set in the environment; a value here will not override it',
          )
        : null;
    final helper = [?f.hint, ?envHint].join('\n');
    final decoration = InputDecoration(
      labelText: f.label,
      helperText: helper.isEmpty ? null : helper,
      helperMaxLines: 3,
    );
    final Widget field;
    switch (f.kind) {
      case _Kind.flag:
        field = SwitchListTile(
          contentPadding: EdgeInsets.zero,
          title: Text(f.label),
          subtitle: helper.isEmpty ? null : Text(helper),
          value: _choices[f.key] == '1',
          onChanged: (v) => setState(() => _choices[f.key] = v ? '1' : ''),
        );
      case _Kind.choice:
        final value = f.options.contains(_choices[f.key])
            ? _choices[f.key]!
            : '';
        field = DropdownButtonFormField<String>(
          initialValue: value,
          isExpanded: true,
          decoration: decoration,
          items: [
            for (final o in f.options)
              DropdownMenuItem(
                value: o,
                child: Text(o.isEmpty ? tr('по умолчанию', 'default') : o),
              ),
          ],
          onChanged: (v) => setState(() => _choices[f.key] = v ?? ''),
        );
      case _Kind.secret:
        final revealed = _revealed.contains(f.key);
        field = TextField(
          controller: _controllers[f.key],
          obscureText: !revealed,
          decoration: decoration.copyWith(
            suffixIcon: IconButton(
              icon: Icon(revealed ? Icons.visibility_off : Icons.visibility),
              onPressed: () => setState(
                () => revealed ? _revealed.remove(f.key) : _revealed.add(f.key),
              ),
            ),
          ),
          onChanged: (_) => setState(() {}),
        );
      case _Kind.number:
      case _Kind.text:
        field = TextField(
          controller: _controllers[f.key],
          keyboardType: f.kind == _Kind.number
              ? TextInputType.number
              : TextInputType.text,
          decoration: decoration,
          onChanged: (_) => setState(() {}),
        );
    }
    return Padding(padding: const EdgeInsets.only(bottom: 12), child: field);
  }
}
