import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

///Состояние фонового прокси, которое присылает Rust при каждом изменении
class ProxyProvider extends ChangeNotifier
{
  ProxyState? _state;
  ///null, пока Rust не прислал первое состояние
  ProxyState? get state => _state;
  bool get running => _state?.running ?? false;
  bool get busy => _state?.busy ?? false;

  void update(ProxyState state)
  {
    _state = state;
    notifyListeners();
  }
}

///Человекочитаемое имя модели: provider:model@effort
String describeModel(ModelInfo? model)
{
  if (model == null)
  {
    return '—';
  }
  final effort = model.reasoning == null ? '' : '@${model.reasoning}';
  return '${model.provider}:${model.model}$effort';
}
