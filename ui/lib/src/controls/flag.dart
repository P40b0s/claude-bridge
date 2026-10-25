import 'package:country_flags/country_flags.dart';
import 'package:flutter/material.dart';

///Флаг страны по ISO-коду (рисуется из SVG, эмодзи-флаги есть не на всех платформах)
class CountryFlagIcon extends StatelessWidget
{
  const CountryFlagIcon({super.key, required this.code, this.width = 32});
  final String code;
  final double width;

  @override
  Widget build(BuildContext context)
  {
    return CountryFlag.fromCountryCode(
      code,
      theme: ImageTheme(width: width, height: width * 0.7, shape: const RoundedRectangle(4)),
    );
  }
}
