import 'package:claude_bridge_ui/src/bindings/bindings.dart';
import 'package:flutter/foundation.dart';

/// Severity level for a toast / error entry.
//enum ErrorSeverity { info, warning, error, success }

/// A single notification entry stored in [ErrorProvider] history.
class ErrorEntry {
  ErrorEntry({
    required this.id,
    required this.message,
    required this.severity,
    this.title,
    this.actionLabel,
    this.onAction,
  }) : timestamp = DateTime.now();

  final String id;
  final String? title;
  final String message;
  final ErrorSeverity severity;
  final DateTime timestamp;
  final String? actionLabel;
  final VoidCallback? onAction;
}

/// Provider holding the history of notifications / errors.
///
/// New entries are auto-shown as toasts by `ToastOverlay`. The full
/// history is available via [history] and can be displayed with
/// `RecentErrorsPanel`.
class ErrorProvider extends ChangeNotifier {
  ErrorProvider({this.maxHistory = 50});

  final int maxHistory;
  final List<ErrorEntry> _history = [];

  /// Unmodifiable view of the history (newest first).
  List<ErrorEntry> get history => List.unmodifiable(_history);

  /// Most recent entry, or null if history is empty.
  ErrorEntry? get latest => _history.isEmpty ? null : _history.first;

  /// Total number of stored entries.
  int get count => _history.length;

  /// Spawn a new notification. Will appear as a toast immediately and
  /// also be added to history.
  ///
  /// Backwards-compatible with the simple `spawnError('msg')` form —
  /// extra params are optional.
  void spawnError(
    String message, {
    String? title,
    ErrorSeverity severity = ErrorSeverity.error,
    String? actionLabel,
    VoidCallback? onAction,
  }) {
    final entry = ErrorEntry(
      id: DateTime.now().microsecondsSinceEpoch.toString(),
      title: title,
      message: message,
      severity: severity,
      actionLabel: actionLabel,
      onAction: onAction,
    );
    _history.insert(0, entry);
    if (_history.length > maxHistory) {
      _history.removeRange(maxHistory, _history.length);
    }
    notifyListeners();
  }

  void clearHistory() {
    _history.clear();
    notifyListeners();
  }
}