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
      'CODEX_REASONING_EFFORT',
      tr('Усилие по умолчанию', 'Default effort'),
      kind: _Kind.choice,
      options: ['', 'low', 'medium', 'high', 'xhigh', 'max'],
      hint: tr(
        'Когда в модели не указан @effort; по умолчанию high',
        'When the model has no @effort; high by default',
      ),
    ),
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
      hint: tr('По умолчанию 95% окна', 'Default 95% of the window'),
    ),
    _Field(
      'CODEX_DISABLE_COMPACT',
      tr('Отключить автосжатие', 'Disable auto-compact'),
      kind: _Kind.flag,
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
      'ANTHROPIC_UPSTREAM_URL',
      tr('Адрес Anthropic API', 'Anthropic API address'),
      hint: tr(
        'По умолчанию https://api.anthropic.com',
        'Default https://api.anthropic.com',
      ),
      family: 'anthropic',
    ),
    _Field(
      'GEMINI_BASE_URL',
      tr('Адрес Gemini API', 'Gemini API address'),
      hint: tr(
        'По умолчанию https://generativelanguage.googleapis.com/v1beta',
        'Default https://generativelanguage.googleapis.com/v1beta',
      ),
      family: 'gemini',
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
