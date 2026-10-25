import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/signals/signals.dart';
import 'package:claude_bridge_ui/src/providers/error_provider.dart';
import 'package:rinf/rinf.dart';

class ErrorService 
{
  final ErrorProvider provider = ErrorProvider();
  late final StreamSubscription _sub;
  
  ErrorService() 
  {
    _sub = ErrorSignal.rustSignalStream.listen((pack) => _onResponse(pack));
  }
  
  void _onResponse(RustSignalPack<ErrorSignal> pack) 
  {
    provider.spawnError(pack.message.error, severity: pack.message.severity);
  }
  void spawnError(String message,
  {
    String? title,
    ErrorSeverity severity = ErrorSeverity.error,
    String? actionLabel
  })
  {
    provider.spawnError(message, severity: severity);
  }

  Future<void> dispose() async 
  {
    await _sub.cancel();
    provider.dispose();
  }
}