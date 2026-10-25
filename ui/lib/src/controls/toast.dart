import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:claude_bridge_ui/src/providers/error_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:claude_bridge_ui/src/i18n.dart';

// ─── Styling helpers ─────────────────────────────────────────────────────────

class _SeverityStyle {
  const _SeverityStyle({
    required this.icon,
    required this.accent,
    required this.background,
  });
  final IconData icon;
  final Color accent;
  final Color background;
}

_SeverityStyle _styleFor(
  ErrorSeverity s,
  ColorScheme scheme,
  Brightness brightness,
) {
  final dark = brightness == Brightness.dark;
  switch (s) {
    case ErrorSeverity.info:
      return _SeverityStyle(
        icon: Icons.info_outline,
        accent: scheme.primary,
        background:
            dark ? scheme.surfaceContainerHigh : scheme.primaryContainer,
      );
    case ErrorSeverity.warning:
      return _SeverityStyle(
        icon: Icons.warning_amber_rounded,
        accent: dark ? Colors.orange.shade300 : Colors.orange.shade800,
        background: dark
            ? Colors.orange.shade900.withValues(alpha: 0.35)
            : Colors.orange.shade50,
      );
    case ErrorSeverity.error:
      return _SeverityStyle(
        icon: Icons.error_outline,
        accent: scheme.error,
        background: dark
            ? scheme.errorContainer.withValues(alpha: 0.45)
            : scheme.errorContainer,
      );
    case ErrorSeverity.success:
      return _SeverityStyle(
        icon: Icons.check_circle_outline,
        accent: dark ? Colors.green.shade300 : Colors.green.shade700,
        background: dark
            ? Colors.green.shade900.withValues(alpha: 0.35)
            : Colors.green.shade50,
      );
  }
}

// ─── ToastOverlay ────────────────────────────────────────────────────────────

/// Toasts in the style of Sonner: compact cards stacked in the bottom-right
/// corner. Hovering fans the stack out and pauses the timers; a swipe to the
/// right dismisses a toast. Place it above the Navigator:
///
/// ```dart
/// MaterialApp(builder: (context, child) => ToastOverlay(child: child!))
/// ```
class ToastOverlay extends StatefulWidget {
  const ToastOverlay({super.key, required this.child, this.maxVisible = 3, this.width = 380});
  final Widget child;
  /// Cards shown at once; older ones leave as new ones arrive.
  final int maxVisible;
  final double width;

  @override
  State<ToastOverlay> createState() => _ToastOverlayState();
}

const _gap = 10.0;
const _peek = 12.0;
const _edge = 20.0;

Duration _durationFor(ErrorSeverity s) => switch (s) {
      ErrorSeverity.error => const Duration(seconds: 8),
      ErrorSeverity.warning => const Duration(seconds: 6),
      _ => const Duration(seconds: 4),
    };

class _ToastOverlayState extends State<ToastOverlay> {
  late ErrorProvider _provider;
  /// Oldest first, so the newest paints on top.
  final List<ErrorEntry> _toasts = [];
  final Set<String> _leaving = {};
  final Map<String, double> _heights = {};
  int _seenLength = 0;
  bool _expanded = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _provider = Provider.of<ErrorProvider>(context, listen: false);
    _seenLength = _provider.history.length;
    _provider.addListener(_onChange);
  }

  @override
  void dispose() {
    _provider.removeListener(_onChange);
    super.dispose();
  }

  void _onChange() {
    if (!mounted) return;
    final history = _provider.history;
    if (history.length <= _seenLength) {
      _seenLength = history.length; // e.g. after clearHistory()
      return;
    }
    final fresh = history.take(history.length - _seenLength).toList().reversed;
    _seenLength = history.length;
    setState(() {
      _toasts.addAll(fresh);
      // Oldest ones beyond the limit fade out rather than vanish.
      final active = _toasts.where((t) => !_leaving.contains(t.id)).toList();
      for (final t in active.take((active.length - widget.maxVisible).clamp(0, active.length))) {
        _leaving.add(t.id);
      }
    });
  }

  void _remove(String id) {
    if (!mounted) return;
    setState(() {
      _toasts.removeWhere((t) => t.id == id);
      _leaving.remove(id);
      _heights.remove(id);
      if (_toasts.isEmpty) _expanded = false;
    });
  }

  void _measured(String id, double height) {
    if (_heights[id] == height) return;
    setState(() => _heights[id] = height);
  }

  @override
  Widget build(BuildContext context) {
    final screen = MediaQuery.sizeOf(context);
    final width = (screen.width - 2 * _edge).clamp(0.0, widget.width);
    // Newest first for layout.
    final ordered = _toasts.reversed.toList();
    final front = ordered.isEmpty ? 0.0 : _heights[ordered.first.id] ?? 64;

    final bottoms = <String, double>{};
    var offset = 0.0;
    for (final (i, t) in ordered.indexed) {
      bottoms[t.id] = _expanded ? offset : i * _peek;
      offset += (_heights[t.id] ?? 64) + _gap;
    }
    final stackHeight = ordered.isEmpty
        ? 0.0
        : _expanded
            ? offset - _gap
            : front + (ordered.length - 1).clamp(0, 2) * _peek;

    return Stack(
      children: [
        widget.child,
        if (_toasts.isNotEmpty)
          Positioned(
            right: _edge,
            bottom: _edge,
            width: width,
            child: MouseRegion(
              onEnter: (_) => setState(() => _expanded = true),
              onExit: (_) => setState(() => _expanded = false),
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 250),
                curve: Curves.easeOutCubic,
                height: stackHeight,
                child: Stack(
                  clipBehavior: Clip.none,
                  children: [
                    for (final t in _toasts)
                      _positioned(t, ordered.indexOf(t), bottoms[t.id]!, front),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }

  Widget _positioned(ErrorEntry entry, int depth, double bottom, double frontHeight) {
    final behind = !_expanded && depth > 0;
    return AnimatedPositioned(
      key: ValueKey(entry.id),
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeOutCubic,
      left: 0,
      right: 0,
      bottom: bottom,
      child: AnimatedScale(
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeOutCubic,
        alignment: Alignment.bottomCenter,
        scale: behind ? 1 - 0.05 * depth : 1,
        child: AnimatedOpacity(
          duration: const Duration(milliseconds: 250),
          opacity: behind && depth > 2 ? 0 : 1,
          child: _ToastCard(
            entry: entry,
            leaving: _leaving.contains(entry.id),
            paused: _expanded,
            collapsedBehind: behind,
            collapsedHeight: frontHeight,
            onMeasured: (h) => _measured(entry.id, h),
            onDismiss: () => setState(() => _leaving.add(entry.id)),
            onGone: () => _remove(entry.id),
          ),
        ),
      ),
    );
  }
}

class _ToastCard extends StatefulWidget {
  const _ToastCard({
    required this.entry,
    required this.leaving,
    required this.paused,
    required this.collapsedBehind,
    required this.collapsedHeight,
    required this.onMeasured,
    required this.onDismiss,
    required this.onGone,
  });
  final ErrorEntry entry;
  /// Play the exit animation, then call [onGone].
  final bool leaving;
  /// The stack is hovered: timers wait.
  final bool paused;
  /// Behind the front card in the collapsed stack: only its edge shows.
  final bool collapsedBehind;
  final double collapsedHeight;
  final ValueChanged<double> onMeasured;
  final VoidCallback onDismiss;
  final VoidCallback onGone;

  @override
  State<_ToastCard> createState() => _ToastCardState();
}

class _ToastCardState extends State<_ToastCard> with TickerProviderStateMixin {
  late final AnimationController _life = AnimationController(vsync: this, duration: _durationFor(widget.entry.severity));
  late final AnimationController _presence = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 380),
    reverseDuration: const Duration(milliseconds: 220),
  );
  final GlobalKey _contentKey = GlobalKey();
  double _drag = 0;
  bool _hovered = false;

  @override
  void initState() {
    super.initState();
    _life.addStatusListener((s) {
      if (s == AnimationStatus.completed) widget.onDismiss();
    });
    _presence.forward();
    if (!widget.paused) _life.forward();
    WidgetsBinding.instance.addPostFrameCallback((_) => _measure());
  }

  @override
  void didUpdateWidget(covariant _ToastCard old) {
    super.didUpdateWidget(old);
    if (widget.leaving && !old.leaving) {
      _life.stop();
      _presence.reverse().then((_) {
        if (mounted) widget.onGone();
      });
    } else if (!widget.leaving && widget.paused != old.paused) {
      widget.paused ? _life.stop() : _life.forward();
    }
    WidgetsBinding.instance.addPostFrameCallback((_) => _measure());
  }

  void _measure() {
    if (!mounted) return;
    final box = _contentKey.currentContext?.findRenderObject() as RenderBox?;
    if (box != null && box.hasSize) widget.onMeasured(box.size.height);
  }

  @override
  void dispose() {
    _life.dispose();
    _presence.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final dark = theme.brightness == Brightness.dark;
    final style = _styleFor(widget.entry.severity, scheme, theme.brightness);
    final entry = widget.entry;

    final card = DecoratedBox(
      key: _contentKey,
      decoration: BoxDecoration(
        color: dark ? scheme.surfaceContainerHigh : scheme.surfaceContainerLowest,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: scheme.outlineVariant.withValues(alpha: dark ? 0.5 : 0.7)),
        boxShadow: [
          BoxShadow(color: Colors.black.withValues(alpha: dark ? 0.35 : 0.10), blurRadius: 24, offset: const Offset(0, 8)),
          BoxShadow(color: Colors.black.withValues(alpha: dark ? 0.2 : 0.05), blurRadius: 3, offset: const Offset(0, 1)),
        ],
      ),
      child: AnimatedOpacity(
        // Cards behind the front one show only their edge.
        duration: const Duration(milliseconds: 200),
        opacity: widget.collapsedBehind ? 0 : 1,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(14, 12, 8, 12),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 28,
                height: 28,
                decoration: BoxDecoration(color: style.accent.withValues(alpha: 0.14), shape: BoxShape.circle),
                child: Icon(style.icon, color: style.accent, size: 17),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      if (entry.title != null) ...[
                        Text(entry.title!, style: theme.textTheme.bodyMedium?.copyWith(fontWeight: FontWeight.w600)),
                        const SizedBox(height: 2),
                      ],
                      Text(
                        entry.message,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          height: 1.35,
                          color: entry.title == null ? scheme.onSurface : scheme.onSurfaceVariant,
                        ),
                      ),
                      if (entry.actionLabel != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 8),
                          child: FilledButton.tonal(
                            onPressed: () {
                              entry.onAction?.call();
                              widget.onDismiss();
                            },
                            style: FilledButton.styleFrom(
                              visualDensity: VisualDensity.compact,
                              padding: const EdgeInsets.symmetric(horizontal: 12),
                            ),
                            child: Text(entry.actionLabel!),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
              AnimatedOpacity(
                duration: const Duration(milliseconds: 150),
                opacity: _hovered ? 1 : 0,
                child: IconButton(
                  icon: const Icon(Icons.close, size: 16),
                  color: scheme.onSurfaceVariant,
                  onPressed: widget.onDismiss,
                  visualDensity: VisualDensity.compact,
                  constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
                  padding: EdgeInsets.zero,
                ),
              ),
            ],
          ),
        ),
      ),
    );

    // Behind the front card: as tall as the front one, content hidden.
    final sized = widget.collapsedBehind
        ? SizedBox(height: widget.collapsedHeight, child: ClipRect(child: OverflowBox(alignment: Alignment.topCenter, minHeight: 0, maxHeight: double.infinity, child: card)))
        : card;

    final enter = CurvedAnimation(parent: _presence, curve: Curves.easeOutBack, reverseCurve: Curves.easeInCubic);
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onHorizontalDragUpdate: (d) => setState(() => _drag = (_drag + d.delta.dx).clamp(-20.0, double.infinity)),
        onHorizontalDragEnd: (d) {
          if (_drag > 90 || d.velocity.pixelsPerSecond.dx > 700) {
            widget.onDismiss();
          } else {
            setState(() => _drag = 0);
          }
        },
        child: AnimatedBuilder(
          animation: enter,
          builder: (context, child) {
            final t = enter.value;
            // In from below; out to the right, following a swipe.
            final dy = _presence.status == AnimationStatus.reverse ? 0.0 : (1 - t) * 24;
            final dx = _presence.status == AnimationStatus.reverse ? _drag + (1 - t) * 60 : _drag;
            return Opacity(
              opacity: (t.clamp(0.0, 1.0) * (1 - (_drag / 250).clamp(0.0, 0.8))),
              child: Transform.translate(offset: Offset(dx, dy), child: child),
            );
          },
          child: sized,
        ),
      ),
    );
  }
}

// ─── RecentErrorsPanel ───────────────────────────────────────────────────────

/// Side panel listing [ErrorProvider.history], styled like the toasts.
/// Open via [RecentErrorsPanel.show].
class RecentErrorsPanel extends StatefulWidget {
  const RecentErrorsPanel({super.key});

  static Future<void> show(BuildContext context) {
    return showGeneralDialog(
      context: context,
      barrierDismissible: true,
      barrierLabel: tr('Закрыть', 'Close'),
      barrierColor: Colors.black.withValues(alpha: 0.25),
      transitionDuration: const Duration(milliseconds: 280),
      pageBuilder: (_, _, _) => const RecentErrorsPanel(),
      transitionBuilder: (context, animation, _, child) {
        final curved = CurvedAnimation(parent: animation, curve: Curves.easeOutCubic, reverseCurve: Curves.easeInCubic);
        return FadeTransition(
          opacity: curved,
          child: SlideTransition(
            position: Tween(begin: const Offset(0.08, 0), end: Offset.zero).animate(curved),
            child: child,
          ),
        );
      },
    );
  }

  @override
  State<RecentErrorsPanel> createState() => _RecentErrorsPanelState();
}

class _RecentErrorsPanelState extends State<RecentErrorsPanel> {
  /// null shows every severity.
  ErrorSeverity? _filter;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final dark = theme.brightness == Brightness.dark;
    final screen = MediaQuery.sizeOf(context);

    return Align(
      alignment: Alignment.topRight,
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Material(
          color: dark ? scheme.surfaceContainer : scheme.surfaceContainerLow,
          elevation: 12,
          shadowColor: Colors.black.withValues(alpha: 0.4),
          borderRadius: BorderRadius.circular(16),
          clipBehavior: Clip.antiAlias,
          child: SizedBox(
            width: (screen.width - 24).clamp(0.0, 440.0),
            height: screen.height - 24,
            child: Consumer<ErrorProvider>(
              builder: (context, provider, _) {
                final all = provider.history;
                final entries = _filter == null ? all : all.where((e) => e.severity == _filter).toList();
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _header(context, provider, all.length),
                    if (all.isNotEmpty) _filters(context, all),
                    Divider(height: 1, color: scheme.outlineVariant.withValues(alpha: 0.5)),
                    Expanded(
                      child: entries.isEmpty
                          ? _EmptyHistory(filtered: all.isNotEmpty)
                          : ListView.separated(
                              padding: const EdgeInsets.all(12),
                              itemCount: entries.length,
                              separatorBuilder: (_, _) => const SizedBox(height: 8),
                              itemBuilder: (_, i) => _NoticeTile(entry: entries[i]),
                            ),
                    ),
                  ],
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  Widget _header(BuildContext context, ErrorProvider provider, int count) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 14, 8, 10),
      child: Row(
        children: [
          Text(tr('Уведомления', 'Notifications'), style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w600)),
          if (count > 0) ...[
            const SizedBox(width: 8),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
              decoration: BoxDecoration(color: scheme.primary.withValues(alpha: 0.14), borderRadius: BorderRadius.circular(10)),
              child: Text('$count', style: theme.textTheme.labelSmall?.copyWith(color: scheme.primary, fontWeight: FontWeight.w700)),
            ),
          ],
          const Spacer(),
          if (count > 0)
            TextButton(
              onPressed: provider.clearHistory,
              style: TextButton.styleFrom(visualDensity: VisualDensity.compact, foregroundColor: scheme.onSurfaceVariant),
              child: Text(tr(tr('Очистить', 'Clear'), 'Clear')),
            ),
          IconButton(
            tooltip: tr('Закрыть', 'Close'),
            icon: const Icon(Icons.close, size: 20),
            color: scheme.onSurfaceVariant,
            onPressed: () => Navigator.of(context).pop(),
          ),
        ],
      ),
    );
  }

  Widget _filters(BuildContext context, List<ErrorEntry> all) {
    final scheme = Theme.of(context).colorScheme;
    final brightness = Theme.of(context).brightness;
    final counts = <ErrorSeverity, int>{};
    for (final e in all) {
      counts[e.severity] = (counts[e.severity] ?? 0) + 1;
    }
    Widget chip(ErrorSeverity? severity, String label) {
      final count = severity == null ? all.length : counts[severity] ?? 0;
      final accent = severity == null ? scheme.primary : _styleFor(severity, scheme, brightness).accent;
      final selected = _filter == severity;
      return ChoiceChip(
        label: Text('$label $count'),
        selected: selected,
        showCheckmark: false,
        visualDensity: VisualDensity.compact,
        avatar: severity == null ? null : Icon(_styleFor(severity, scheme, brightness).icon, size: 16, color: accent),
        selectedColor: accent.withValues(alpha: 0.16),
        side: BorderSide(color: selected ? accent.withValues(alpha: 0.5) : scheme.outlineVariant.withValues(alpha: 0.6)),
        onSelected: (_) => setState(() => _filter = severity),
      );
    }

    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 10),
      child: Wrap(
        spacing: 6,
        runSpacing: 6,
        children: [
          chip(null, tr('Все', 'All')),
          for (final (severity, label) in [
            (ErrorSeverity.error, tr('Ошибки', 'Errors')),
            (ErrorSeverity.warning, tr('Предупреждения', 'Warnings')),
            (ErrorSeverity.info, tr('Инфо', 'Info')),
            (ErrorSeverity.success, tr('Успех', 'Success')),
          ])
            if ((counts[severity] ?? 0) > 0) chip(severity, label),
        ],
      ),
    );
  }
}

class _EmptyHistory extends StatelessWidget {
  const _EmptyHistory({required this.filtered});
  /// History has entries, just none of the selected kind.
  final bool filtered;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 64,
            height: 64,
            decoration: BoxDecoration(color: scheme.primary.withValues(alpha: 0.10), shape: BoxShape.circle),
            child: Icon(filtered ? Icons.filter_alt_off_outlined : Icons.notifications_none_rounded, size: 30, color: scheme.primary),
          ),
          const SizedBox(height: 14),
          Text(filtered ? tr('Ничего не найдено', 'Nothing found') : tr('Пока тихо', 'All quiet'), style: theme.textTheme.titleSmall),
          const SizedBox(height: 4),
          Text(
            filtered ? tr('Уведомлений этого типа нет', 'No notifications of this type') : tr('Здесь появятся уведомления и ошибки', 'Notifications and errors will show up here'),
            style: theme.textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
          ),
        ],
      ),
    );
  }
}

class _NoticeTile extends StatefulWidget {
  const _NoticeTile({required this.entry});
  final ErrorEntry entry;

  @override
  State<_NoticeTile> createState() => _NoticeTileState();
}

class _NoticeTileState extends State<_NoticeTile> {
  bool _hovered = false;
  bool _copied = false;

  String _ago(DateTime t) {
    final d = DateTime.now().difference(t);
    if (d.inSeconds < 60) return tr('только что', 'just now');
    if (d.inMinutes < 60) return tr('${d.inMinutes} мин назад', '${d.inMinutes} min ago');
    if (d.inHours < 24) return tr('${d.inHours} ч назад', '${d.inHours} h ago');
    return tr('${d.inDays} д назад', '${d.inDays} d ago');
  }

  String _clock(DateTime t) => '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}:${t.second.toString().padLeft(2, '0')}';

  Future<void> _copy() async {
    final e = widget.entry;
    await Clipboard.setData(ClipboardData(text: [?e.title, e.message].join('\n')));
    if (!mounted) return;
    setState(() => _copied = true);
    await Future<void>.delayed(const Duration(milliseconds: 1500));
    if (mounted) setState(() => _copied = false);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final dark = theme.brightness == Brightness.dark;
    final entry = widget.entry;
    final style = _styleFor(entry.severity, scheme, theme.brightness);

    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 150),
        decoration: BoxDecoration(
          color: dark ? scheme.surfaceContainerHigh : scheme.surfaceContainerLowest,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: _hovered ? style.accent.withValues(alpha: 0.45) : scheme.outlineVariant.withValues(alpha: dark ? 0.5 : 0.7),
          ),
          boxShadow: [
            BoxShadow(color: Colors.black.withValues(alpha: dark ? 0.2 : 0.04), blurRadius: 3, offset: const Offset(0, 1)),
          ],
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(14, 12, 8, 12),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 28,
                height: 28,
                decoration: BoxDecoration(color: style.accent.withValues(alpha: 0.14), shape: BoxShape.circle),
                child: Icon(style.icon, color: style.accent, size: 17),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (entry.title != null) ...[
                        Text(entry.title!, style: theme.textTheme.bodyMedium?.copyWith(fontWeight: FontWeight.w600)),
                        const SizedBox(height: 2),
                      ],
                      SelectableText(
                        entry.message,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          height: 1.35,
                          color: entry.title == null ? scheme.onSurface : scheme.onSurfaceVariant,
                        ),
                      ),
                      const SizedBox(height: 6),
                      Tooltip(
                        message: _clock(entry.timestamp),
                        child: Text(_ago(entry.timestamp), style: theme.textTheme.labelSmall?.copyWith(color: scheme.onSurfaceVariant)),
                      ),
                      if (entry.actionLabel != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 8),
                          child: FilledButton.tonal(
                            onPressed: entry.onAction,
                            style: FilledButton.styleFrom(
                              visualDensity: VisualDensity.compact,
                              padding: const EdgeInsets.symmetric(horizontal: 12),
                            ),
                            child: Text(entry.actionLabel!),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
              AnimatedOpacity(
                duration: const Duration(milliseconds: 150),
                opacity: _hovered || _copied ? 1 : 0,
                child: IconButton(
                  tooltip: _copied ? tr('Скопировано', 'Copied') : tr('Копировать', 'Copy'),
                  icon: Icon(_copied ? Icons.check : Icons.copy_rounded, size: 16),
                  color: _copied ? style.accent : scheme.onSurfaceVariant,
                  onPressed: _copy,
                  visualDensity: VisualDensity.compact,
                  constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
                  padding: EdgeInsets.zero,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
