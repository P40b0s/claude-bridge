import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/providers/settings_provider.dart';

class SettingsService
{
  final SettingsProvider provider = SettingsProvider();
  late final StreamSubscription _sub;

  SettingsService()
  {
    _sub = SettingsState.rustSignalStream.listen((pack)
    {
      provider.update(pack.message);
      _syncLanguage();
    });
  }

  ///Сохраняет только переданные ключи; пустое значение удаляет ключ из .env
  void save(Map<String, String> values)
  {
    if (values.isEmpty)
    {
      return;
    }
    SaveSettings(values: values).sendSignalToRust();
  }

  ///Включает или отключает группу провайдеров; прокси и UI подхватывают сразу
  void setProviderEnabled(String family, bool enabled)
  {
    final disabled = provider.disabledProviders;
    enabled ? disabled.remove(family) : disabled.add(family);
    save({disabledProvidersKey: (disabled.toList()..sort()).join(',')});
  }

  ///Язык из .env; если его там нет, туда записывается системный, чтобы Rust писал уведомления на нём же
  void _syncLanguage()
  {
    final saved = parseLang(provider.value(langKey));
    if (saved == null)
    {
      save({langKey: langNotifier.value.name});
    }
    else
    {
      langNotifier.value = saved;
    }
  }

  void setLanguage(AppLang lang)
  {
    langNotifier.value = lang;
    save({langKey: lang.name});
  }

  Future<void> dispose() async
  {
    await _sub.cancel();
    provider.dispose();
  }
}
