import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Лимиты подписки: полоса на каждое окно, сколько израсходовано и когда сброс
class UsageBars extends StatelessWidget
{
  const UsageBars({super.key, required this.usage, this.source});
  final Usage usage;
  ///Откуда цифры, если это не очевидно
  final String? source;

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final hint = TextStyle(color: theme.hintColor);
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(tr('Лимиты', 'Limits'), style: theme.textTheme.titleSmall),
          if (usage.limited)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(tr('Лимит исчерпан', 'Limit reached'), style: TextStyle(color: theme.colorScheme.error)),
            ),
          for (final window in usage.windows) _WindowBar(window),
          Row(
            children: [
              Expanded(
                child: Text(
                  [?source, tr('обновлено ${formatAgo(usage.observedAt)}', 'updated ${formatAgo(usage.observedAt)}')].join(', '),
                  style: theme.textTheme.bodySmall?.merge(hint),
                ),
              ),
              IconButton(
                tooltip: tr('Обновить лимиты', 'Refresh limits'),
                visualDensity: VisualDensity.compact,
                iconSize: 18,
                icon: const Icon(Icons.refresh),
                onPressed: context.appServices.accountsService.refreshUsage,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _WindowBar extends StatelessWidget
{
  const _WindowBar(this.window);
  final UsageWindow window;

  String get _label
  {
    final span = window.windowMinutes == null ? null : formatWindow(window.windowMinutes!);
    final model = window.model == null ? null : '${window.model![0].toUpperCase()}${window.model!.substring(1)}';
    return switch ((span, model))
    {
      (final s?, final m?) => '$s, $m',
      (final s?, null) => s,
      (null, final m?) when window.id == window.model => m,
      _ => window.id,
    };
  }

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final used = window.usedPercent.clamp(0, 100).toDouble();
    final color = used >= 90
        ? theme.colorScheme.error
        : used >= 70
            ? Colors.orange
            : theme.colorScheme.primary;
    final reset = window.resetsAt == null
        ? null
        : tr('сброс через ${formatTimeLeft(window.resetsAt!)} (${formatDateTime(window.resetsAt!)})', 'resets in ${formatTimeLeft(window.resetsAt!)} (${formatDateTime(window.resetsAt!)})');
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(child: Text(_label, overflow: TextOverflow.ellipsis)),
              Text('${used.toStringAsFixed(used < 10 && used != 0 ? 1 : 0)}%'),
            ],
          ),
          const SizedBox(height: 4),
          LinearProgressIndicator(
            value: used / 100,
            minHeight: 6,
            borderRadius: BorderRadius.circular(3),
            color: color,
            backgroundColor: theme.colorScheme.surfaceContainerHighest,
          ),
          if (reset != null)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(reset, style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor)),
            ),
        ],
      ),
    );
  }
}
