import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/providers/requests_provider.dart';
import 'package:claude_bridge_ui/src/providers/settings_provider.dart';
import 'package:flutter/foundation.dart';

const contextWarnKey = 'CLAUDE_BRIDGE_CONTEXT_WARN_TOKENS';
const _defaultThreshold = 150000;

///Следит за размером промпта последнего запроса: каждый запрос отправляет весь разговор
///заново, поэтому длинная сессия быстро съедает лимиты подписки
class ContextWatch extends ChangeNotifier {
  ContextWatch(this._requests, this._settings) {
    _requests.addListener(_check);
    _settings.addListener(_check);
  }

  final RequestsProvider _requests;
  final SettingsProvider _settings;

  ///Контекст последнего завершённого запроса основного разговора, токенов
  int? context;
  bool warning = false;

  ///0 отключает предупреждение
  int get threshold =>
      int.tryParse(_settings.value(contextWarnKey).trim()) ?? _defaultThreshold;

  String get message => tr(
    'Контекст ${formatTokens(context ?? 0)} токенов на запрос (порог ${formatTokens(threshold)}): начните новую сессию или /compact',
    'Context is ${formatTokens(context ?? 0)} tokens per request (limit ${formatTokens(threshold)}): start a new session or /compact',
  );

  void _check() {
    //Служебные вызовы Claude Code (заголовки, субагенты) идут с голым claude-* и малым контекстом
    final latest = _requests.entries
        .where(
          (e) =>
              e.phase == RequestPhase.ok &&
              e.inputTokens != null &&
              !e.requested.toLowerCase().startsWith('claude-'),
        )
        .firstOrNull;
    final size = latest == null
        ? null
        : (latest.inputTokens ?? 0) +
              (latest.cacheReadTokens ?? 0) +
              (latest.cacheWriteTokens ?? 0);
    final over = size != null && threshold > 0 && size >= threshold;
    if (size == context && over == warning) {
      return;
    }
    final crossed = over && !warning;
    context = size;
    warning = over;
    if (crossed) {
      ShowNotification(
        title: tr('Длинная сессия', 'Long session'),
        body: message,
      ).sendSignalToRust();
    }
    notifyListeners();
  }

  @override
  void dispose() {
    _requests.removeListener(_check);
    _settings.removeListener(_check);
    super.dispose();
  }
}
