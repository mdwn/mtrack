import 'package:wakelock_plus/wakelock_plus.dart';

import '../app/ports.dart';

/// Keeps the screen on through `wakelock_plus`.
class WakelockScreenWake implements ScreenWake {
  /// A screen wake over the plugin.
  const WakelockScreenWake();

  @override
  Future<void> setEnabled(bool enabled) => WakelockPlus.toggle(enable: enabled);
}
