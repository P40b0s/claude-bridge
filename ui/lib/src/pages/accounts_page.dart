import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/controls/usage_bars.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Все провайдеры: вход Claude Code, входы через браузер (ChatGPT), API-ключи и лимиты подписок
class AccountsPage extends StatelessWidget {
  const AccountsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final accounts = context.appServices.accountsService.provider;
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([accounts, settings]),
      builder: (context, _) {
        final codexUsage = accounts.codexUsage;
        final on = settings.isEnabled;
        if (settings.disabledProviders.containsAll(const [
          'anthropic',
          'codex',
          'openai',
          'openai-compat',
        ])) {
          return Center(
            child: Text(
              tr(
                'Все провайдеры отключены. Включите нужные на вкладке «Провайдеры».',
                'All providers are disabled. Turn on the ones you need in the Providers tab.',
              ),
              style: TextStyle(color: Theme.of(context).hintColor),
            ),
          );
        }
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            if (on('anthropic'))
              _AccountGroup(
                title: 'Claude',
                icon: Icons.psychology_outlined,
                onAdd: () => _newAccount(context, AuthProvider.claude),
                children: [
                  for (final account in accounts.claudeAccounts)
                    _ClaudeCodeCard(
                      account: account,
                      usage: account.active ? accounts.claudeUsage : null,
                    ),
                  if (accounts.claudeAccounts.isEmpty)
                    _EmptyAccounts(
                      text: tr(
                        'Нет сохранённых аккаунтов Claude',
                        'No saved Claude accounts',
                      ),
                    ),
                ],
              ),
            if (on('codex'))
              _AccountGroup(
                title: 'ChatGPT / Codex',
                icon: Icons.auto_awesome,
                onAdd: () => _newAccount(context, AuthProvider.chatGpt),
                children: [
                  for (final account in accounts.chatgptAccounts)
                    _AccountCard(
                      title: account.id,
                      icon: Icons.account_circle_outlined,
                      provider: AuthProvider.chatGpt,
                      profile: account.id,
                      active: account.active,
                      signedIn: true,
                      details: [
                        tr(
                          '${account.email ?? 'неизвестный аккаунт'}, план ${account.plan ?? 'неизвестен'}',
                          '${account.email ?? 'unknown account'}, plan ${account.plan ?? 'unknown'}',
                        ),
                        if (account.subscriptionUntil != null)
                          formatSubscriptionEnd(account.subscriptionUntil!),
                        account.source == 'codex-cli'
                            ? tr(
                                'токены Codex CLI (~/.codex/auth.json)',
                                'Codex CLI tokens (~/.codex/auth.json)',
                              )
                            : tr(
                                'вход через claude-bridge',
                                'claude-bridge login',
                              ),
                        formatExpiry(account.expiresAt),
                      ],
                      hint: tr(
                        'Модели GPT-5.6 Sol, Terra, Luna и GPT-5.5 по подписке ChatGPT.',
                        'GPT-5.6 Sol, Terra, Luna and GPT-5.5 on a ChatGPT subscription.',
                      ),
                      canLogout: account.source != 'codex-cli',
                      usage: account.active ? _usage(codexUsage) : null,
                    ),
                  if (accounts.chatgptAccounts.isEmpty)
                    _EmptyAccounts(
                      text: tr(
                        'Нет сохранённых аккаунтов ChatGPT',
                        'No saved ChatGPT accounts',
                      ),
                    ),
                ],
              ),
            if (on('openai-compat')) const _OpenAiCompatCard(),
            if (on('openai'))
              _ApiKeyCard(
                title: 'OpenAI API',
                icon: Icons.key,
                envKey: 'OPENAI_API_KEY',
                hint: tr(
                  'Маршрут openai: модели OpenAI через Responses API с оплатой по ключу.',
                  'The openai route: OpenAI models through the Responses API, billed by key.',
                ),
                consoleUrl: 'platform.openai.com/api-keys',
              ),
            if (on('anthropic'))
              _ApiKeyCard(
                title: 'Anthropic API',
                icon: Icons.key,
                envKey: 'ANTHROPIC_API_KEY',
                hint: tr(
                  'Модели Claude по ключу. Нужен, только если Claude Code не вошел сам.',
                  "Claude models by key. Needed only if Claude Code isn't logged in itself.",
                ),
                consoleUrl: 'console.anthropic.com/settings/keys',
              ),
          ],
        );
      },
    );
  }

  ///Лимиты в карточке: полосы, причина их отсутствия или ничего, пока не загрузились
  static Widget? _usage(Usage? usage) {
    if (usage == null) {
      return null;
    }
    if (usage.error != null) {
      return _UsageNote(
        tr(
          'Лимиты не получены: ${usage.error}',
          'Limits not loaded: ${usage.error}',
        ),
      );
    }
    if (usage.resets.isEmpty) {
      return UsageBars(usage: usage);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        UsageBars(usage: usage),
        _ResetsList(usage.resets),
      ],
    );
  }
}

Future<void> _newAccount(BuildContext context, AuthProvider provider) async {
  final controller = TextEditingController();
  String? error;
  final profile = await showDialog<String>(
    context: context,
    builder: (dialogContext) => StatefulBuilder(
      builder: (context, setDialogState) => AlertDialog(
        title: Text(tr('Добавить аккаунт', 'Add account')),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: InputDecoration(
            labelText: tr('Имя профиля', 'Profile name'),
            hintText: 'work-account',
            errorText: error,
            helperText: tr(
              'Буквы, цифры, точка, дефис и подчёркивание',
              'Letters, digits, dot, dash and underscore',
            ),
          ),
          onSubmitted: (_) {
            final value = controller.text.trim();
            if (RegExp(r'^[A-Za-z0-9._-]{1,64}$').hasMatch(value) &&
                value != '.' &&
                value != '..') {
              Navigator.pop(dialogContext, value);
            } else {
              setDialogState(
                () => error = tr('Недопустимое имя', 'Invalid name'),
              );
            }
          },
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: Text(tr('Отмена', 'Cancel')),
          ),
          FilledButton(
            onPressed: () {
              final value = controller.text.trim();
              if (RegExp(r'^[A-Za-z0-9._-]{1,64}$').hasMatch(value) &&
                  value != '.' &&
                  value != '..') {
                Navigator.pop(dialogContext, value);
              } else {
                setDialogState(
                  () => error = tr('Недопустимое имя', 'Invalid name'),
                );
              }
            },
            child: Text(tr('Войти', 'Log in')),
          ),
        ],
      ),
    ),
  );
  controller.dispose();
  if (profile != null && context.mounted) {
    context.appServices.accountsService.login(provider, profile);
  }
}

class _AccountGroup extends StatelessWidget {
  const _AccountGroup({
    required this.title,
    required this.icon,
    required this.onAdd,
    required this.children,
  });
  final String title;
  final IconData icon;
  final VoidCallback onAdd;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => SectionCard(
    title: title,
    icon: icon,
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ...children,
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: FilledButton.icon(
            onPressed: onAdd,
            icon: const Icon(Icons.person_add_alt_1),
            label: Text(tr('Добавить аккаунт', 'Add account')),
          ),
        ),
      ],
    ),
  );
}

class _EmptyAccounts extends StatelessWidget {
  const _EmptyAccounts({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 12),
    child: Text(text, style: TextStyle(color: Theme.of(context).hintColor)),
  );
}

///Бесплатные сбросы лимитов Codex: каждый можно потратить кнопкой
class _ResetsList extends StatefulWidget {
  const _ResetsList(this.resets);
  final List<UsageReset> resets;

  @override
  State<_ResetsList> createState() => _ResetsListState();
}

class _ResetsListState extends State<_ResetsList> {
  ///Сброс, ответа на который ещё ждём
  String? _using;

  @override
  void didUpdateWidget(_ResetsList old) {
    super.didUpdateWidget(old);
    //Список обновился после сброса
    if (_using != null && !widget.resets.any((r) => r.id == _using)) {
      _using = null;
    }
  }

  Future<void> _use(UsageReset reset) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(tr('Использовать сброс?', 'Use a reset?')),
        content: Text(
          tr(
            '${reset.title}: 5-часовой и недельный лимиты Codex обнулятся. Сброс одноразовый. '
                'Если лимиты сейчас не израсходованы, OpenAI его не спишет.',
            '${reset.title}: the Codex 5-hour and weekly limits start over. A reset is single-use. '
                'If the limits are not used up right now, OpenAI does not spend it.',
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(tr('Отмена', 'Cancel')),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(tr('Сбросить', 'Reset')),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) {
      return;
    }
    setState(() => _using = reset.id);
    context.appServices.accountsService.useReset(reset.id);
    //Если ответ не изменил список (нечего сбрасывать), кнопка снова доступна
    Future.delayed(const Duration(seconds: 20), () {
      if (mounted && _using == reset.id) {
        setState(() => _using = null);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            tr(
              'Бесплатные сбросы лимитов: ${widget.resets.length}',
              'Free limit resets: ${widget.resets.length}',
            ),
            style: theme.textTheme.titleSmall,
          ),
          for (final r in widget.resets)
            ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: const Icon(Icons.restart_alt),
              title: Text(r.title),
              subtitle: r.expiresAt == null
                  ? null
                  : Text(
                      tr(
                        'действует до ${formatDateTime(r.expiresAt!)}',
                        'expires ${formatDateTime(r.expiresAt!)}',
                      ),
                    ),
              trailing: _using == r.id
                  ? const SizedBox(
                      width: 20,
                      height: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : OutlinedButton(
                      onPressed: _using != null ? null : () => _use(r),
                      child: Text(tr('Использовать', 'Use')),
                    ),
            ),
        ],
      ),
    );
  }
}

class _AccountCard extends StatelessWidget {
  const _AccountCard({
    required this.title,
    required this.icon,
    required this.provider,
    required this.profile,
    required this.active,
    required this.signedIn,
    required this.details,
    required this.hint,
    this.canLogout = true,
    this.usage,
  });
  final String title;
  final IconData icon;
  final AuthProvider provider;
  final String profile;
  final bool active;
  final bool signedIn;
  final List<String> details;
  final String hint;
  final bool canLogout;
  final Widget? usage;

  @override
  Widget build(BuildContext context) {
    final service = context.appServices.accountsService;
    final waiting = service.provider.isWaiting(provider, profile);
    final login = service.provider.login;
    final theme = Theme.of(context);
    return _StatusCard(
      title: title,
      icon: icon,
      ok: signedIn,
      status: active
          ? tr('Активный аккаунт', 'Active account')
          : tr('Сохранён', 'Saved'),
      details: details,
      hint: hint,
      trailing: waiting
          ? OutlinedButton(
              onPressed: service.cancelLogin,
              child: Text(tr(tr('Отменить', 'Cancel'), 'Cancel')),
            )
          : Wrap(
              spacing: 8,
              children: [
                if (!active)
                  OutlinedButton(
                    onPressed: service.provider.anyWaiting
                        ? null
                        : () => service.activate(provider, profile),
                    child: Text(tr('Активировать', 'Activate')),
                  ),
                if (signedIn && canLogout)
                  OutlinedButton(
                    onPressed: () => service.logout(provider, profile),
                    child: Text(tr(tr('Выйти', 'Log out'), 'Log out')),
                  ),
                FilledButton(
                  onPressed: service.provider.anyWaiting
                      ? null
                      : () => service.login(provider, profile),
                  child: Text(
                    signedIn
                        ? tr('Войти заново', 'Log in again')
                        : tr('Войти', 'Log in'),
                  ),
                ),
              ],
            ),
      footer: [
        ?usage,
        if (waiting) ...[
          const SizedBox(height: 12),
          const LinearProgressIndicator(),
          const SizedBox(height: 8),
          Text(
            tr(
              tr(
                'Завершите вход в открывшемся браузере. Если он не открылся, откройте ссылку:',
                "Finish logging in in the browser that opened. If it didn't open, use this link:",
              ),
              "Finish logging in in the browser that opened. If it didn't open, use this link:",
            ),
          ),
          if (login?.url != null)
            Row(
              children: [
                Expanded(
                  child: SelectableText(
                    login!.url!,
                    maxLines: 2,
                    style: theme.textTheme.bodySmall,
                  ),
                ),
                IconButton(
                  tooltip: tr('Скопировать', 'Copy'),
                  icon: const Icon(Icons.copy),
                  onPressed: () =>
                      Clipboard.setData(ClipboardData(text: login.url!)),
                ),
              ],
            ),
        ],
      ],
    );
  }
}

class _OpenAiCompatCard extends StatelessWidget {
  const _OpenAiCompatCard();

  @override
  Widget build(BuildContext context) {
    final settings = context.appServices.settingsService;
    final provider = settings.provider;
    final url = provider.value('OPENAI_COMPAT_BASE_URL');
    final key = provider.value('OPENAI_COMPAT_API_KEY');
    final urlFromEnvironment = provider.fromEnvironment(
      'OPENAI_COMPAT_BASE_URL',
    );
    final keyFromEnvironment = provider.fromEnvironment(
      'OPENAI_COMPAT_API_KEY',
    );
    final connected = url.isNotEmpty || urlFromEnvironment;
    return _StatusCard(
      title: 'OpenAI-compatible',
      icon: Icons.hub_outlined,
      ok: connected,
      status: connected
          ? tr('Адрес задан', 'URL set')
          : tr('Адрес не задан', 'No URL'),
      details: [
        if (url.isNotEmpty) 'OPENAI_COMPAT_BASE_URL = $url',
        if (urlFromEnvironment)
          tr(
            'OPENAI_COMPAT_BASE_URL задан в переменных окружения',
            'OPENAI_COMPAT_BASE_URL is set in the environment',
          ),
        if (key.isNotEmpty) 'OPENAI_COMPAT_API_KEY = ${_ApiKeyCard._mask(key)}',
        if (keyFromEnvironment)
          tr(
            'OPENAI_COMPAT_API_KEY задан в переменных окружения',
            'OPENAI_COMPAT_API_KEY is set in the environment',
          ),
      ],
      hint: tr(
        'Маршрут openai-compat:<model> для OpenRouter, Ollama, vLLM, LM Studio, DeepSeek и других Chat Completions API. Ключ необязателен.',
        'The openai-compat:<model> route for OpenRouter, Ollama, vLLM, LM Studio, DeepSeek and other Chat Completions APIs. The key is optional.',
      ),
      trailing: FilledButton(
        onPressed: () => _edit(context, url, key.isNotEmpty),
        child: Text(
          connected ? tr('Изменить', 'Edit') : tr('Настроить', 'Configure'),
        ),
      ),
    );
  }

  Future<void> _edit(
    BuildContext context,
    String savedUrl,
    bool hasSavedKey,
  ) async {
    final url = TextEditingController(text: savedUrl);
    final key = TextEditingController();
    final values = await showDialog<Map<String, String>>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('OpenAI-compatible'),
        content: SizedBox(
          width: 520,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: url,
                autofocus: true,
                decoration: const InputDecoration(
                  labelText: 'OPENAI_COMPAT_BASE_URL',
                  hintText: 'http://localhost:11434/v1',
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: key,
                obscureText: true,
                decoration: InputDecoration(
                  labelText: 'OPENAI_COMPAT_API_KEY',
                  helperText: hasSavedKey
                      ? tr(
                          'Не заполняйте, чтобы сохранить текущий ключ; «Удалить ключ» удалит его.',
                          'Leave blank to keep the current key; “Remove key” deletes it.',
                        )
                      : tr(
                          'Необязательно для локальных серверов.',
                          'Optional for local servers.',
                        ),
                  helperMaxLines: 2,
                ),
              ),
            ],
          ),
        ),
        actions: [
          if (hasSavedKey)
            TextButton(
              onPressed: () => Navigator.pop(context, {
                'OPENAI_COMPAT_BASE_URL': url.text.trim(),
                'OPENAI_COMPAT_API_KEY': '',
              }),
              child: Text(tr('Удалить ключ', 'Remove key')),
            ),
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(tr('Отмена', 'Cancel')),
          ),
          FilledButton(
            onPressed: () {
              final result = {'OPENAI_COMPAT_BASE_URL': url.text.trim()};
              if (key.text.trim().isNotEmpty) {
                result['OPENAI_COMPAT_API_KEY'] = key.text.trim();
              }
              Navigator.pop(context, result);
            },
            child: Text(tr('Сохранить', 'Save')),
          ),
        ],
      ),
    );
    url.dispose();
    key.dispose();
    if (values != null && context.mounted) {
      context.appServices.settingsService.save(values);
    }
  }
}

///Провайдер с API-ключом: тот же вид, что у входов через браузер. Ключ хранится в .env
class _ApiKeyCard extends StatelessWidget {
  const _ApiKeyCard({
    required this.title,
    required this.icon,
    required this.envKey,
    required this.hint,
    required this.consoleUrl,
  });
  final String title;
  final IconData icon;
  final String envKey;
  final String hint;

  ///Где получить ключ, без схемы
  final String consoleUrl;

  @override
  Widget build(BuildContext context) {
    final settings = context.appServices.settingsService;
    final saved = settings.provider.value(envKey);
    final fromEnvironment = settings.provider.fromEnvironment(envKey);
    final hasKey = saved.isNotEmpty || fromEnvironment;
    return _StatusCard(
      title: title,
      icon: icon,
      ok: hasKey,
      status: hasKey
          ? tr('Ключ задан', 'Key set')
          : tr('Ключ не задан', 'No key'),
      details: [
        if (saved.isNotEmpty) '$envKey = ${_mask(saved)}',
        if (fromEnvironment)
          tr(
            '$envKey задан в переменных окружения',
            '$envKey is set in the environment',
          ),
        if (!hasKey)
          tr('Ключ можно получить на $consoleUrl', 'Get a key at $consoleUrl'),
      ],
      hint: hint,
      trailing: Wrap(
        spacing: 8,
        children: [
          if (saved.isNotEmpty)
            OutlinedButton(
              onPressed: () => settings.save({envKey: ''}),
              child: Text(tr(tr('Удалить', 'Delete'), 'Delete')),
            ),
          FilledButton(
            onPressed: () => _editKey(context),
            child: Text(
              saved.isNotEmpty
                  ? tr('Заменить', 'Replace')
                  : tr('Задать ключ', 'Set key'),
            ),
          ),
        ],
      ),
    );
  }

  static String _mask(String key) => key.length <= 8
      ? '••••'
      : '${key.substring(0, 3)}…${key.substring(key.length - 4)}';

  Future<void> _editKey(BuildContext context) async {
    final settings = context.appServices.settingsService;
    final controller = TextEditingController();
    final key = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: SizedBox(
          width: 480,
          child: TextField(
            controller: controller,
            autofocus: true,
            obscureText: true,
            decoration: InputDecoration(
              labelText: envKey,
              helperText: tr(
                'Сохраняется в ~/.claude-bridge/.env',
                'Saved to ~/.claude-bridge/.env',
              ),
            ),
            onSubmitted: (v) => Navigator.pop(context, v),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(tr(tr('Отмена', 'Cancel'), 'Cancel')),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: Text(tr(tr('Сохранить', 'Save'), 'Save')),
          ),
        ],
      ),
    );
    controller.dispose();
    if (key != null && key.trim().isNotEmpty) {
      settings.save({envKey: key.trim()});
    }
  }
}

///Общий вид карточки провайдера: статус, подробности, подсказка
class _StatusCard extends StatelessWidget {
  const _StatusCard({
    required this.title,
    required this.icon,
    required this.ok,
    required this.status,
    required this.details,
    required this.hint,
    this.trailing,
    this.footer = const [],
  });
  final String title;
  final IconData icon;
  final bool ok;
  final String status;
  final List<String> details;
  final String hint;
  final Widget? trailing;
  final List<Widget> footer;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SectionCard(
      title: title,
      icon: icon,
      trailing: trailing,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(
                ok ? Icons.check_circle : Icons.radio_button_unchecked,
                size: 18,
                color: ok ? Colors.green : theme.hintColor,
              ),
              const SizedBox(width: 8),
              Text(status),
            ],
          ),
          for (final line in details)
            Padding(
              padding: const EdgeInsets.only(left: 26, top: 2),
              child: SelectableText(line),
            ),
          const SizedBox(height: 8),
          Text(hint, style: TextStyle(color: theme.hintColor)),
          ...footer,
        ],
      ),
    );
  }
}

///Claude по подписке: Claude Code входит сам, прокси только пересылает его запросы
class _ClaudeCodeCard extends StatelessWidget {
  const _ClaudeCodeCard({required this.account, required this.usage});
  final ClaudeCodeAccount account;
  final Usage? usage;

  @override
  Widget build(BuildContext context) {
    final service = context.appServices.accountsService;
    final waiting = service.provider.isWaiting(AuthProvider.claude, account.id);
    return _StatusCard(
      title: account.id,
      icon: Icons.account_circle_outlined,
      ok: true,
      status: account.active
          ? tr('Активный аккаунт', 'Active account')
          : tr('Сохранён', 'Saved'),
      details: [
        account.email ?? tr('неизвестный аккаунт', 'unknown account'),
        _plan(account),
      ],
      hint: tr(
        'История проектов, настройки и MCP общие для всех аккаунтов; переключаются только OAuth-данные. '
            'Перед активацией другого аккаунта закройте запущенные сессии Claude Code.',
        'Project history, settings and MCP are shared by every account; only OAuth data is switched. '
            'Close running Claude Code sessions before activating another account.',
      ),
      trailing: waiting
          ? OutlinedButton(
              onPressed: service.cancelLogin,
              child: Text(tr('Отменить', 'Cancel')),
            )
          : Wrap(
              spacing: 8,
              children: [
                if (!account.active)
                  OutlinedButton(
                    onPressed: service.provider.anyWaiting
                        ? null
                        : () =>
                              service.activate(AuthProvider.claude, account.id),
                    child: Text(tr('Активировать', 'Activate')),
                  ),
                OutlinedButton(
                  onPressed: () =>
                      service.logout(AuthProvider.claude, account.id),
                  child: Text(tr('Выйти', 'Log out')),
                ),
                FilledButton(
                  onPressed: service.provider.anyWaiting
                      ? null
                      : () => service.login(AuthProvider.claude, account.id),
                  child: Text(tr('Войти заново', 'Log in again')),
                ),
              ],
            ),
      footer: [
        if (usage != null)
          UsageBars(
            usage: usage!,
            source: tr(
              'по последнему ответу Claude через прокси',
              "from Claude's last reply through the proxy",
            ),
          )
        else if (account.active)
          _UsageNote(
            tr(
              'Лимиты появятся после первого запроса к Claude через прокси.',
              'Limits appear after the first Claude request through the proxy.',
            ),
          ),
        if (account.active)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Tooltip(
              message: tr(
                'Бесплатный сброс лимитов, если Anthropic его выдал, есть только на сайте: Settings → Usage → Reset for free. '
                    'Страница откроется в выбранном браузере через выход Anthropic.',
                'A free limit reset, when Anthropic grants one, is only on the website: Settings → Usage → Reset for free. '
                    'The page opens in the selected browser through the Anthropic exit.',
              ),
              child: TextButton.icon(
                onPressed: () => OpenClaudeUsage().sendSignalToRust(),
                icon: const Icon(Icons.open_in_new, size: 18),
                label: Text(
                  tr(
                    'Лимиты и сбросы на claude.ai',
                    'Limits and resets on claude.ai',
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

///«Подписка Pro с 06.02.2026, оплата через App Store»; когда она закончится, Claude Code не хранит
String _plan(ClaudeCodeAccount a) {
  if (a.plan == null) {
    return a.organization ??
        tr(
          'без подписки (ключ Anthropic Console)',
          'no subscription (Anthropic Console key)',
        );
  }
  final since = a.subscriptionSince == null
      ? ''
      : '${tr(' с ', ' since ')}${formatDate(a.subscriptionSince!)}';
  final billing = switch (a.billing) {
    null => '',
    'card' => tr(', оплата картой', ', paid by card'),
    final b => tr(', оплата через $b', ', paid via $b'),
  };
  return '${tr('Подписка', 'Plan')} ${a.plan}$since$billing';
}

class _UsageNote extends StatelessWidget {
  const _UsageNote(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Text(text, style: TextStyle(color: Theme.of(context).hintColor)),
    );
  }
}
