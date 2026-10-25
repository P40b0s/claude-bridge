import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/requests_provider.dart';

class RequestsService
{
  final RequestsProvider provider = RequestsProvider();
  late final StreamSubscription _loggedSub;
  late final StreamSubscription _resetSub;

  RequestsService()
  {
    _loggedSub = RequestLogged.rustSignalStream.listen((pack) => provider.upsert(pack.message.entry));
    //Подключились к новому процессу прокси: его id начинаются заново, а недавние запросы он пришлет сам
    _resetSub = RequestLogReset.rustSignalStream.listen((_) => provider.clear());
  }

  Future<void> dispose() async
  {
    await _loggedSub.cancel();
    await _resetSub.cancel();
    provider.dispose();
  }
}
