import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/flag.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/providers/geo_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Конструктор маршрутов: выходы (напрямую, SSH-сервер, прокси), выход и разрешённые страны
///для каждого провайдера
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
        return ListView(
          padding: const EdgeInsets.all(24),
          children: [
            SectionCard(
              title: tr('Ограничение по стране', 'Country restriction'),
              icon: Icons.shield_outlined,
              trailing: IconButton(
                tooltip: tr('Проверить сейчас', 'Check now'),
                onPressed: state.checking ? null : service.refresh,
                icon: const Icon(Icons.refresh),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    value: state.enabled,
                    onChanged: (v) => service.setPolicy(enabled: v, allowed: geo.allowed),
                    title: Text(tr('Проксировать только из разрешённых стран', 'Proxy only from allowed countries')),
                    subtitle: Text(
                      tr('Перед каждым запросом прокси спрашивает у хоста провайдера (Cloudflare /cdn-cgi/trace), из какой страны '
                          'тот видит запрос, через выбранный выход. Если страна не разрешена для этого провайдера или выход '
                          'недоступен, запрос не отправляется, а приложение показывает предупреждение.',
                          'Before every request the proxy asks the provider\'s host (Cloudflare /cdn-cgi/trace) which country it '
                          'sees the request from, through the chosen exit. If the country is not allowed for that provider or the '
                          'exit is down, nothing is sent and the app shows a warning.'),
                    ),
                  ),
                  if (state.checking) const LinearProgressIndicator(),
                  if (state.error != null)
                    Padding(
                      padding: const EdgeInsets.only(top: 8),
                      child: Text(state.error!, style: TextStyle(color: theme.colorScheme.error)),
                    ),
                ],
              ),
            ),
            for (final p in state.providers) _ProviderCard(provider: p, state: state),
            _ExitsCard(state: state),
            _SharedCountriesCard(state: state),
          ],
        );
      },
    );
  }
}

///Флаг, страна и IP, или ошибка проверки
class _Location extends StatelessWidget
{
  const _Location({required this.exit, this.prefix});
  final ExitInfo exit;
  final String? prefix;

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final code = exit.countryCode;
    if (code != null)
    {
      return Row(
        children: [
          CountryFlagIcon(code: code, width: 24),
          const SizedBox(width: 8),
          Flexible(
            child: SelectableText(
              '${prefix ?? ''}${countryName(code, exit.country ?? code)} ($code), IP ${exit.ip}',
            ),
          ),
        ],
      );
    }
    if (exit.error != null)
    {
      return Text(exit.error!, style: TextStyle(color: theme.colorScheme.error));
    }
    return Text(tr('ещё не проверено', 'not checked yet'), style: TextStyle(color: theme.hintColor));
  }
}

IconData _exitIcon(String kind) => switch (kind)
{
  'ssh' => Icons.dns_outlined,
  'proxy' => Icons.swap_horiz,
  _ => Icons.lan_outlined,
};

String _exitName(ExitInfo e) => e.kind == 'direct' ? tr('Напрямую', 'Direct') : e.label;

class _ProviderCard extends StatelessWidget
{
  const _ProviderCard({required this.provider, required this.state});
  final ProviderGeo provider;
  final GeoState state;

  @override
  Widget build(BuildContext context)
  {
    final service = context.appServices.geoService;
    final theme = Theme.of(context);
    final p = provider;
    final ok = GeoProvider.permits(p);
    final allowed = p.allowed.toSet();
    void save({String? exit, Set<String>? countries, bool? own})
    {
      final keepOwn = own ?? p.own;
      service.setProvider(
        p.family,
        exit: exit ?? p.exit.spec,
        allowed: keepOwn ? (countries ?? allowed) : null,
      );
    }

    final exitSpecs = state.exits.map((e) => e.spec).toSet();
    return SectionCard(
      title: familyName(p.family),
      icon: _exitIcon(p.exit.kind),
      trailing: !state.enabled
          ? null
          : Icon(
              ok == true ? Icons.check_circle_outline : Icons.block,
              color: ok == true ? Colors.green : theme.colorScheme.error,
            ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Text(tr('Выход:', 'Exit:')),
              const SizedBox(width: 12),
              Expanded(
                child: DropdownButton<String>(
                  isExpanded: true,
                  value: exitSpecs.contains(p.exit.spec) ? p.exit.spec : null,
                  hint: Text(p.exit.label),
                  items: [
                    for (final e in state.exits)
                      DropdownMenuItem(
                        value: e.spec,
                        child: Row(
                          children: [
                            Icon(_exitIcon(e.kind), size: 18),
                            const SizedBox(width: 8),
                            Flexible(child: Text(_exitName(e), overflow: TextOverflow.ellipsis)),
                            if (e.countryCode != null) ...[
                              const SizedBox(width: 8),
                              CountryFlagIcon(code: e.countryCode!, width: 20),
                            ],
                          ],
                        ),
                      ),
                  ],
                  onChanged: (v) => v == null ? null : save(exit: v),
                ),
              ),
            ],
          ),
          if (p.family == 'anthropic')
            Text(
              tr('Через этот же выход и с этими же странами идёт весь прочий трафик Claude Code: вход и токены, MCP, WebFetch, '
                  'запускаемые им команды.',
                  "Claude Code's other traffic takes this exit and these countries too: login and tokens, MCP, WebFetch, "
                  'the commands it runs.'),
              style: TextStyle(color: theme.hintColor),
            ),
          const SizedBox(height: 8),
          _Location(exit: p.exit, prefix: '${tr('видит', 'sees')} ${Uri.tryParse(p.origin)?.host ?? p.origin}: '),
          if (state.enabled && ok != true)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                ok == false
                    ? tr('Страна не разрешена: запросы к провайдеру отклоняются.', 'Country not allowed: requests to this provider are refused.')
                    : tr('Страна не определена: запросы к провайдеру отклоняются.', 'Country unknown: requests to this provider are refused.'),
                style: TextStyle(color: theme.colorScheme.error),
              ),
            ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            value: p.own,
            onChanged: (v) => save(own: v),
            title: Text(tr('Свой список стран', 'Own country list')),
            subtitle: Text(
              p.own
                  ? tr('Только отмеченные ниже страны.', 'Only the countries checked below.')
                  : tr('Общий список (внизу страницы).', 'The shared list (at the bottom of the page).'),
            ),
          ),
          if (p.own)
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final c in state.countries)
                  FilterChip(
                    avatar: CountryFlagIcon(code: c.code, width: 18),
                    label: Text(c.code),
                    tooltip: countryName(c.code, c.name),
                    selected: allowed.contains(c.code),
                    onSelected: (v) => save(countries: v ? {...allowed, c.code} : (Set.of(allowed)..remove(c.code))),
                  ),
              ],
            ),
          if (p.own && allowed.isEmpty && state.enabled)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                tr('Ни одна страна не отмечена: провайдер заблокирован полностью.', 'No country is checked: this provider is blocked entirely.'),
                style: TextStyle(color: theme.colorScheme.error),
              ),
            ),
        ],
      ),
    );
  }
}

class _ExitsCard extends StatefulWidget
{
  const _ExitsCard({required this.state});
  final GeoState state;

  @override
  State<_ExitsCard> createState() => _ExitsCardState();
}

class _ExitsCardState extends State<_ExitsCard>
{
  final _controller = TextEditingController();

  @override
  void dispose()
  {
    _controller.dispose();
    super.dispose();
  }

  List<String> get _specs => widget.state.exits.where((e) => e.kind != 'direct').map((e) => e.spec).toList();

  void _add()
  {
    final text = _controller.text.trim();
    if (text.isEmpty)
    {
      return;
    }
    context.appServices.geoService.setExits([..._specs, text]);
    _controller.clear();
  }

  @override
  Widget build(BuildContext context)
  {
    final theme = Theme.of(context);
    final state = widget.state;
    return SectionCard(
      title: tr('Выходы', 'Exits'),
      icon: Icons.alt_route,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final e in state.exits)
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(_exitIcon(e.kind)),
              title: Text(_exitName(e)),
              subtitle: _Location(exit: e),
              trailing: e.kind == 'direct'
                  ? null
                  : IconButton(
                      tooltip: tr('Удалить', 'Remove'),
                      icon: const Icon(Icons.delete_outline),
                      onPressed: () => context.appServices.geoService.setExits(_specs.where((s) => s != e.spec)),
                    ),
            ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _controller,
                  onSubmitted: (_) => _add(),
                  decoration: InputDecoration(
                    isDense: true,
                    border: const OutlineInputBorder(),
                    labelText: tr('Новый выход', 'New exit'),
                    hintText: 'ssh://root@203.0.113.7  ·  socks5://127.0.0.1:1080  ·  http://user:pass@host:3128',
                  ),
                ),
              ),
              const SizedBox(width: 8),
              FilledButton(onPressed: _add, child: Text(tr('Добавить', 'Add'))),
            ],
          ),
          const SizedBox(height: 8),
          Text(
            tr('Форматы:\n'
                '  ssh://[user@]host[:port] (порт 22 по умолчанию, работают алиасы из ~/.ssh/config)\n'
                '  socks5://[user:pass@]host[:port] (1080 по умолчанию; имена резолвятся на стороне прокси)\n'
                '  http://[user:pass@]host[:port]\n'
                'Логин и пароль прокси указываются прямо в адресе. Символы @ : / # ? % в них кодируйте: @ → %40, : → %3A, / → %2F, # → %23, ? → %3F, % → %25.',
                'Formats:\n'
                '  ssh://[user@]host[:port] (port 22 by default, ~/.ssh/config aliases work)\n'
                '  socks5://[user:pass@]host[:port] (1080 by default; names are resolved on the proxy side)\n'
                '  http://[user:pass@]host[:port]\n'
                'Proxy login and password go right into the address. Encode @ : / # ? % in them: @ → %40, : → %3A, / → %2F, # → %23, ? → %3F, % → %25.'),
            style: TextStyle(color: theme.hintColor),
          ),
          const SizedBox(height: 8),
          Text(
            tr('SSH: прокси сам держит туннель (ssh -D) и пускает через него запросы. На сервере нужен только sshd с входом по '
                'ключу: ключ должен работать без пароля (ssh-copy-id user@host, затем проверьте ssh user@host). '
                'Если туннель упал, запросы не уходят напрямую, а отклоняются.',
                'SSH: the proxy keeps the tunnel itself (ssh -D) and sends requests through it. The server only needs sshd '
                'with key login; the key must work without a password prompt (ssh-copy-id user@host, then try ssh user@host). '
                'If the tunnel is down, requests are refused, never sent direct.'),
            style: TextStyle(color: theme.hintColor),
          ),
          if (!state.proxyRunning && state.exits.any((e) => e.kind == 'ssh'))
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                tr('Прокси не запущен: SSH-выходы проверяются, когда он работает.', 'The proxy is not running: SSH exits are checked while it runs.'),
                style: TextStyle(color: theme.colorScheme.error),
              ),
            ),
        ],
      ),
    );
  }
}

class _SharedCountriesCard extends StatelessWidget
{
  const _SharedCountriesCard({required this.state});
  final GeoState state;

  @override
  Widget build(BuildContext context)
  {
    final service = context.appServices.geoService;
    final allowed = state.allowed.toSet();
    void save(Set<String> countries) => service.setPolicy(enabled: state.enabled, allowed: countries);
    return SectionCard(
      title: tr('Общий список стран', 'Shared country list'),
      icon: Icons.public,
      trailing: Wrap(
        spacing: 8,
        children: [
          TextButton(
            onPressed: allowed.length == state.countries.length ? null : () => save({for (final c in state.countries) c.code}),
            child: Text(tr('Отметить все', 'Check all')),
          ),
          TextButton(
            onPressed: allowed.isEmpty ? null : () => save(<String>{}),
            child: Text(tr('Снять все', 'Uncheck all')),
          ),
        ],
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            tr('Для провайдеров без своего списка.', 'For providers without their own list.'),
            style: TextStyle(color: Theme.of(context).hintColor),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final c in state.countries)
                FilterChip(
                  avatar: CountryFlagIcon(code: c.code, width: 18),
                  label: Text(countryName(c.code, c.name)),
                  selected: allowed.contains(c.code),
                  onSelected: (v) => save(v ? {...allowed, c.code} : (Set.of(allowed)..remove(c.code))),
                ),
            ],
          ),
        ],
      ),
    );
  }
}
