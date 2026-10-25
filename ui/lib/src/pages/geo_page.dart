import 'package:claude_bridge_ui/src/controls/flag.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Запрет проксирования, если текущий IP не из отмеченных стран
class GeoPage extends StatelessWidget
{
  const GeoPage({super.key});

  @override
  Widget build(BuildContext context)
  {
    final service = context.appServices.geoService;
    return ListenableBuilder(
      listenable: service.provider,
      builder: (context, _)
      {
        final geo = service.provider;
        final state = geo.state;
        final theme = Theme.of(context);
        if (state == null)
        {
          return const Padding(padding: EdgeInsets.all(24), child: LinearProgressIndicator());
        }
        final allowed = geo.allowed;
        void save({bool? enabled, Set<String>? countries}) =>
            service.setPolicy(enabled: enabled ?? state.enabled, allowed: countries ?? allowed);
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            SectionCard(
              title: tr('Ограничение по стране', 'Country restriction'),
              icon: Icons.shield_outlined,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    value: state.enabled,
                    onChanged: (v) => save(enabled: v),
                    title: Text(tr(tr('Проксировать только из выбранных стран', 'Proxy only from the selected countries'), 'Proxy only from the selected countries')),
                    subtitle: Text(
                      tr('Если страна текущего IP не отмечена (или определить её не удалось), прокси отвечает ошибкой и ничего не отправляет '
                          'провайдерам. Действует сразу, перезапуск не нужен.',
                          'If the current IP is in an unchecked country (or the country is unknown), the proxy answers with an error and sends '
                          'nothing to the providers. Applies immediately, no restart needed.'),
                    ),
                  ),
                  if (state.enabled && allowed.isEmpty)
                    Padding(
                      padding: const EdgeInsets.only(top: 8),
                      child: Text(tr('Ни одна страна не отмечена: проксирование заблокировано полностью.', 'No country is checked: proxying is blocked entirely.'), style: TextStyle(color: theme.colorScheme.error)),
                    ),
                ],
              ),
            ),
            SectionCard(
              title: tr('Разрешённые страны', 'Allowed countries'),
              icon: Icons.public,
              trailing: Wrap(
                spacing: 8,
                children: [
                  TextButton(
                    onPressed: allowed.length == state.countries.length ? null : () => save(countries: {for (final c in state.countries) c.code}),
                    child: Text(tr(tr('Отметить все', 'Check all'), 'Check all')),
                  ),
                  TextButton(
                    onPressed: allowed.isEmpty ? null : () => save(countries: <String>{}),
                    child: Text(tr(tr('Снять все', 'Uncheck all'), 'Uncheck all')),
                  ),
                ],
              ),
              child: Column(
                children: [
                  for (final country in state.countries)
                    CheckboxListTile(
                      contentPadding: EdgeInsets.zero,
                      controlAffinity: ListTileControlAffinity.leading,
                      value: allowed.contains(country.code),
                      onChanged: (v) => save(
                        countries: v == true ? {...allowed, country.code} : (Set.of(allowed)..remove(country.code)),
                      ),
                      secondary: CountryFlagIcon(code: country.code),
                      title: Text(countryName(country.code, country.name)),
                      subtitle: Text(country.code),
                      tileColor: country.code == state.countryCode ? theme.colorScheme.primary.withValues(alpha: 0.08) : null,
                    ),
                  if (state.countryCode != null)
                    Padding(
                      padding: const EdgeInsets.only(top: 8),
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Text('${tr('Сейчас', 'Now')}: ${countryName(state.countryCode!, state.country ?? '')} (${state.countryCode}), IP ${state.ip}', style: TextStyle(color: theme.hintColor)),
                      ),
                    ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}
