import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

///Расход токенов за период; данные - файлы ~/.claude-bridge/stats/ГГГГ-ММ-ДД.json
class StatsService
{
  final ValueNotifier<StatsReport?> report = ValueNotifier(null);
  late final StreamSubscription _sub;

  StatsService()
  {
    _sub = StatsReport.rustSignalStream.listen((pack) => report.value = pack.message);
  }

  void request(DateTime from, DateTime to) => StatsRequest(from: _day(from), to: _day(to)).sendSignalToRust();

  static String _day(DateTime d) =>
      '${d.year.toString().padLeft(4, '0')}-${d.month.toString().padLeft(2, '0')}-${d.day.toString().padLeft(2, '0')}';

  Future<void> dispose() async
  {
    await _sub.cancel();
    report.dispose();
  }
}
