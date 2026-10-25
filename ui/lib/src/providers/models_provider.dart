import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

class ModelsProvider extends ChangeNotifier {
  ModelsState? _state;
  ModelsState? get state => _state;
  List<SavedModelInfo> get models => _state?.models ?? const [];

  void update(ModelsState state) {
    _state = state;
    notifyListeners();
  }
}

String modelSpec(SavedModelInfo model) {
  final effort = model.reasoning == null ? '' : '@${model.reasoning}';
  return '${model.provider}:${model.model}$effort';
}
