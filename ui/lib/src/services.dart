import 'package:claude_bridge_ui/src/services/accounts_service.dart';
import 'package:claude_bridge_ui/src/services/context_watch.dart';
import 'package:claude_bridge_ui/src/services/error_service.dart';
import 'package:claude_bridge_ui/src/services/geo_service.dart';
import 'package:claude_bridge_ui/src/services/models_service.dart';
import 'package:claude_bridge_ui/src/services/proxy_service.dart';
import 'package:claude_bridge_ui/src/services/requests_service.dart';
import 'package:claude_bridge_ui/src/services/settings_service.dart';
import 'package:claude_bridge_ui/src/services/tray_service.dart';
import 'package:flutter/material.dart';

class AppServices {
  static late final AppServices I;
  late ErrorService errorService;
  late ProxyService proxyService;
  late AccountsService accountsService;
  late RequestsService requestsService;
  late SettingsService settingsService;
  late ModelsService modelsService;
  late GeoService geoService;
  late ContextWatch contextWatch;
  late TrayService trayService;
  AppServices._() {
    errorService = ErrorService();
    proxyService = ProxyService();
    accountsService = AccountsService();
    requestsService = RequestsService();
    settingsService = SettingsService();
    contextWatch = ContextWatch(
      requestsService.provider,
      settingsService.provider,
    );
    trayService = TrayService(proxyService, contextWatch);
    modelsService = ModelsService();
    geoService = GeoService();
  }

  ///Сервисы подписываются на сигналы Rust, поэтому создаются до initializeRust
  static void init() {
    I = AppServices._();
  }

  Future<void> dispose() async {
    await trayService.dispose();
    contextWatch.dispose();
    await errorService.dispose();
    await proxyService.dispose();
    await accountsService.dispose();
    await requestsService.dispose();
    await settingsService.dispose();
    await modelsService.dispose();
    await geoService.dispose();
  }
}

extension AppServicesX on BuildContext {
  AppServices get appServices => AppServices.I;
}
