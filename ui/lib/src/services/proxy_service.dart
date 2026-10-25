import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/proxy_provider.dart';
import 'package:rinf/rinf.dart';

class ProxyService
{
  final ProxyProvider provider = ProxyProvider();
  late final StreamSubscription _sub;

  ProxyService()
  {
    _sub = ProxyState.rustSignalStream.listen((pack) => _onState(pack));
  }

  void _onState(RustSignalPack<ProxyState> pack)
  {
    provider.update(pack.message);
  }

  void start() => ProxyCommand(action: ProxyAction.start).sendSignalToRust();
  void stop() => ProxyCommand(action: ProxyAction.stop).sendSignalToRust();
  void restart() => ProxyCommand(action: ProxyAction.restart).sendSignalToRust();

  ///Переключает все сессии на модель (спецификация как в /model, например terra@max)
  void pinModel(String model) => PinModel(model: model).sendSignalToRust();
  ///Сессии снова выбирают модель сами через /model
  void unpinModel() => PinModel(model: null).sendSignalToRust();

  ///Прописывает прокси в ~/.claude/settings.json или убирает его оттуда
  void connectClaudeCode(bool connect) => ConnectClaudeCode(connect: connect).sendSignalToRust();

  Future<void> dispose() async
  {
    await _sub.cancel();
    provider.dispose();
  }
}
