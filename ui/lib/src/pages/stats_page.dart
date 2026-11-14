import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:intl/intl.dart';

enum _Preset { today, week, month, custom }

int _all(StatsTokens t) => t.input + t.cacheRead + t.cacheWrite + t.output;

///Сколько токенов потрачено за выбранный период: всего, по дням и по моделям
class StatsPage extends StatefulWidget
{
  const StatsPage({super.key});

  @override
  State<StatsPage> createState() => _StatsPageState();
}

class _StatsPageState extends State<StatsPage>
{
  _Preset _preset = _Preset.week;
  late DateTimeRange _range = _presetRange(_Preset.week);
  Timer? _timer;

  static DateTime get _today
  {
    final now = DateTime.now();
    return DateTime(now.year, now.month, now.day);
  }

  static DateTimeRange _presetRange(_Preset p) => switch (p)
  {
    _Preset.today => DateTimeRange(start: _today, end: _today),
    _Preset.month => DateTimeRange(start: _today.subtract(const Duration(days: 29)), end: _today),
    _ => DateTimeRange(start: _today.subtract(const Duration(days: 6)), end: _today),
  };

  @override
  void initState()
  {
    super.initState();
    _load();
    //Пока приложение открыто, цифры подтягиваются сами
    _timer = Timer.periodic(const Duration(seconds: 30), (_) => _load());
  }

  @override
  void dispose()
  {
    _timer?.cancel();
    super.dispose();
  }

  void _load()
  {
    //Готовые периоды сдвигаются вместе с датой
    if (_preset != _Preset.custom)
    {
      _range = _presetRange(_preset);
    }
    context.appServices.statsService.request(_range.start, _range.end);
  }

  Future<void> _pick(_Preset p) async
  {
    if (p != _Preset.custom)
    {
      setState(() => _preset = p);
      _load();
      return;
    }
    final picked = await showDateRangePicker(
      context: context,
      firstDate: DateTime(2024),
      lastDate: _today,
      initialDateRange: _range,
    );
    if (picked == null || !mounted)
    {
      return;
    }
    setState(()
    {
      _preset = _Preset.custom;
      _range = picked;
    });
    _load();
  }

  @override
  Widget build(BuildContext context)
  {
    final service = context.appServices.statsService;
    final dates = DateFormat('dd.MM.yyyy');
    return ValueListenableBuilder<StatsReport?>(
      valueListenable: service.report,
      builder: (context, report, _)
      {
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            Row(
              children: [
                SegmentedButton<_Preset>(
                  segments: [
                    ButtonSegment(value: _Preset.today, label: Text(tr('Сегодня', 'Today'))),
                    ButtonSegment(value: _Preset.week, label: Text(tr('7 дней', '7 days'))),
                    ButtonSegment(value: _Preset.month, label: Text(tr('30 дней', '30 days'))),
                    ButtonSegment(value: _Preset.custom, label: Text(tr('Период…', 'Range…')), icon: const Icon(Icons.date_range)),
                  ],
                  selected: {_preset},
                  showSelectedIcon: false,
                  onSelectionChanged: (s) => _pick(s.first),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Text(
                    _range.start == _range.end
                        ? dates.format(_range.start)
                        : '${dates.format(_range.start)} — ${dates.format(_range.end)}',
                  ),
                ),
                IconButton(tooltip: tr('Обновить', 'Refresh'), onPressed: _load, icon: const Icon(Icons.refresh)),
              ],
            ),
            const SizedBox(height: 16),
            if (report == null)
              const LinearProgressIndicator()
            else ...[
              _Totals(report.total),
              if (report.days.length > 1) _Days(report.days),
              _Models(report.models),
              Text(
                tr('Файлы: ${report.dir}', 'Files: ${report.dir}'),
                style: TextStyle(color: Theme.of(context).hintColor),
              ),
            ],
          ],
        );
      },
    );
  }
}

class _Totals extends StatelessWidget
{
  const _Totals(this.t);
  final StatsTokens t;

  @override
  Widget build(BuildContext context)
  {
    final context0 = t.input + t.cacheRead + t.cacheWrite;
    final hit = context0 == 0 ? 0 : (t.cacheRead * 100 / context0).round();
    Widget stat(String label, String value, [String? hint]) => Tooltip(
      message: hint ?? '',
      child: Chip(label: Text('$label: $value')),
    );
    return SectionCard(
      title: tr('Итого', 'Total'),
      icon: Icons.summarize_outlined,
      child: Wrap(
        spacing: 8,
        runSpacing: 8,
        children: [
          stat(tr('Запросов', 'Requests'), '${t.requests}'),
          stat(tr('Всего токенов', 'All tokens'), formatTokens(_all(t))),
          stat(tr('Вход', 'Input'), formatTokens(t.input), tr('Без кэша', 'Not from the cache')),
          stat(tr('Из кэша', 'Cached'), formatTokens(t.cacheRead)),
          stat(tr('Запись в кэш', 'Cache write'), formatTokens(t.cacheWrite)),
          stat(tr('Выход', 'Output'), formatTokens(t.output)),
          stat(tr('Попадание в кэш', 'Cache hit'), '$hit%'),
          if (t.requests > 0) stat(tr('В среднем на запрос', 'Per request'), formatTokens(_all(t) ~/ t.requests)),
        ],
      ),
    );
  }
}

///Столбцы по дням: светлая часть - из кэша, тёмная - остальное
class _Days extends StatelessWidget
{
  const _Days(this.days);
  final List<StatsDay> days;

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final max = days.fold<int>(0, (m, d) => _all(d.tokens) > m ? _all(d.tokens) : m);
    final label = DateFormat('d.MM');
    const height = 140.0;
    return SectionCard(
      title: tr('По дням', 'By day'),
      icon: Icons.bar_chart,
      child: SizedBox(
        height: height + 24,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            for (final d in days)
              Expanded(
                child: Tooltip(
                  message: '${d.date}\n'
                      '${tr('всего', 'all')} ${formatTokens(_all(d.tokens))}, ${tr('запросов', 'requests')} ${d.tokens.requests}\n'
                      '${tr('из кэша', 'cached')} ${formatTokens(d.tokens.cacheRead)}, '
                      '${tr('вход', 'input')} ${formatTokens(d.tokens.input)}, ${tr('выход', 'output')} ${formatTokens(d.tokens.output)}',
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 2),
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        if (max > 0) ...[
                          Container(
                            height: height * d.tokens.cacheRead / max,
                            color: theme.colorScheme.primary.withValues(alpha: 0.35),
                          ),
                          Container(
                            height: height * (_all(d.tokens) - d.tokens.cacheRead) / max,
                            color: theme.colorScheme.primary,
                          ),
                        ],
                        const SizedBox(height: 4),
                        if (days.length <= 31)
                          FittedBox(
                            child: Text(label.format(DateTime.parse(d.date)), style: theme.textTheme.labelSmall),
                          ),
                      ],
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Models extends StatelessWidget
{
  const _Models(this.models);
  final List<StatsModel> models;

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    return SectionCard(
      title: tr('По моделям', 'By model'),
      icon: Icons.smart_toy_outlined,
      child: models.isEmpty
          ? Text(tr('За этот период запросов не было.', 'No requests in this period.'), style: TextStyle(color: theme.hintColor))
          : SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: DataTable(
                columnSpacing: 24,
                columns: [
                  DataColumn(label: Text(tr('Модель', 'Model'))),
                  DataColumn(label: Text(tr('Запросов', 'Requests')), numeric: true),
                  DataColumn(label: Text(tr('Всего', 'All')), numeric: true),
                  DataColumn(label: Text(tr('Вход', 'Input')), numeric: true),
                  DataColumn(label: Text(tr('Из кэша', 'Cached')), numeric: true),
                  DataColumn(label: Text(tr('Запись в кэш', 'Cache write')), numeric: true),
                  DataColumn(label: Text(tr('Выход', 'Output')), numeric: true),
                ],
                rows: [
                  for (final m in models)
                    DataRow(cells: [
                      DataCell(SelectableText(m.model)),
                      DataCell(Text('${m.tokens.requests}')),
                      DataCell(Text(formatTokens(_all(m.tokens)))),
                      DataCell(Text(formatTokens(m.tokens.input))),
                      DataCell(Text(formatTokens(m.tokens.cacheRead))),
                      DataCell(Text(formatTokens(m.tokens.cacheWrite))),
                      DataCell(Text(formatTokens(m.tokens.output))),
                    ]),
                ],
              ),
            ),
    );
  }
}
