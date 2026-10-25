import 'dart:async';

import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/models_provider.dart';

class ModelsService {
  final ModelsProvider provider = ModelsProvider();
  late final StreamSubscription _sub;

  ModelsService() {
    _sub = ModelsState.rustSignalStream.listen(
      (pack) => provider.update(pack.message),
    );
  }

  void save({
    String? id,
    required String name,
    required String provider,
    required String model,
    String? reasoning,
  }) => SaveModel(
    id: id,
    name: name,
    provider: provider,
    model: model,
    reasoning: reasoning,
  ).sendSignalToRust();

  void delete(String id) => DeleteModel(id: id).sendSignalToRust();
  void select(String id) => SelectModel(id: id).sendSignalToRust();

  Future<void> dispose() async {
    await _sub.cancel();
    provider.dispose();
  }
}
