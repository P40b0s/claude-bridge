import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Живой журнал запросов, которые проходят через прокси
class RequestsPage extends StatelessWidget
{
  const RequestsPage({super.key});

  @override
  Widget build(BuildContext context)
  {
    final requests = context.appServices.requestsService.provider;
    final proxy = context.appServices.proxyService.provider;
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([requests, proxy, settings]),
      builder: (context, _)
      {
        //Запросы к отключённым провайдерам не показываются, в том числе в счётчиках
        final entries = requests.entries.where((e) => settings.providerEnabled(e.provider)).toList();
        int sum(int? Function(RequestEntry) f) => entries.fold(0, (total, e) => total + (f(e) ?? 0));
        return Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(24, 16, 24, 8),
              child: Row(
                children: [
                  Expanded(
                    child: Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        _Stat(tr('Запросов', 'Requests'), '${entries.length}'),
                        _Stat(tr('Выполняется', 'Running'), '${entries.where((e) => e.phase == RequestPhase.running).length}'),
                        _Stat(tr('Ошибок', 'Errors'), '${entries.where((e) => e.phase == RequestPhase.error).length}'),
                        _Stat(tr('Вход', 'Input'), formatTokens(sum((e) => e.inputTokens))),
                        _Stat(tr('Из кэша', 'Cached'), formatTokens(sum((e) => e.cacheReadTokens))),
                        _Stat(tr('Выход', 'Output'), formatTokens(sum((e) => e.outputTokens))),
                      ],
                    ),
                  ),
                  IconButton(
                    tooltip: tr('Очистить', 'Clear'),
                    onPressed: entries.isEmpty ? null : requests.clear,
                    icon: const Icon(Icons.delete_sweep_outlined),
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Expanded(
              child: entries.isEmpty
                  ? Center(
                      child: Text(
                        proxy.running ? tr('Запросов пока не было', 'No requests yet') : tr('Прокси не запущен', 'The proxy is not running'),
                        style: TextStyle(color: Theme.of(context).hintColor),
                      ),
                    )
                  : ListView.separated(
                      itemCount: entries.length,
                      separatorBuilder: (_, _) => const Divider(height: 1),
                      itemBuilder: (context, i) => _RequestTile(entries[i]),
                    ),
            ),
          ],
        );
      },
    );
  }
}

class _Stat extends StatelessWidget
{
  const _Stat(this.label, this.value);
  final String label;
  final String value;

  @override
  Widget build(BuildContext context)
  {
    return Chip(label: Text('$label: $value'), visualDensity: VisualDensity.compact);
  }
}

class _RequestTile extends StatelessWidget
{
  const _RequestTile(this.entry);
  final RequestEntry entry;

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final target = entry.model == null
        ? entry.requested
        : '${entry.provider}:${entry.model}${entry.reasoning == null ? '' : '@${entry.reasoning}'}';
    final remapped = entry.requested != entry.model && entry.requested.isNotEmpty;
    final usage = entry.inputTokens == null
        ? ''
        : tr('  ·  вход ${formatTokens(entry.inputTokens!)}, кэш ${formatTokens(entry.cacheReadTokens ?? 0)}, выход ${formatTokens(entry.outputTokens ?? 0)}', '  ·  input ${formatTokens(entry.inputTokens!)}, cached ${formatTokens(entry.cacheReadTokens ?? 0)}, output ${formatTokens(entry.outputTokens ?? 0)}');
    final duration = entry.durationMs == null ? '' : '  ·  ${formatDuration(entry.durationMs!)}';
    final stop = entry.stopReason == null ? '' : '  ·  ${entry.stopReason}';
    return ListTile(
      dense: true,
      leading: _phaseIcon(entry.phase, theme),
      title: Text(remapped ? '$target  ←  ${entry.requested}' : target),
      subtitle: Text(
        entry.error ?? '${formatTime(entry.startedAt)}${entry.stream ? '  ·  stream' : ''}$duration$usage$stop',
        style: entry.error == null ? null : TextStyle(color: theme.colorScheme.error),
        maxLines: 3,
        overflow: TextOverflow.ellipsis,
      ),
      trailing: entry.httpStatus == null ? null : Text('${entry.httpStatus}'),
    );
  }

  Widget _phaseIcon(RequestPhase phase, ThemeData theme)
  {
    switch (phase)
    {
      case RequestPhase.running:
        return const SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2));
      case RequestPhase.ok:
        return const Icon(Icons.check_circle_outline, color: Colors.green);
      case RequestPhase.error:
        return Icon(Icons.error_outline, color: theme.colorScheme.error);
      case RequestPhase.cancelled:
        return Icon(Icons.cancel_outlined, color: theme.hintColor);
    }
  }
}
