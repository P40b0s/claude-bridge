import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

///Текущее местоположение и список стран, из которых разрешено проксирование
class GeoProvider extends ChangeNotifier
{
  GeoState? _state;
  ///null, пока Rust не прислал первое состояние
  GeoState? get state => _state;
  bool get enabled => _state?.enabled ?? false;
  Set<String> get allowed => (_state?.allowed ?? const <String>[]).toSet();

  ///Страна текущего IP в списке разрешённых; null, если страна не определена
  bool? get locationAllowed
  {
    final code = _state?.countryCode;
    return code == null ? null : allowed.contains(code);
  }

  ///Запросы сейчас блокируются
  bool get blocking => enabled && locationAllowed != true;

  void update(GeoState state)
  {
    _state = state;
    notifyListeners();
  }
}
