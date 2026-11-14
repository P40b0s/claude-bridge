import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

///Выходы, маршруты провайдеров и страны, из которых разрешено проксирование
class GeoProvider extends ChangeNotifier
{
  GeoState? _state;
  ///null, пока Rust не прислал первое состояние
  GeoState? get state => _state;
  bool get enabled => _state?.enabled ?? false;
  ///Общий список для провайдеров без своего
  Set<String> get allowed => (_state?.allowed ?? const <String>[]).toSet();
  List<ProviderGeo> get providers => _state?.providers ?? const [];

  ///Страна, которую видит провайдер, в его списке; null, если страна не определена
  static bool? permits(ProviderGeo p)
  {
    final code = p.exit.countryCode;
    return code == null ? null : p.allowed.contains(code);
  }

  ///Провайдеры, запросы к которым сейчас отклоняются
  List<ProviderGeo> get blocked => enabled ? providers.where((p) => permits(p) != true).toList() : const [];

  ///Хотя бы один провайдер сейчас блокируется
  bool get blocking => blocked.isNotEmpty;

  void update(GeoState state)
  {
    _state = state;
    notifyListeners();
  }
}

///Название группы провайдеров для показа
String familyName(String family) => switch (family)
{
  'anthropic' => 'Anthropic',
  'codex' => 'ChatGPT / Codex',
  'openai' => 'OpenAI API',
  'openai-compat' => 'OpenAI-compatible',
  _ => family,
};
