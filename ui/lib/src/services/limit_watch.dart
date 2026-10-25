import 'dart:async';

import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/providers/accounts_provider.dart';
import 'package:claude_bridge_ui/src/providers/requests_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:intl/intl.dart';

///С какого расхода пятичасового окна предупреждать в любом случае, процентов
const _warnPercent = 80.0;

///За какой отрезок считать темп расхода
const _paceSpan = Duration(minutes: 20);

///Меньше этого отрезка темп слишком шумный
const _minPaceSpan = Duration(minutes: 3);

///Следит за лимитами подписок: исчерпанный лимит (прокси до сброса отвечает сам, не
///отправляя запрос наверх) и прогноз пятичасового окна Claude и Codex по темпу расхода
class LimitWatch extends ChangeNotifier {
  LimitWatch(this._requests, this._accounts) {
    _requests.addListener(_checkLimited);
    _accounts.addListener(_checkForecast);
  }

  final RequestsProvider _requests;
  final AccountsProvider _accounts;

  ///Провайдер маршрута (codex-oauth…) -> когда сбросится его лимит, Unix ms
  final Map<String, int> _limitedUntil = {};
  Timer? _expiry;

  ///Окно ('claude', 'codex') -> замеры расхода в текущем окне
  final Map<String, List<(int, double)>> _samples = {};

  ///Окно -> текст прогноза, пока он актуален
  final Map<String, String> _forecasts = {};

  ///Уже показанные уведомления, чтобы не повторять их на каждый запрос или замер
  final Set<String> _notified = {};

  bool get limited => _limitedUntil.isNotEmpty;
  bool get warning => limited || _forecasts.isNotEmpty;

  String get message => [
    for (final MapEntry(key: provider, value: until) in _limitedUntil.entries)
      _limitedMessage(provider, until),
    ..._forecasts.values,
  ].join('\n');

  void _checkLimited() {
    final now = DateTime.now().millisecondsSinceEpoch;
    var changed = false;
    //Записи идут новыми сверху: берём последний сброс для каждого провайдера
    for (final e in _requests.entries.reversed) {
      final until = e.retryAt;
      final provider = e.provider;
      if (until == null || provider == null || until <= now) {
        continue;
      }
      if (_limitedUntil[provider] != until) {
        _limitedUntil[provider] = until;
        changed = true;
        if (_notified.add('limit:$provider:$until')) {
          ShowNotification(
            title: tr('Лимит исчерпан', 'Limit reached'),
            body: _limitedMessage(provider, until),
          ).sendSignalToRust();
        }
      }
    }
    if (_expire(now) || changed) {
      _scheduleExpiry();
      notifyListeners();
    }
  }

  ///Убирает сброшенные лимиты; true, если что-то убрано
  bool _expire(int now) {
    final before = _limitedUntil.length;
    _limitedUntil.removeWhere((_, until) => until <= now);
    return _limitedUntil.length != before;
  }

  void _scheduleExpiry() {
    _expiry?.cancel();
    if (_limitedUntil.isEmpty) {
      return;
    }
    final next = _limitedUntil.values.reduce((a, b) => a < b ? a : b);
    final delay = DateTime.fromMillisecondsSinceEpoch(next)
        .difference(DateTime.now());
    _expiry = Timer(delay.isNegative ? Duration.zero : delay, () {
      if (_expire(DateTime.now().millisecondsSinceEpoch)) {
        _scheduleExpiry();
        notifyListeners();
      }
    });
  }

  void _checkForecast() {
    var changed = false;
    for (final (key, name, usage) in [
      ('claude', 'Claude', _accounts.claudeUsage),
      ('codex', 'ChatGPT', _accounts.codexUsage),
    ]) {
      final text = _forecast(key, name, usage);
      if (text != _forecasts[key]) {
        changed = true;
        if (text == null) {
          _forecasts.remove(key);
        } else {
          _forecasts[key] = text;
        }
      }
    }
    if (changed) {
      notifyListeners();
    }
  }

  ///Текст предупреждения о пятичасовом окне или null, если всё в порядке
  String? _forecast(String key, String name, Usage? usage) {
    final window = usage?.windows
        .where((w) => w.windowMinutes == 300 && w.resetsAt != null)
        .firstOrNull;
    if (usage == null || window == null) {
      _samples.remove(key);
      return null;
    }
    final resetsAt = window.resetsAt!;
    final samples = _samples.putIfAbsent(key, () => []);
    //Новое окно: старые замеры ничего не говорят о темпе
    if (samples.isNotEmpty && window.usedPercent < samples.last.$2) {
      samples.clear();
    }
    if (samples.isEmpty || samples.last.$1 != usage.observedAt) {
      samples.add((usage.observedAt, window.usedPercent));
    }
    samples.removeWhere(
      (s) => usage.observedAt - s.$1 > _paceSpan.inMilliseconds,
    );

    final used = window.usedPercent;
    final first = samples.first;
    final span = usage.observedAt - first.$1;
    //Процентов в миллисекунду
    final pace = span >= _minPaceSpan.inMilliseconds
        ? (used - first.$2) / span
        : 0.0;
    final runsOutAt = pace > 0 && used < 100
        ? usage.observedAt + ((100 - used) / pace).round()
        : null;
    final early = runsOutAt != null && runsOutAt < resetsAt;
    if (used < _warnPercent && !early) {
      return null;
    }

    final reset = _clock(resetsAt);
    final text = early
        ? tr(
            '$name: израсходовано ${used.round()}% пятичасового лимита, при таком темпе он закончится к ${_clock(runsOutAt)}, а сброс в $reset',
            '$name: ${used.round()}% of the 5-hour limit used; at this pace it runs out by ${_clock(runsOutAt)}, the reset is at $reset',
          )
        : tr(
            '$name: израсходовано ${used.round()}% пятичасового лимита, сброс в $reset',
            '$name: ${used.round()}% of the 5-hour limit used, the reset is at $reset',
          );
    //Одно уведомление на окно о каждом поводе: порог 80% и нехватка до сброса
    if (_notified.add(
      'forecast:$key:$resetsAt:${early ? 'pace' : 'percent'}',
    )) {
      ShowNotification(
        title: tr('Лимит скоро закончится', 'Limit running out'),
        body: text,
      ).sendSignalToRust();
    }
    return text;
  }

  String _limitedMessage(String provider, int until) => tr(
    'Лимит ${_providerName(provider)} до ${_clock(until)}: прокси отвечает сразу, не отправляя запрос; смените модель через /model',
    '${_providerName(provider)} limit until ${_clock(until)}: the proxy answers at once without sending the request; switch the model with /model',
  );

  String _providerName(String provider) => switch (provider) {
    'codex-oauth' => 'ChatGPT',
    'openai' => 'OpenAI API',
    'openai-compat' => tr('OpenAI-совместимого API', 'OpenAI-compatible API'),
    'gemini' || 'gemini-oauth' => 'Gemini',
    'anthropic' => 'Anthropic',
    _ => provider,
  };

  ///17:46, или 26.10 09:15, если не сегодня
  String _clock(int unixMs) {
    final at = DateTime.fromMillisecondsSinceEpoch(unixMs);
    final now = DateTime.now();
    final today =
        at.year == now.year && at.month == now.month && at.day == now.day;
    return DateFormat(today ? 'HH:mm' : 'dd.MM HH:mm').format(at);
  }

  @override
  void dispose() {
    _expiry?.cancel();
    _requests.removeListener(_checkLimited);
    _accounts.removeListener(_checkForecast);
    super.dispose();
  }
}
