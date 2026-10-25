import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/controls/usage_bars.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Все провайдеры: вход Claude Code, входы через браузер (ChatGPT, два аккаунта Google), API-ключи и лимиты подписок
class AccountsPage extends StatelessWidget {
  const AccountsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final accounts = context.appServices.accountsService.provider;
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([accounts, settings]),
      builder: (context, _) {
        final chatgpt = accounts.chatgpt;
        final google1 = accounts.google(1);
        final google2 = accounts.google(2);
        final codexUsage = accounts.codexUsage;
        final on = settings.isEnabled;
        if (settings.disabledProviders.containsAll(const [
          'anthropic',
          'codex',
          'openai',
          'openai-compat',
          'gemini',
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
              _ClaudeCodeCard(
                account: accounts.claudeCode,
                usage: accounts.claudeUsage,
              ),
            if (on('codex'))
              _AccountCard(
                title: 'ChatGPT / Codex',
                icon: Icons.auto_awesome,
                provider: AuthProvider.chatGpt,
                account: 1,
                signedIn: chatgpt != null,
                details: [
                  if (chatgpt != null) ...[
                    tr(
                      '${chatgpt.email ?? 'неизвестный аккаунт'}, план ${chatgpt.plan ?? 'неизвестен'}',
                      '${chatgpt.email ?? 'unknown account'}, plan ${chatgpt.plan ?? 'unknown'}',
                    ),
                    if (chatgpt.subscriptionUntil != null)
                      formatSubscriptionEnd(chatgpt.subscriptionUntil!),
                    chatgpt.source == 'codex-cli'
                        ? tr(
                            'токены Codex CLI (~/.codex/auth.json)',
                            'Codex CLI tokens (~/.codex/auth.json)',
                          )
                        : tr('вход через claude-bridge', 'claude-bridge login'),
                    formatExpiry(chatgpt.expiresAt),
                  ],
                ],
                hint: tr(
                  'Модели GPT-5.6 Sol, Terra, Luna и GPT-5.5 по подписке ChatGPT.',
                  'GPT-5.6 Sol, Terra, Luna and GPT-5.5 on a ChatGPT subscription.',
                ),
                canLogout: chatgpt?.source != 'codex-cli',
                usage: chatgpt == null ? null : _usage(codexUsage),
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
            if (on('gemini')) ...[
              _AccountCard(
                title: 'Google (Gemini)',
                icon: Icons.diamond_outlined,
                provider: AuthProvider.google,
                account: 1,
                signedIn: google1 != null,
                details: _googleDetails(google1),
                hint: tr(
                  'Gemini через Code Assist или стандартный Gemini API.',
                  'Gemini through Code Assist or the standard Gemini API.',
                ),
                usage: google1 == null
                    ? null
                    : _usage(
                        accounts.geminiUsage(1),
                        standardApi: google1.projectId == null,
                      ),
              ),
              _AccountCard(
                title: tr('Google, запасной аккаунт', 'Google, backup account'),
                icon: Icons.diamond_outlined,
                provider: AuthProvider.google,
                account: 2,
                signedIn: google2 != null,
                details: _googleDetails(google2),
                hint: tr(
                  'Используется автоматически, когда первый аккаунт упирается в лимит (429).',
                  'Used automatically when the first account hits its limit (429).',
                ),
                usage: google2 == null
                    ? null
                    : _usage(
                        accounts.geminiUsage(2),
                        standardApi: google2.projectId == null,
                      ),
              ),
              _ApiKeyCard(
                title: 'Gemini API',
                icon: Icons.key,
                envKey: 'GEMINI_API_KEY',
                hint: tr(
                  'Маршрут gemini без входа через Google.',
                  'The gemini route, without a Google login.',
                ),
                consoleUrl: 'aistudio.google.com/apikey',
              ),
            ],
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
  static Widget? _usage(Usage? usage, {bool standardApi = false}) {
    if (usage == null) {
      return null;
    }
    if (usage.error != null) {
      return _UsageNote(
        standardApi
            ? tr(
                'Лимиты: стандартный Gemini API их не сообщает, они есть только у Code Assist.',
                "Limits: the standard Gemini API doesn't report them, only Code Assist does.",
              )
            : tr(
                'Лимиты не получены: ${usage.error}',
                'Limits not loaded: ${usage.error}',
              ),
      );
    }
    return UsageBars(usage: usage);
  }

  List<String> _googleDetails(GoogleAccount? account) {
    if (account == null) {
      return const [];
    }
    return [
      account.email ?? tr('неизвестный аккаунт', 'unknown account'),
      account.projectId == null
          ? tr('стандартный Gemini API', 'standard Gemini API')
          : tr(
              'Code Assist, проект ${account.projectId}',
              'Code Assist, project ${account.projectId}',
            ),
      formatExpiry(account.expiresAt),
    ];
  }
}

class _AccountCard extends StatelessWidget {
  const _AccountCard({
    required this.title,
    required this.icon,
    required this.provider,
    required this.account,
    required this.signedIn,
    required this.details,
    required this.hint,
    this.canLogout = true,
    this.usage,
  });
  final String title;
  final IconData icon;
  final AuthProvider provider;
  final int account;
  final bool signedIn;
  final List<String> details;
  final String hint;
  final bool canLogout;
  final Widget? usage;

  @override
  Widget build(BuildContext context) {
    final service = context.appServices.accountsService;
    final waiting = service.provider.isWaiting(provider, account);
    final login = service.provider.login;
    final theme = Theme.of(context);
    return _StatusCard(
      title: title,
      icon: icon,
      ok: signedIn,
      status: signedIn
          ? tr('Вход выполнен', 'Logged in')
          : tr('Вход не выполнен', 'Not logged in'),
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
                if (signedIn && canLogout)
                  OutlinedButton(
                    onPressed: () => service.logout(provider, account: account),
                    child: Text(tr(tr('Выйти', 'Log out'), 'Log out')),
                  ),
                FilledButton(
                  onPressed: service.provider.anyWaiting
                      ? null
                      : () => service.login(provider, account: account),
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
  final ClaudeCodeAccount? account;
  final Usage? usage;

  @override
  Widget build(BuildContext context) {
    final signedIn = account != null;
    return _StatusCard(
      title: 'Claude',
      icon: Icons.psychology_outlined,
      ok: signedIn,
      status: signedIn
          ? tr('Вход Claude Code', 'Claude Code login')
          : tr('Claude Code не вошел', "Claude Code isn't logged in"),
      details: [
        if (signedIn) ...[
          account!.email ?? tr('неизвестный аккаунт', 'unknown account'),
          _plan(account!),
        ],
        if (!signedIn)
          tr(
            'Запустите claude и выполните /login, или задайте ключ Anthropic API ниже',
            'Run claude and do /login, or set an Anthropic API key below',
          ),
      ],
      hint: tr(
        'Claude Code входит сам; прокси пересылает его запросы к Claude как есть и не хранит токены. '
            'В сессии /model opus (или anthropic:sonnet, anthropic:haiku) — Claude, остальные модели — через прокси. '
            'Пока в сессии выбрана не Claude, в Anthropic из нее не уходит ни одного запроса.',
        'Claude Code logs in itself; the proxy relays its Claude requests as is and stores no tokens. '
            'In a session, /model opus (or anthropic:sonnet, anthropic:haiku) is Claude, other models go through the proxy. '
            'While a non-Claude model is selected, the session sends nothing to Anthropic.',
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
        else if (signedIn)
          _UsageNote(
            tr(
              'Лимиты появятся после первого запроса к Claude через прокси.',
              'Limits appear after the first Claude request through the proxy.',
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
