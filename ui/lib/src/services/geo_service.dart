import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/geo_provider.dart';

class GeoService
{
  final GeoProvider provider = GeoProvider();
  late final StreamSubscription _sub;

  GeoService()
  {
    _sub = GeoState.rustSignalStream.listen((pack) => provider.update(pack.message));
  }

  ///Проверить выходы и провайдеров сейчас, не дожидаясь таймера
  void refresh() => RefreshLocation().sendSignalToRust();

  ///Включает проверку и задаёт общий список стран; прокси подхватывает сразу, без перезапуска
  void setPolicy({required bool enabled, required Iterable<String> allowed}) =>
      SetGeoPolicy(enabled: enabled, allowed: allowed.toList()).sendSignalToRust();

  ///Выход и страны одного провайдера; allowed == null - общий список
  void setProvider(String family, {required String exit, required Iterable<String>? allowed}) =>
      SetProviderGeo(family: family, exit: exit, allowed: allowed?.toList()).sendSignalToRust();

  ///Заменяет список выходов (кроме прямого)
  void setExits(Iterable<String> exits) => SetExits(exits: exits.toList()).sendSignalToRust();

  Future<void> dispose() async
  {
    await _sub.cancel();
    provider.dispose();
  }
}
