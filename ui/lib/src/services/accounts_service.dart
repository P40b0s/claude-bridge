import 'dart:async';
import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/accounts_provider.dart';

class AccountsService
{
  final AccountsProvider provider = AccountsProvider();
  late final StreamSubscription _accountsSub;
  late final StreamSubscription _loginSub;
  late final StreamSubscription _usageSub;

  AccountsService()
  {
    _accountsSub = AccountsState.rustSignalStream.listen((pack) => provider.updateAccounts(pack.message));
    _loginSub = LoginState.rustSignalStream.listen((pack) => provider.updateLogin(pack.message));
    _usageSub = UsageState.rustSignalStream.listen((pack) => provider.updateUsage(pack.message));
  }

  ///Лимиты обновляются раз в минуту; это - сразу
  void refreshUsage() => RefreshUsage().sendSignalToRust();

  ///Открывает браузер для входа; account - слот Google (1 или 2)
  void login(AuthProvider authProvider, {int account = 1})
  {
    LoginRequest(provider: authProvider, account: account).sendSignalToRust();
  }

  void cancelLogin() => CancelLogin().sendSignalToRust();

  void logout(AuthProvider authProvider, {int account = 1})
  {
    LogoutRequest(provider: authProvider, account: account).sendSignalToRust();
  }

  Future<void> dispose() async
  {
    await _accountsSub.cancel();
    await _loginSub.cancel();
    await _usageSub.cancel();
    provider.dispose();
  }
}
