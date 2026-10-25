import 'package:intl/intl.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///1234 -> 1.2k, 2500000 -> 2.5M
String formatTokens(int tokens)
{
  if (tokens >= 1000000)
  {
    return '${(tokens / 1000000).toStringAsFixed(1)}M';
  }
  if (tokens >= 1000)
  {
    return '${(tokens / 1000).toStringAsFixed(1)}k';
  }
  return '$tokens';
}

String formatDuration(int ms)
{
  if (ms < 1000)
  {
    return tr('$ms мс', '$ms ms');
  }
  final seconds = ms / 1000;
  if (seconds < 60)
  {
    return tr('${seconds.toStringAsFixed(1)} с', '${seconds.toStringAsFixed(1)} s');
  }
  return tr('${seconds ~/ 60} мин ${(seconds % 60).round()} с', '${seconds ~/ 60} min ${(seconds % 60).round()} s');
}

String formatTime(int unixMs) => DateFormat('HH:mm:ss').format(DateTime.fromMillisecondsSinceEpoch(unixMs));

///Сколько осталось жить токену; просроченный обновляется прокси сам
String formatExpiry(int expiresAtMs)
{
  final left = DateTime.fromMillisecondsSinceEpoch(expiresAtMs).difference(DateTime.now());
  if (left.isNegative)
  {
    return tr('токен истек, обновится при следующем запросе', 'token expired, refreshed on the next request');
  }
  if (left.inHours >= 24)
  {
    return tr('токен действует еще ${left.inDays} дн', 'token valid for ${left.inDays} more d');
  }
  if (left.inMinutes >= 60)
  {
    return tr('токен действует еще ${left.inHours} ч', 'token valid for ${left.inHours} more h');
  }
  return tr('токен действует еще ${left.inMinutes} мин', 'token valid for ${left.inMinutes} more min');
}

String formatUptime(int startedAtMs)
{
  final up = DateTime.now().difference(DateTime.fromMillisecondsSinceEpoch(startedAtMs));
  if (up.inHours > 0)
  {
    return tr('${up.inHours} ч ${up.inMinutes % 60} мин', '${up.inHours} h ${up.inMinutes % 60} min');
  }
  return tr('${up.inMinutes} мин', '${up.inMinutes} min');
}

///Длина окна лимита: 300 -> 5 часов, 10080 -> 7 дней
String formatWindow(int minutes)
{
  if (minutes >= 24 * 60 && minutes % (24 * 60) == 0)
  {
    final days = minutes ~/ (24 * 60);
    return '$days ${plural(days, 'день', 'дня', 'дней', 'day', 'days')}';
  }
  if (minutes >= 60 && minutes % 60 == 0)
  {
    final hours = minutes ~/ 60;
    return '$hours ${plural(hours, 'час', 'часа', 'часов', 'hour', 'hours')}';
  }
  return tr('$minutes мин', '$minutes min');
}

///Сколько осталось до момента: 3 дн 12 ч, 2 ч 15 мин, 40 мин
String formatTimeLeft(int atMs)
{
  final left = DateTime.fromMillisecondsSinceEpoch(atMs).difference(DateTime.now());
  if (left.isNegative)
  {
    return tr('уже', 'now');
  }
  if (left.inDays > 0)
  {
    return tr('${left.inDays} дн ${left.inHours % 24} ч', '${left.inDays} d ${left.inHours % 24} h');
  }
  if (left.inHours > 0)
  {
    return tr('${left.inHours} ч ${left.inMinutes % 60} мин', '${left.inHours} h ${left.inMinutes % 60} min');
  }
  return tr('${left.inMinutes} мин', '${left.inMinutes} min');
}

String formatDate(int unixMs) => DateFormat('dd.MM.yyyy').format(DateTime.fromMillisecondsSinceEpoch(unixMs));

///Окончание подписки: «до 25.11.2026, осталось 31 день» или «закончилась 01.10.2026»
String formatSubscriptionEnd(int untilMs)
{
  final left = DateTime.fromMillisecondsSinceEpoch(untilMs).difference(DateTime.now());
  if (left.isNegative)
  {
    return tr('подписка закончилась ${formatDate(untilMs)}', 'subscription ended ${formatDate(untilMs)}');
  }
  final days = left.inDays;
  final rest = days == 0
      ? tr('заканчивается сегодня', 'ends today')
      : tr('осталось $days ${plural(days, 'день', 'дня', 'дней', 'day', 'days')}', '$days ${days == 1 ? 'day' : 'days'} left');
  return tr('подписка до ${formatDate(untilMs)}, $rest', 'subscription until ${formatDate(untilMs)}, $rest');
}

String formatDateTime(int unixMs) => DateFormat('dd.MM HH:mm').format(DateTime.fromMillisecondsSinceEpoch(unixMs));

///Давность: только что, 5 мин назад, 2 ч назад
String formatAgo(int unixMs)
{
  final ago = DateTime.now().difference(DateTime.fromMillisecondsSinceEpoch(unixMs));
  if (ago.inMinutes < 1)
  {
    return tr('только что', 'just now');
  }
  if (ago.inHours < 1)
  {
    return tr('${ago.inMinutes} мин назад', '${ago.inMinutes} min ago');
  }
  if (ago.inDays < 1)
  {
    return tr('${ago.inHours} ч назад', '${ago.inHours} h ago');
  }
  return formatDateTime(unixMs);
}
