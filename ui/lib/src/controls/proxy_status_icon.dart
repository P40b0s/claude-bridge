import 'package:claude_bridge_ui/src/providers/proxy_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Значок состояния прокси для шапки
class ProxyStatusIcon extends StatelessWidget
{
  const ProxyStatusIcon({super.key});
  static const double size = 28.0;

  @override
  Widget build(BuildContext context)
  {
    final proxy = context.appServices.proxyService.provider;
    return ListenableBuilder(
      listenable: proxy,
      builder: (context, _)
      {
        final state = proxy.state;
        if (state == null)
        {
          return _status(Icons.hourglass_empty, Colors.amber, tr('Проверка прокси...', 'Checking the proxy...'));
        }
        if (state.busy)
        {
          return const Padding(
            padding: EdgeInsets.all(14),
            child: SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2.5)),
          );
        }
        if (!state.running)
        {
          return _status(Icons.power_off_sharp, Colors.redAccent, tr('Прокси не запущен (порт ${state.port})', 'The proxy is not running (port ${state.port})'));
        }
        final pinned = state.pinned == null ? '' : tr('\nЗакреплена модель ${describeModel(state.pinned)}', '\nPinned model ${describeModel(state.pinned)}');
        return _status(Icons.power_rounded, Colors.lightGreen,
            tr('Прокси работает на 127.0.0.1:${state.port}, PID ${state.pid}\nПоследняя модель ${describeModel(state.active)}$pinned', 'The proxy runs on 127.0.0.1:${state.port}, PID ${state.pid}\nLast model ${describeModel(state.active)}$pinned'));
      },
    );
  }

  Widget _status(IconData icon, Color color, String message)
  {
    return Tooltip(
      message: message,
      child: Icon(icon, size: size, color: color),
    );
  }
}
