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

/// How long a press-and-hold button has to be held.
const Duration holdToStopDuration = Duration(milliseconds: 600);

/// The Stop button: press and hold to stop, so a brushed screen cannot end a
/// song.
///
/// A [HoldButton] in the Stop button's colours. For a screen reader it has a
/// long-press action that stops at once, and a tap that gives the hint.
class HoldToStop extends StatelessWidget {
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
  Widget build(BuildContext context) {
    final colors = context.colors;
    return HoldButton(
      label: 'STOP',
      fontSize: 26,
      semanticLabel: 'Stop',
      semanticHint: 'Press and hold to stop',
      onComplete: onStop,
      onEarlyRelease: onEarlyRelease,
      enabled: enabled,
      height: height,
      fill: colors.stop,
      progress: colors.stopFill,
      foreground: colors.onStop,
      borderColor: colors.shadow,
      shadow: true,
    );
  }
}

/// A button that has to be held: for what a brushed screen must not do.
///
/// Holding fills the button over [holdToStopDuration]. When the fill is full
/// [onComplete] is called, with a haptic. Letting go early starts over and
/// calls [onEarlyRelease], which the screen uses to show the hint. A tap is
/// an early release.
///
/// For a screen reader the button has a long-press action that completes at
/// once, and a tap that gives the hint.
class HoldButton extends StatefulWidget {
  /// A button that says [label].
  const HoldButton({
    required this.label,
    required this.semanticLabel,
    required this.semanticHint,
    required this.onComplete,
    required this.onEarlyRelease,
    required this.fill,
    required this.progress,
    required this.foreground,
    required this.borderColor,
    this.fontSize = 26,
    this.enabled = true,
    this.height = 108,
    this.shadow = false,
    super.key,
  });

  /// The text on the button.
  final String label;

  /// The label size before text scaling.
  final double fontSize;

  /// What a screen reader calls it.
  final String semanticLabel;

  /// What a screen reader says it takes.
  final String semanticHint;

  /// Called once, when the hold completes.
  final VoidCallback onComplete;

  /// Called when the finger lifts before the hold completes.
  final VoidCallback onEarlyRelease;

  /// False dims the button and ignores presses.
  final bool enabled;

  /// The button's height before text scaling.
  final double height;

  /// The button's fill.
  final Color fill;

  /// The colour that grows over the fill while it is held.
  final Color progress;

  /// The colour of the label.
  final Color foreground;

  /// The colour of the border.
  final Color borderColor;

  /// Whether to draw the hard shadow.
  final bool shadow;

  @override
  State<HoldButton> createState() => _HoldButtonState();
}

class _HoldButtonState extends State<HoldButton>
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
      widget.onComplete();
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
    // What the hold did can replace this button while the finger is still
    // down; its lift arrives after the button is gone.
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
  void didUpdateWidget(HoldButton oldWidget) {
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
    final radius = BorderRadius.circular(widget.height < 80 ? 14 : 18);
    return Semantics(
      button: true,
      enabled: widget.enabled,
      label: widget.semanticLabel,
      hint: widget.semanticHint,
      excludeSemantics: true,
      onTap: widget.enabled ? widget.onEarlyRelease : null,
      onLongPress: widget.enabled ? widget.onComplete : null,
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
              color: widget.fill,
              borderRadius: radius,
              border: Border.all(color: widget.borderColor, width: 2),
              boxShadow: widget.shadow
                  ? [
                      BoxShadow(
                        color: colors.shadow,
                        offset: const Offset(4, 4),
                      ),
                    ]
                  : null,
            ),
            child: ClipRRect(
              borderRadius: radius - const BorderRadius.all(Radius.circular(2)),
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
                          child: ColoredBox(color: widget.progress),
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(
                      widget.label,
                      textAlign: TextAlign.center,
                      style: AppText.button(widget.fontSize)
                          .copyWith(color: widget.foreground),
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
