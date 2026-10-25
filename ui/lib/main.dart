import 'dart:io' as io;

import 'package:claude_bridge_ui/src/controls/proxy_status_icon.dart';
import 'package:claude_bridge_ui/src/controls/toast.dart';
import 'package:claude_bridge_ui/src/i18n.dart';
import 'package:claude_bridge_ui/src/pages/accounts_page.dart';
import 'package:claude_bridge_ui/src/pages/geo_page.dart';
import 'package:claude_bridge_ui/src/pages/home_page.dart';
import 'package:claude_bridge_ui/src/pages/models_page.dart';
import 'package:claude_bridge_ui/src/pages/providers_page.dart';
import 'package:claude_bridge_ui/src/pages/requests_page.dart';
import 'package:claude_bridge_ui/src/pages/settings_page.dart';
import 'package:claude_bridge_ui/src/providers/error_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:claude_bridge_ui/src/themes.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';

import 'src/bindings/bindings.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  AppServices.init();
  await initializeRust(assignRustSignal);
  await AppServices.I.trayService.init();
  AppLifecycleListener(
    onExitRequested: () async {
      await AppServices.I.dispose();
      finalizeRust();
      return io.exit(0);
    },
  );
  runApp(const ClaudeBridgeApp());
}

class ClaudeBridgeApp extends StatelessWidget {
  const ClaudeBridgeApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider.value(
          value: context.appServices.errorService.provider,
        ),
      ],
      child: ListenableBuilder(
        listenable: Listenable.merge([themeNotifier, langNotifier]),
        builder: (_, _) {
          return MaterialApp(
            title: 'Claude Bridge',
            theme: AppTheme.light,
            darkTheme: AppTheme.dark,
            themeMode: themeNotifier.value,
            builder: (context, child) => ToastOverlay(child: child!),
            //Строки берутся через tr() при построении, поэтому при смене языка дерево строится заново
            home: HomeShell(key: ValueKey(langNotifier.value)),
          );
        },
      ),
    );
  }
}

class _Destination {
  const _Destination(this.label, this.icon, this.selectedIcon, this.page);
  final String label;
  final IconData icon;
  final IconData selectedIcon;
  final Widget page;
}

List<_Destination> get _destinations => [
  _Destination(
    tr('Главная', 'Home'),
    Icons.hub_outlined,
    Icons.hub,
    const HomePage(),
  ),
  _Destination(
    tr('Провайдеры', 'Providers'),
    Icons.extension_outlined,
    Icons.extension,
    const ProvidersPage(),
  ),
  _Destination(
    tr('Аккаунты', 'Accounts'),
    Icons.account_circle_outlined,
    Icons.account_circle,
    const AccountsPage(),
  ),
  _Destination(
    tr('Модели', 'Models'),
    Icons.smart_toy_outlined,
    Icons.smart_toy,
    const ModelsPage(),
  ),
  _Destination(
    tr('Запросы', 'Requests'),
    Icons.list_alt_outlined,
    Icons.list_alt,
    const RequestsPage(),
  ),
  _Destination(
    tr('Страны', 'Countries'),
    Icons.public_outlined,
    Icons.public,
    const GeoPage(),
  ),
  _Destination(
    tr('Настройки', 'Settings'),
    Icons.settings_outlined,
    Icons.settings,
    const SettingsPage(),
  ),
];

class HomeShell extends StatefulWidget {
  const HomeShell({super.key});

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  ///Статическое: вкладка сохраняется, когда смена языка пересоздаёт HomeShell
  static int _index = 0;

  @override
  Widget build(BuildContext context) {
    final dark = Theme.of(context).brightness == Brightness.dark;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Claude Bridge'),
        shape: Border(
          bottom: BorderSide(color: Theme.of(context).colorScheme.primary),
        ),
        leading: const ProxyStatusIcon(),
        actions: [
          TextButton(
            onPressed: () => context.appServices.settingsService.setLanguage(
              isRussian ? AppLang.en : AppLang.ru,
            ),
            child: Text(isRussian ? 'RU' : 'EN'),
          ),
          IconButton(
            icon: Icon(dark ? Icons.wb_sunny : Icons.nightlight_round),
            tooltip: tr('Тема', 'Theme'),
            onPressed: () =>
                themeNotifier.value = dark ? ThemeMode.light : ThemeMode.dark,
          ),
          Consumer<ErrorProvider>(
            builder: (context, errors, _) => IconButton(
              icon: Badge(
                isLabelVisible: errors.count > 0,
                label: Text('${errors.count}'),
                child: const Icon(Icons.history),
              ),
              tooltip: tr('Уведомления', 'Notifications'),
              onPressed: () => RecentErrorsPanel.show(context),
            ),
          ),
        ],
      ),
      body: Row(
        children: [
          NavigationRail(
            selectedIndex: _index,
            labelType: NavigationRailLabelType.all,
            onDestinationSelected: (i) => setState(() => _index = i),
            destinations: [
              for (final d in _destinations)
                NavigationRailDestination(
                  icon: Icon(d.icon),
                  selectedIcon: Icon(d.selectedIcon),
                  label: Text(d.label),
                ),
            ],
          ),
          const VerticalDivider(width: 1),
          Expanded(
            child: Align(
              alignment: Alignment.topCenter,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 1100),
                //IndexedStack сохраняет состояние форм при переключении вкладок
                child: IndexedStack(
                  index: _index,
                  children: [for (final d in _destinations) d.page],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
