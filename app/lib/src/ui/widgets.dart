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
import 'package:flutter/scheduler.dart';

import 'theme.dart';

/// A widget that rebuilds about ten times a second while [running], and only
/// then: a ticker that is stopped when the song is, and muted by the framework
/// when the screen is not visible.
class LiveClock extends StatefulWidget {
  /// Builds with [builder], and again on every tick while [running].
  const LiveClock({required this.running, required this.builder, super.key});

  /// Whether to tick.
  final bool running;

  /// Builds the content, reading the time it shows afresh each call.
  final WidgetBuilder builder;

  @override
  State<LiveClock> createState() => _LiveClockState();
}

class _LiveClockState extends State<LiveClock>
    with SingleTickerProviderStateMixin {
  static const Duration _interval = Duration(milliseconds: 100);

  late final Ticker _ticker = createTicker(_onTick);
  Duration _last = Duration.zero;

  @override
  void initState() {
    super.initState();
    _sync();
  }

  @override
  void didUpdateWidget(LiveClock oldWidget) {
    super.didUpdateWidget(oldWidget);
    _sync();
  }

  void _sync() {
    if (widget.running && !_ticker.isActive) {
      _last = Duration.zero;
      _ticker.start();
    } else if (!widget.running && _ticker.isActive) {
      _ticker.stop();
    }
  }

  void _onTick(Duration elapsed) {
    if (elapsed - _last >= _interval) {
      _last = elapsed;
      setState(() {});
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.builder(context);
}

/// A title that wraps to [maxLines] lines and shrinks, a step at a time,
/// before it would be cut off. Only at [minSize] does it truncate.
///
/// It measures against the width of the screen less [inset] on both sides
/// rather than asking its parent, so it can sit in a column that is measured
/// for its intrinsic height.
class FittedTitle extends StatelessWidget {
  /// A title in the display face.
  const FittedTitle(
    this.text, {
    this.inset = 18,
    this.maxSize = 34,
    this.minSize = 20,
    this.maxLines = 2,
    super.key,
  });

  /// The words.
  final String text;

  /// The space on each side of the title, within the screen's width.
  final double inset;

  /// The size to use if it fits.
  final double maxSize;

  /// The smallest size, below which the text is cut off.
  final double minSize;

  /// How many lines it may take.
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    final color = context.colors.text;
    final scaler = MediaQuery.textScalerOf(context);
    final direction = Directionality.of(context);
    final width = MediaQuery.sizeOf(context).width - 2 * inset;
    var size = maxSize;
    while (size > minSize && !_fits(text, size, width, scaler, direction)) {
      size -= 2;
    }
    size = size < minSize ? minSize : size;
    return Text(
      text,
      maxLines: maxLines,
      overflow: TextOverflow.ellipsis,
      style: AppText.songTitle(size).copyWith(color: color),
    );
  }

  bool _fits(
    String text,
    double size,
    double width,
    TextScaler scaler,
    TextDirection direction,
  ) {
    final style = AppText.songTitle(size);
    final lines = TextPainter(
      text: TextSpan(text: text, style: style),
      textDirection: direction,
      textScaler: scaler,
      maxLines: maxLines,
    )..layout(maxWidth: width);
    final exceeded = lines.didExceedMaxLines;
    lines.dispose();
    if (exceeded) {
      return false;
    }
    // A word wider than the line would be broken in the middle.
    for (final word in text.split(' ')) {
      final single = TextPainter(
        text: TextSpan(text: word, style: style),
        textDirection: direction,
        textScaler: scaler,
      )..layout();
      final wider = single.width > width;
      single.dispose();
      if (wider) {
        return false;
      }
    }
    return true;
  }
}

/// A large rounded button with the web UI's hard shadow. Disabled, it is
/// dimmed and does nothing.
class ChunkyButton extends StatelessWidget {
  /// A button with [label] (or [child]) that calls [onPressed] when tapped.
  const ChunkyButton({
    required this.onPressed,
    required this.semanticLabel,
    this.label,
    this.child,
    this.fill,
    this.foreground,
    this.borderColor,
    this.height = 64,
    this.fontSize = 20,
    this.shadow = false,
    super.key,
  });

  /// Called on a tap; null disables the button.
  final VoidCallback? onPressed;

  /// What a screen reader says.
  final String semanticLabel;

  /// Text on the button, in the display face.
  final String? label;

  /// Content in place of [label].
  final Widget? child;

  /// The button's fill; the surface colour by default.
  final Color? fill;

  /// The colour of the content.
  final Color? foreground;

  /// The colour of the border.
  final Color? borderColor;

  /// The button's height before text scaling.
  final double height;

  /// The label's size before text scaling.
  final double fontSize;

  /// Whether to draw the hard shadow.
  final bool shadow;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final enabled = onPressed != null;
    final color = foreground ?? colors.text;
    final content =
        child ??
        Text(
          label!,
          style: AppText.button(fontSize).copyWith(color: color),
          textAlign: TextAlign.center,
        );
    return Semantics(
      button: true,
      enabled: enabled,
      label: semanticLabel,
      excludeSemantics: true,
      onTap: onPressed,
      child: Opacity(
        opacity: enabled ? 1 : 0.35,
        child: Container(
          constraints: BoxConstraints(minHeight: height),
          decoration: BoxDecoration(
            color: fill ?? colors.surface,
            borderRadius: BorderRadius.circular(18),
            border: Border.all(color: borderColor ?? colors.line, width: 2),
            boxShadow: shadow
                ? [BoxShadow(color: colors.shadow, offset: const Offset(4, 4))]
                : null,
          ),
          child: Material(
            type: MaterialType.transparency,
            borderRadius: BorderRadius.circular(16),
            child: InkWell(
              borderRadius: BorderRadius.circular(16),
              onTap: onPressed,
              child: Center(
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: content,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A card-like row background: a surface with rounded corners and an optional
/// coloured bar down its left edge.
class RowCard extends StatelessWidget {
  /// A row with [child] inside.
  const RowCard({
    required this.child,
    this.tint,
    this.mark,
    this.onTap,
    this.onLongPress,
    this.semanticLabel,
    this.semanticHint,
    this.enabled = true,
    this.padding = const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
    super.key,
  });

  /// Space between the card's edge and its content.
  final EdgeInsets padding;

  /// The row's content.
  final Widget child;

  /// A fill that replaces the surface colour, for the current row.
  final Color? tint;

  /// The colour of the bar down the left edge.
  final Color? mark;

  /// Called on a tap; null makes the row inert.
  final VoidCallback? onTap;

  /// Called on a long press.
  final VoidCallback? onLongPress;

  /// What a screen reader says, in place of the content's own text.
  final String? semanticLabel;

  /// What a screen reader says the row will do.
  final String? semanticHint;

  /// Whether the row looks usable. An inert row is dimmed.
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final card = DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: colors.line.withAlpha(120)),
      ),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: tint ?? Colors.transparent,
          borderRadius: BorderRadius.circular(12),
          border: mark == null
              ? null
              : Border(left: BorderSide(color: mark!, width: 3)),
        ),
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(
            borderRadius: BorderRadius.circular(12),
            onTap: onTap,
            onLongPress: onLongPress,
            child: Padding(padding: padding, child: child),
          ),
        ),
      ),
    );
    return Semantics(
      container: true,
      button: onTap != null || onLongPress != null,
      enabled: onTap != null || onLongPress != null,
      label: semanticLabel,
      hint: semanticHint,
      excludeSemantics: semanticLabel != null,
      onTap: onTap,
      onLongPress: onLongPress,
      child: Opacity(opacity: enabled ? 1 : 0.5, child: card),
    );
  }
}

/// A note in a box: help, a hint, or an explanation.
class NoteBox extends StatelessWidget {
  /// A note with [child] in it.
  const NoteBox({required this.child, this.tint, super.key});

  /// The note's content.
  final Widget child;

  /// A fill in place of the surface colour.
  final Color? tint;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: tint ?? colors.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: colors.line.withAlpha(120)),
      ),
      child: DefaultTextStyle(
        style: AppText.small.copyWith(color: colors.muted, height: 1.45),
        child: child,
      ),
    );
  }
}

/// Lays [children] out in a column that fills the screen when there is room
/// and scrolls when there is not, so large text never overflows. Put an
/// [Spacer] among the children to push what follows to the bottom.
class ScrollingColumn extends StatelessWidget {
  /// A column of [children].
  const ScrollingColumn({
    required this.children,
    this.padding = const EdgeInsets.fromLTRB(18, 6, 18, 14),
    super.key,
  });

  /// The content, top to bottom.
  final List<Widget> children;

  /// Space around the content.
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        return SingleChildScrollView(
          padding: padding,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              minHeight: constraints.maxHeight - padding.vertical,
            ),
            child: IntrinsicHeight(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: children,
              ),
            ),
          ),
        );
      },
    );
  }
}
