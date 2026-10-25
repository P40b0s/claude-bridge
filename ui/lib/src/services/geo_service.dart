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

  void refresh() => RefreshLocation().sendSignalToRust();

  ///Сохраняет настройку; прокси подхватывает её сразу, без перезапуска
  void setPolicy({required bool enabled, required Iterable<String> allowed}) =>
      SetGeoPolicy(enabled: enabled, allowed: allowed.toList()).sendSignalToRust();

  Future<void> dispose() async
  {
    await _sub.cancel();
    provider.dispose();
  }
}
