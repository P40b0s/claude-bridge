import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

const disabledProvidersKey = 'CLAUDE_BRIDGE_DISABLED_PROVIDERS';

///Группа, которую включают и выключают целиком (как Provider::family в Rust)
String providerFamily(String provider) => switch (provider) {
  'codex-oauth' => 'codex',
  'openai-compat' => 'openai-compat',
  'gemini-oauth' || 'gemini' => 'gemini',
  _ => provider,
};

class SettingsProvider extends ChangeNotifier {
  SettingsState? _state;
  SettingsState? get state => _state;

  ///Значение из .env или пустая строка
  String value(String key) => _state?.values[key] ?? '';
  bool fromEnvironment(String key) =>
      _state?.fromEnvironment.contains(key) ?? false;
  List<PresetInfo> get presets => _state?.presets ?? const [];
  List<String> get efforts => _state?.efforts ?? const [];

  ///Отключённые группы провайдеров: anthropic, codex, openai, gemini
  Set<String> get disabledProviders =>
      value(disabledProvidersKey)
          .split(',')
          .map((f) => f.trim().toLowerCase())
          .where((f) => f.isNotEmpty)
          .toSet();
  bool isEnabled(String family) => !disabledProviders.contains(family);

  ///Провайдер маршрута (codex-oauth, gemini...) включён; неизвестные считаются включёнными
  bool providerEnabled(String? provider) =>
      provider == null || isEnabled(providerFamily(provider));

  void update(SettingsState state) {
    _state = state;
    notifyListeners();
  }
}
