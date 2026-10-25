// ignore_for_file: deprecated_member_use

import 'dart:async';
import 'dart:io' as io;

import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/services/proxy_service.dart';
import 'package:flutter/foundation.dart';
import 'package:nativeapi/nativeapi.dart' show ContextMenuTrigger;
import 'package:tray_manager/legacy.dart';
import 'package:window_manager/window_manager.dart';

class TrayService with WindowListener {
  TrayService(this._proxyService);

  final ProxyService _proxyService;
  late final VoidCallback _onProxyChanged;
  late final VoidCallback _onLanguageChanged;
  bool _exiting = false;

  Future<void> init() async {
    if (kIsWeb || !_supported) {
      return;
    }
    _onProxyChanged = _update;
    _onLanguageChanged = _update;
    _proxyService.provider.addListener(_onProxyChanged);
    langNotifier.addListener(_onLanguageChanged);
    windowManager.addListener(this);
    await windowManager.setPreventClose(true);
    await _updateTray();
  }

  bool get _supported =>
      io.Platform.isLinux || io.Platform.isMacOS || io.Platform.isWindows;

  void _update() {
    unawaited(_updateTray());
  }

  Future<void> _updateTray() async {
    final state = _proxyService.provider.state;
    final (status, icon) = switch (state) {
      null => (
        tr('Проверка прокси', 'Checking proxy'),
        'assets/icons/bridge_checking.png',
      ),
      _ when state.busy => (
        tr('Прокси обрабатывает запрос', 'Proxy is processing a request'),
        'assets/icons/bridge_busy.png',
      ),
      _ when state.running => (
        tr(
          'Прокси работает на 127.0.0.1:${state.port}',
          'Proxy running on 127.0.0.1:${state.port}',
        ),
        'assets/icons/bridge_running.png',
      ),
      _ => (
        tr('Прокси остановлен', 'Proxy stopped'),
        'assets/icons/bridge_stopped.png',
      ),
    };
    await trayManager.setIcon(icon);
    await trayManager.setToolTip('Claude Bridge — $status');
    trayManager.trayIcon?.setContextMenuTrigger(ContextMenuTrigger.rightClicked);
    await trayManager.setContextMenu(
      Menu(
        items: [
          MenuItem(label: status, disabled: true),
          MenuItem.separator(),
          MenuItem(
            label: tr('Открыть Claude Bridge', 'Show Claude Bridge'),
            onClick: (_) => _showWindow(),
          ),
          if (state?.running ?? false)
            MenuItem(
              label: tr('Остановить прокси', 'Stop proxy'),
              onClick: (_) => _proxyService.stop(),
            )
          else
            MenuItem(
              label: tr('Запустить прокси', 'Start proxy'),
              onClick: (_) => _proxyService.start(),
            ),
          MenuItem(
            label: tr('Перезапустить прокси', 'Restart proxy'),
            onClick: (_) => _proxyService.restart(),
          ),
          MenuItem.separator(),
          MenuItem(label: tr('Выйти', 'Exit'), onClick: (_) => _exit()),
        ],
      ),
    );
  }

  Future<void> _showWindow() async {
    await windowManager.show();
    await windowManager.focus();
  }

  Future<void> _exit() async {
    _exiting = true;
    await windowManager.destroy();
  }

  @override
  void onWindowClose() {
    if (_exiting) {
      io.exit(0);
    }
    unawaited(windowManager.hide());
  }

  Future<void> dispose() async {
    if (_supported) {
      _proxyService.provider.removeListener(_onProxyChanged);
      langNotifier.removeListener(_onLanguageChanged);
      windowManager.removeListener(this);
      await trayManager.destroy();
    }
  }
}
