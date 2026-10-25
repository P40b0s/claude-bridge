import 'dart:io' as io;

import 'package:flutter/foundation.dart';

enum AppLang { ru, en }

///Ключ в ~/.claude-bridge/.env; Rust читает его же для своих уведомлений
const langKey = 'CLAUDE_BRIDGE_LANG';

///Язык системы, пока пользователь не выбрал свой
AppLang systemLang() => io.Platform.localeName.toLowerCase().startsWith('ru') ? AppLang.ru : AppLang.en;

AppLang? parseLang(String value) => switch (value.trim().toLowerCase())
{
  'ru' => AppLang.ru,
  'en' => AppLang.en,
  _ => null,
};

final ValueNotifier<AppLang> langNotifier = ValueNotifier(systemLang());

bool get isRussian => langNotifier.value == AppLang.ru;

///Текст на текущем языке. Строки остаются рядом с кодом, который их показывает
String tr(String ru, String en) => isRussian ? ru : en;

///1 день / 2 дня / 5 дней по-русски, 1 day / 2 days по-английски
String plural(int n, String one, String few, String many, String enOne, String enMany)
{
  if (!isRussian)
  {
    return n == 1 ? enOne : enMany;
  }
  final mod100 = n % 100;
  final mod10 = n % 10;
  if (mod100 >= 11 && mod100 <= 14)
  {
    return many;
  }
  if (mod10 == 1)
  {
    return one;
  }
  return mod10 >= 2 && mod10 <= 4 ? few : many;
}

const _countriesRu = {
  'DE': 'Германия',
  'NL': 'Нидерланды',
  'SE': 'Швеция',
  'EE': 'Эстония',
  'LV': 'Латвия',
  'GB': 'Великобритания',
  'FR': 'Франция',
  'LT': 'Литва',
  'CH': 'Швейцария',
  'PL': 'Польша',
  'US': 'США',
};

///Название страны на текущем языке; fallback - английское имя от Rust или сервиса геолокации
String countryName(String code, String fallback) => isRussian ? _countriesRu[code.toUpperCase()] ?? fallback : fallback;
