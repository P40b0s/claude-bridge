import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/providers/accounts_provider.dart';
import 'package:claude_bridge_ui/src/providers/settings_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

class _Family {
  const _Family(
    this.id,
    this.title,
    this.icon,
    this.description,
    this.routes,
    this.status,
  );

  ///Как Provider::family в Rust
  final String id;
  final String title;
  final IconData icon;
  final String description;

  ///Маршруты /model, которые относятся к группе
  final String routes;

  ///Чем провайдер сейчас подключён; null - не подключён
  final String? Function(AccountsProvider accounts, SettingsProvider settings)
  status;
}

List<_Family> get _families => [
  _Family(
    'anthropic',
    'Anthropic (Claude)',
    Icons.psychology_outlined,
    tr(
      'Модели Claude: через вход Claude Code или по ANTHROPIC_API_KEY.',
      "Claude models: through Claude Code's login or ANTHROPIC_API_KEY.",
    ),
    'opus, sonnet, haiku, anthropic:…',
    (a, s) => a.claudeCode != null
        ? tr('вход Claude Code', 'Claude Code login')
        : (_hasKey(s, 'ANTHROPIC_API_KEY') ? tr('API-ключ', 'API key') : null),
  ),
  _Family(
    'codex',
    'ChatGPT / Codex',
    Icons.auto_awesome,
    tr(
      'GPT-5.6 Sol, Terra, Luna и GPT-5.5 по подписке ChatGPT.',
      'GPT-5.6 Sol, Terra, Luna and GPT-5.5 on a ChatGPT subscription.',
    ),
    'sol, terra, luna, gpt55, fast…max, codex-oauth:…',
    (a, _) => a.chatgpt == null ? null : tr('вход ChatGPT', 'ChatGPT login'),
  ),
  _Family(
    'openai',
    'OpenAI API',
    Icons.key,
    tr(
      'Модели OpenAI через Responses API с оплатой по ключу.',
      'OpenAI models through the Responses API, billed by key.',
    ),
    'openai:…',
    (_, s) => _hasKey(s, 'OPENAI_API_KEY') ? tr('API-ключ', 'API key') : null,
  ),
  _Family(
    'openai-compat',
    'OpenAI-compatible',
    Icons.hub_outlined,
    tr(
      'Любой сервер с Chat Completions API: OpenRouter, Ollama, vLLM, LM Studio, DeepSeek и другие.',
      'Any server with a Chat Completions API: OpenRouter, Ollama, vLLM, LM Studio, DeepSeek and others.',
    ),
    'openai-compat:…',
    (_, s) =>
        s.value('OPENAI_COMPAT_BASE_URL').isNotEmpty ||
            s.fromEnvironment('OPENAI_COMPAT_BASE_URL')
        ? tr('адрес задан', 'URL set')
        : null,
  ),
  _Family(
    'gemini',
    'Google (Gemini)',
    Icons.diamond_outlined,
    tr(
      'Gemini через вход в Google (Code Assist) или по GEMINI_API_KEY.',
      'Gemini through a Google login (Code Assist) or GEMINI_API_KEY.',
    ),
    'gemini, gemini-flash, gemini-oauth:…, gemini:…',
    (a, s) => a.google(1) != null
        ? tr('вход Google', 'Google login')
        : (_hasKey(s, 'GEMINI_API_KEY') ? tr('API-ключ', 'API key') : null),
  ),
];

bool _hasKey(SettingsProvider s, String key) =>
    s.value(key).isNotEmpty || s.fromEnvironment(key);

///Какие провайдеры используются: отключённые не видны в UI, и прокси не принимает запросы к ним
class ProvidersPage extends StatelessWidget {
  const ProvidersPage({super.key});

  @override
  Widget build(BuildContext context) {
    final settingsService = context.appServices.settingsService;
    final settings = settingsService.provider;
    final accounts = context.appServices.accountsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([settings, accounts]),
      builder: (context, _) {
        final theme = Theme.of(context);
        if (settings.state == null) {
          return const Padding(
            padding: EdgeInsets.all(24),
            child: LinearProgressIndicator(),
          );
        }
        final enabledCount = _families
            .where((f) => settings.isEnabled(f.id))
            .length;
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            SectionCard(
              title: tr('Провайдеры', 'Providers'),
              icon: Icons.extension_outlined,
              trailing: Text(
                tr(
                  'включено $enabledCount из ${_families.length}',
                  '$enabledCount of ${_families.length} on',
                ),
                style: TextStyle(color: theme.hintColor),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    tr(
                      'Отключённый провайдер пропадает из аккаунтов, лимитов, списка моделей, настроек и журнала запросов, '
                          'а прокси отвечает ошибкой на запросы к нему. Вход и ключи при этом сохраняются. Действует сразу.',
                      'A disabled provider disappears from accounts, limits, the model list, settings and the request log, '
                          'and the proxy answers requests to it with an error. Logins and keys are kept. Applies immediately.',
                    ),
                    style: TextStyle(color: theme.hintColor),
                  ),
                  const SizedBox(height: 8),
                  for (final f in _families)
                    _FamilyTile(
                      family: f,
                      enabled: settings.isEnabled(f.id),
                      status: f.status(accounts, settings),
                      onChanged: (v) =>
                          settingsService.setProviderEnabled(f.id, v),
                    ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

class _FamilyTile extends StatelessWidget {
  const _FamilyTile({
    required this.family,
    required this.enabled,
    required this.status,
    required this.onChanged,
  });
  final _Family family;
  final bool enabled;
  final String? status;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final connected = status != null;
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: AnimatedOpacity(
        duration: const Duration(milliseconds: 150),
        opacity: enabled ? 1 : 0.6,
        child: SwitchListTile(
          contentPadding: const EdgeInsets.symmetric(
            horizontal: 12,
            vertical: 4,
          ),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(12),
            side: BorderSide(
              color: enabled
                  ? scheme.primary.withValues(alpha: 0.4)
                  : scheme.outlineVariant.withValues(alpha: 0.6),
            ),
          ),
          value: enabled,
          onChanged: onChanged,
          secondary: Container(
            width: 36,
            height: 36,
            decoration: BoxDecoration(
              color: (enabled ? scheme.primary : scheme.outline).withValues(
                alpha: 0.12,
              ),
              shape: BoxShape.circle,
            ),
            child: Icon(
              family.icon,
              size: 20,
              color: enabled ? scheme.primary : scheme.outline,
            ),
          ),
          title: Row(
            children: [
              Flexible(
                child: Text(
                  family.title,
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
              ),
              const SizedBox(width: 8),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
                decoration: BoxDecoration(
                  color: (connected ? Colors.green : scheme.outline).withValues(
                    alpha: 0.14,
                  ),
                  borderRadius: BorderRadius.circular(10),
                ),
                child: Text(
                  status ?? tr('не подключён', 'not connected'),
                  style: theme.textTheme.labelSmall?.copyWith(
                    color: connected
                        ? Colors.green.shade700
                        : scheme.onSurfaceVariant,
                  ),
                ),
              ),
            ],
          ),
          subtitle: Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text('${family.description}\n/model: ${family.routes}'),
          ),
          isThreeLine: true,
        ),
      ),
    );
  }
}
