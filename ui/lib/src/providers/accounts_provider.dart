import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

class AccountsProvider extends ChangeNotifier {
  AccountsState? _state;
  List<ClaudeCodeAccount> get claudeAccounts => _state?.claudeCode ?? const [];
  List<ChatGptAccount> get chatgptAccounts => _state?.chatgpt ?? const [];
  ClaudeCodeAccount? get claudeCode {
    for (final account in claudeAccounts) {
      if (account.active) return account;
    }
    return null;
  }

  ChatGptAccount? get chatgpt {
    for (final account in chatgptAccounts) {
      if (account.active) return account;
    }
    return null;
  }

  LoginState? _login;

  ///Текущий вход через браузер (или его итог), null если входа не было
  LoginState? get login => _login;
  bool isWaiting(AuthProvider provider, [String? profile]) =>
      _login != null &&
      _login!.phase == LoginPhase.waiting &&
      _login!.provider == provider &&
      (profile == null || _login!.profile == profile);
  bool get anyWaiting => _login?.phase == LoginPhase.waiting;

  UsageState? _usage;

  ///Лимиты Claude из последнего ответа Anthropic, пересланного прокси
  Usage? get claudeUsage => _usage?.claude;
  Usage? get codexUsage => _usage?.codex;

  void updateUsage(UsageState state) {
    _usage = state;
    notifyListeners();
  }

  void updateAccounts(AccountsState state) {
    _state = state;
    notifyListeners();
  }

  void updateLogin(LoginState state) {
    _login = state;
    notifyListeners();
  }
}
