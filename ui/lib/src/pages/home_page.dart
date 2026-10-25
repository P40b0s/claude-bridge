import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/controls/flag.dart';
import 'package:claude_bridge_ui/src/controls/section_card.dart';
import 'package:claude_bridge_ui/src/format.dart';
import 'package:claude_bridge_ui/src/providers/proxy_provider.dart';
import 'package:claude_bridge_ui/src/services.dart';
import 'package:flutter/material.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

///Главная: прокси, подключение Claude Code и модель для всех сессий
class HomePage extends StatelessWidget
{
  const HomePage({super.key});

  @override
  Widget build(BuildContext context)
  {
    return ListView(
      padding: const EdgeInsets.all(24),
      children: const [
        LocationCard(),
        ProxyCard(),
        ModelCard(),
      ],
    );
  }
}

///Откуда сейчас уходит трафик: IP, страна и разрешено ли из неё проксирование
class LocationCard extends StatelessWidget
{
  const LocationCard({super.key});

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
        final hasLocation = state?.countryCode != null;
        return SectionCard(
          title: tr('Местоположение', 'Location'),
          icon: Icons.public,
          trailing: IconButton(
            tooltip: tr('Обновить', 'Refresh'),
            onPressed: state == null || state.checking ? null : service.refresh,
            icon: const Icon(Icons.refresh),
          ),
          child: state == null || (!hasLocation && state.error == null)
              ? const LinearProgressIndicator()
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    if (hasLocation)
                      Row(
                        children: [
                          CountryFlagIcon(code: state.countryCode!, width: 48),
                          const SizedBox(width: 16),
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text('${countryName(state.countryCode!, state.country ?? '')} (${state.countryCode})', style: theme.textTheme.titleMedium),
                                SelectableText('IP ${state.ip}'),
                              ],
                            ),
                          ),
                        ],
                      ),
                    if (state.error != null)
                      Padding(
                        padding: EdgeInsets.only(top: hasLocation ? 8 : 0),
                        child: Text(state.error!, style: TextStyle(color: theme.colorScheme.error)),
                      ),
                    if (state.enabled)
                      Padding(
                        padding: const EdgeInsets.only(top: 12),
                        child: Row(
                          children: [
                            Icon(
                              geo.blocking ? Icons.block : Icons.check_circle_outline,
                              size: 18,
                              color: geo.blocking ? theme.colorScheme.error : Colors.green,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                geo.blocking
                                    ? (hasLocation
                                        ? tr('Страны нет в списке разрешённых: запросы через прокси блокируются.', 'The country is not allowed: requests through the proxy are blocked.')
                                        : tr('Страна не определена: запросы через прокси блокируются.', 'The country is unknown: requests through the proxy are blocked.'))
                                    : tr('Страна в списке разрешённых: проксирование работает.', 'The country is allowed: proxying works.'),
                              ),
                            ),
                          ],
                        ),
                      ),
                  ],
                ),
        );
      },
    );
  }
}

class ProxyCard extends StatelessWidget
{
  const ProxyCard({super.key});

  @override
  Widget build(BuildContext context)
  {
    final service = context.appServices.proxyService;
    return ListenableBuilder(
      listenable: service.provider,
      builder: (context, _)
      {
        final state = service.provider.state;
        final running = service.provider.running;
        final busy = service.provider.busy || state == null;
        return SectionCard(
          title: tr('Прокси', 'Proxy'),
          icon: Icons.hub_outlined,
          trailing: Wrap(
            spacing: 8,
            children: [
              if (!running)
                FilledButton.icon(
                  onPressed: busy || state.cliPath == null ? null : service.start,
                  icon: const Icon(Icons.play_arrow),
                  label: Text(tr(tr('Запустить', 'Start'), 'Start')),
                ),
              if (running) ...[
                OutlinedButton.icon(
                  onPressed: busy ? null : service.restart,
                  icon: const Icon(Icons.restart_alt),
                  label: Text(tr(tr('Перезапустить', 'Restart'), 'Restart')),
                ),
                OutlinedButton.icon(
                  onPressed: busy ? null : service.stop,
                  icon: const Icon(Icons.stop),
                  label: Text(tr(tr('Остановить', 'Stop'), 'Stop')),
                ),
              ],
            ],
          ),
          child: state == null
              ? const LinearProgressIndicator()
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _Line(tr('Адрес', 'Address'), 'http://127.0.0.1:${state.port}'),
                    _Line(tr('Состояние', 'State'), running ? tr('работает, PID ${state.pid}, ${formatUptime(state.startedAt ?? 0)}', 'running, PID ${state.pid}, ${formatUptime(state.startedAt ?? 0)}') : tr('остановлен', 'stopped')),
                    if (running) _Line(tr('Последняя модель', 'Last model'), describeModel(state.active)),
                    _Line('CLI', state.cliPath ?? tr('не найден: укажите путь к claude-bridge в настройках', 'not found: set the path to claude-bridge in the settings')),
                    const Divider(height: 24),
                    SwitchListTile(
                      contentPadding: EdgeInsets.zero,
                      value: state.autostart,
                      onChanged: (v) => context.appServices.settingsService.save({'CLAUDE_BRIDGE_AUTOSTART': v ? '' : '0'}),
                      title: Text(tr(tr('Запускать прокси вместе с приложением', 'Start the proxy with the app'), 'Start the proxy with the app')),
                      subtitle: Text(tr(tr('Прокси работает в фоне и после закрытия приложения; остановить его можно здесь или claude-bridge stop.', 'The proxy keeps running in the background after the app closes; stop it here or with claude-bridge stop.'), 'The proxy keeps running in the background after the app closes; stop it here or with claude-bridge stop.')),
                    ),
                    SwitchListTile(
                      contentPadding: EdgeInsets.zero,
                      value: state.claudeCodeConnected,
                      onChanged: (v) => service.connectClaudeCode(v),
                      title: Text(tr(tr('Любой claude через прокси', 'Any claude through the proxy'), 'Any claude through the proxy')),
                      subtitle: Text(_connectionHint(state, running)),
                    ),
                  ],
                ),
        );
      },
    );
  }
}

String _connectionHint(ProxyState state, bool running)
{
  if (state.claudeCodeConnected)
  {
    final warning = running ? '' : tr(' Прокси остановлен: пока он не запущен, claude не будет отвечать.', ' The proxy is stopped: claude will not answer until it runs.');
    return tr('Адрес прокси прописан в ${state.claudeCodeSettings}, так что через него идёт и обычный claude, и расширения для IDE.$warning',
        'The proxy address is in ${state.claudeCodeSettings}, so plain claude and IDE extensions go through it too.$warning');
  }
  if (state.claudeCodeOtherUrl != null)
  {
    return tr('Сейчас в ${state.claudeCodeSettings} указан другой адрес: ${state.claudeCodeOtherUrl}. Включение заменит его.', '${state.claudeCodeSettings} points at another address: ${state.claudeCodeOtherUrl}. Turning this on replaces it.');
  }
  return tr('Выключено: через прокси идут только сессии, запущенные через claude-bridge codex, gemini и т. д. Включение пропишет адрес прокси в ${state.claudeCodeSettings}.',
      'Off: only sessions started with claude-bridge codex, gemini etc. use the proxy. Turning this on writes the proxy address to ${state.claudeCodeSettings}.');
}

class _Line extends StatelessWidget
{
  const _Line(this.label, this.value);
  final String label;
  final String value;

  @override
  Widget build(BuildContext context)
  {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(width: 160, child: Text(label, style: TextStyle(color: Theme.of(context).hintColor))),
          Expanded(child: SelectableText(value)),
        ],
      ),
    );
  }
}

///Переключение модели всех сессий на лету
class ModelCard extends StatefulWidget
{
  const ModelCard({super.key});

  @override
  State<ModelCard> createState() => _ModelCardState();
}

class _ModelCardState extends State<ModelCard>
{
  final _custom = TextEditingController();
  String? _preset = 'opus';
  String? _effort;

  @override
  void dispose()
  {
    _custom.dispose();
    super.dispose();
  }

  String? get _spec
  {
    final base = _custom.text.trim().isNotEmpty ? _custom.text.trim() : _preset;
    if (base == null)
    {
      return null;
    }
    return _effort == null || base.contains('@') ? base : '$base@$_effort';
  }

  @override
  Widget build(BuildContext context)
  {
    final proxy = context.appServices.proxyService;
    final settings = context.appServices.settingsService.provider;
    return ListenableBuilder(
      listenable: Listenable.merge([proxy.provider, settings]),
      builder: (context, _)
      {
        final running = proxy.provider.running;
        final pinned = proxy.provider.state?.pinned;
        final presets = settings.presets.where((p) => settings.providerEnabled(p.provider)).toList();
        //Пресет отключённого провайдера не может остаться выбранным
        if (_custom.text.trim().isEmpty && !presets.any((p) => p.alias == _preset))
        {
          _preset = presets.firstOrNull?.alias;
        }
        return SectionCard(
          title: tr('Модель для всех сессий', 'Model for all sessions'),
          icon: Icons.swap_horiz,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                pinned == null
                    ? tr('Сейчас каждая сессия использует модель, выбранную в ней через /model.', 'Each session uses the model picked in it with /model.')
                    : tr('Все запросы идут на ${describeModel(pinned)}. /model в Claude Code игнорируется, пока модель закреплена.', 'All requests go to ${describeModel(pinned)}. /model in Claude Code is ignored while a model is pinned.'),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 12,
                runSpacing: 12,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  SizedBox(
                    width: 320,
                    child: DropdownButtonFormField<String>(
                      key: ValueKey(presets.map((p) => p.alias).join(',')),
                      initialValue: _preset,
                      isExpanded: true,
                      decoration: InputDecoration(labelText: tr('Модель', 'Model')),
                      items: [
                        for (final p in presets)
                          DropdownMenuItem(value: p.alias, child: Text('${p.alias} — ${p.description}', overflow: TextOverflow.ellipsis)),
                      ],
                      onChanged: (v) => setState(() => _preset = v),
                    ),
                  ),
                  SizedBox(
                    width: 260,
                    child: TextField(
                      controller: _custom,
                      decoration: InputDecoration(labelText: tr('или своя', 'or your own'), hintText: 'openai:gpt-5.5, terra@max'),
                      onChanged: (_) => setState(() {}),
                    ),
                  ),
                  SizedBox(
                    width: 170,
                    child: DropdownButtonFormField<String?>(
                      initialValue: _effort,
                      isExpanded: true,
                      decoration: InputDecoration(labelText: tr('Усилие', 'Effort')),
                      items: [
                        DropdownMenuItem(value: null, child: Text(tr('по умолчанию', 'default'), overflow: TextOverflow.ellipsis)),
                        for (final e in settings.efforts) DropdownMenuItem(value: e, child: Text(e)),
                      ],
                      onChanged: (v) => setState(() => _effort = v),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                children: [
                  FilledButton.icon(
                    onPressed: running && _spec != null ? () => proxy.pinModel(_spec!) : null,
                    icon: const Icon(Icons.push_pin_outlined),
                    label: Text(_spec == null ? tr('Закрепить', 'Pin') : tr('Закрепить $_spec', 'Pin $_spec')),
                  ),
                  OutlinedButton.icon(
                    onPressed: running && pinned != null ? proxy.unpinModel : null,
                    icon: const Icon(Icons.undo),
                    label: Text(tr(tr('Вернуть /model', 'Back to /model'), 'Back to /model')),
                  ),
                ],
              ),
              if (!running)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(tr('Запустите прокси, чтобы переключать модель.', 'Start the proxy to switch the model.'), style: TextStyle(color: Theme.of(context).hintColor)),
                ),
            ],
          ),
        );
      },
    );
  }
}
