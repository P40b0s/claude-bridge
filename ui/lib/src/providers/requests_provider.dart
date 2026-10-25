import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

///Журнал запросов прокси, новые сверху
class RequestsProvider extends ChangeNotifier
{
  RequestsProvider({this.maxEntries = 500});
  final int maxEntries;
  final List<RequestEntry> _entries = [];
  List<RequestEntry> get entries => List.unmodifiable(_entries);

  int get running => _entries.where((e) => e.phase == RequestPhase.running).length;
  int get errors => _entries.where((e) => e.phase == RequestPhase.error).length;
  int get inputTokens => _entries.fold(0, (sum, e) => sum + (e.inputTokens ?? 0));
  int get cacheReadTokens => _entries.fold(0, (sum, e) => sum + (e.cacheReadTokens ?? 0));
  int get outputTokens => _entries.fold(0, (sum, e) => sum + (e.outputTokens ?? 0));

  ///Прокси присылает запрос дважды: при старте и по завершении, с тем же id
  void upsert(RequestEntry entry)
  {
    final index = _entries.indexWhere((e) => e.id == entry.id);
    if (index >= 0)
    {
      _entries[index] = entry;
    }
    else
    {
      _entries.insert(0, entry);
      if (_entries.length > maxEntries)
      {
        _entries.removeRange(maxEntries, _entries.length);
      }
    }
    notifyListeners();
  }

  void clear()
  {
    _entries.clear();
    notifyListeners();
  }
}
