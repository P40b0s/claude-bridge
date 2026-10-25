import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

class AccountsProvider extends ChangeNotifier
{
  AccountsState? _state;
  ///Собственный вход Claude Code (/login), null если не вошел
  ClaudeCodeAccount? get claudeCode => _state?.claudeCode;
  ChatGptAccount? get chatgpt => _state?.chatgpt;
  ///Аккаунт Google в слоте 1 или 2, null если не привязан
  GoogleAccount? google(int account) => _state?.google.where((g) => g.account == account).firstOrNull;

  LoginState? _login;
  ///Текущий вход через браузер (или его итог), null если входа не было
  LoginState? get login => _login;
  bool isWaiting(AuthProvider provider, int account) =>
      _login != null && _login!.phase == LoginPhase.waiting && _login!.provider == provider && _login!.account == account;
  bool get anyWaiting => _login?.phase == LoginPhase.waiting;

  UsageState? _usage;
  ///Лимиты Claude из последнего ответа Anthropic, пересланного прокси
  Usage? get claudeUsage => _usage?.claude;
  Usage? get codexUsage => _usage?.codex;
  Usage? geminiUsage(int account) => _usage?.gemini.where((u) => u.account == account).firstOrNull;

  void updateUsage(UsageState state)
  {
    _usage = state;
    notifyListeners();
  }

  void updateAccounts(AccountsState state)
  {
    _state = state;
    notifyListeners();
  }

  void updateLogin(LoginState state)
  {
    _login = state;
    notifyListeners();
  }
}
