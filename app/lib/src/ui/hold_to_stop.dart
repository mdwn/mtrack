// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'theme.dart';

/// How long the Stop button has to be held.
const Duration holdToStopDuration = Duration(milliseconds: 600);

/// The Stop button: press and hold to stop, so a brushed screen cannot end a
/// song.
///
/// Holding fills the button over [holdToStopDuration]. When the fill is full
/// [onStop] is called, with a haptic. Letting go early starts over and calls
/// [onEarlyRelease], which the screen uses to show the hint. A tap is an early
/// release.
///
/// For a screen reader the button has a long-press action that stops at once,
/// and a tap that gives the hint.
class HoldToStop extends StatefulWidget {
  /// A Stop button.
  const HoldToStop({
    required this.onStop,
    required this.onEarlyRelease,
    this.enabled = true,
    this.height = 108,
    super.key,
  });

  /// Called once, when the hold completes.
  final VoidCallback onStop;

  /// Called when the finger lifts before the hold completes.
  final VoidCallback onEarlyRelease;

  /// False dims the button and ignores presses.
  final bool enabled;

  /// The button's height before text scaling.
  final double height;

  @override
  State<HoldToStop> createState() => _HoldToStopState();
}

class _HoldToStopState extends State<HoldToStop>
    with SingleTickerProviderStateMixin {
  late final AnimationController _fill = AnimationController(
    vsync: this,
    duration: holdToStopDuration,
  )..addStatusListener(_onStatus);

  bool _fired = false;

  void _onStatus(AnimationStatus status) {
    if (status == AnimationStatus.completed && !_fired) {
      _fired = true;
      HapticFeedback.heavyImpact();
      widget.onStop();
    }
  }

  void _down() {
    if (!widget.enabled) {
      return;
    }
    _fired = false;
    _fill.forward(from: 0);
  }

  void _up() {
    // The song stopping replaces this button with Play while the finger is
    // still down; its lift arrives after the button is gone.
    if (!mounted) {
      return;
    }
    if (_fired) {
      _fill.value = 0;
      return;
    }
    final started = _fill.isAnimating || _fill.value > 0;
    _fill.value = 0;
    if (started) {
      widget.onEarlyRelease();
    }
  }

  @override
  void didUpdateWidget(HoldToStop oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!widget.enabled && _fill.value > 0 && !_fired) {
      _fill.value = 0;
    }
  }

  @override
  void dispose() {
    _fill.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final radius = BorderRadius.circular(18);
    return Semantics(
      button: true,
      enabled: widget.enabled,
      label: 'Stop',
      hint: 'Press and hold to stop',
      excludeSemantics: true,
      onTap: widget.enabled ? widget.onEarlyRelease : null,
      onLongPress: widget.enabled ? widget.onStop : null,
      child: Opacity(
        opacity: widget.enabled ? 1 : 0.35,
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerDown: (_) => _down(),
          onPointerUp: (_) => _up(),
          onPointerCancel: (_) => _up(),
          child: Container(
            constraints: BoxConstraints(minHeight: widget.height),
            decoration: BoxDecoration(
              color: colors.stop,
              borderRadius: radius,
              border: Border.all(color: colors.shadow, width: 2),
              boxShadow: [
                BoxShadow(color: colors.shadow, offset: const Offset(4, 4)),
              ],
            ),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(16),
              child: Stack(
                alignment: Alignment.center,
                children: [
                  Positioned.fill(
                    child: AnimatedBuilder(
                      animation: _fill,
                      builder: (context, _) => Align(
                        alignment: Alignment.centerLeft,
                        child: FractionallySizedBox(
                          widthFactor: _fill.value,
                          heightFactor: 1,
                          child: ColoredBox(color: colors.stopFill),
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(
                      'STOP',
                      style: AppText.button(26).copyWith(color: colors.onStop),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
