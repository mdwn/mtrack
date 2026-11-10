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

import '../app/levels.dart';
import '../app/session.dart';
import 'format.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// The Levels tab: a fader and a mute for each of the player's output tracks.
class LevelsScreen extends StatefulWidget {
  /// The Levels tab. [active] is true while the tab is the one on show.
  const LevelsScreen({required this.active, super.key});

  /// Whether this tab is showing.
  final bool active;

  @override
  State<LevelsScreen> createState() => _LevelsScreenState();
}

class _LevelsScreenState extends State<LevelsScreen> {
  Levels? _levels;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _levels = SessionScope.read(context).levels;
    _tell();
  }

  @override
  void didUpdateWidget(LevelsScreen old) {
    super.didUpdateWidget(old);
    if (widget.active != old.active) {
      _tell();
    }
  }

  /// Tells the levels whether the tab is on show, once the frame is built:
  /// that can start a fetch, which changes what is on screen.
  void _tell() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        _levels?.setVisible(widget.active);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final levels = session.levels;
    return ListenableBuilder(
      listenable: levels,
      builder: (context, _) => _body(context, session, levels),
    );
  }

  Widget _body(BuildContext context, Session session, Levels levels) {
    final colors = context.colors;
    final live = session.isReady;
    final tracks = levels.tracks;
    final Widget body;
    switch (levels.phase) {
      case LevelsPhase.tooOld:
        body = const _Message(text: levelsTooOldMessage);
      case LevelsPhase.failed:
        body = _Message(
          text: 'Could not get the levels from the player.',
          action: ChunkyButton(
            height: 52,
            fontSize: 16,
            label: 'Retry',
            semanticLabel: 'Retry',
            onPressed: live ? levels.retry : null,
          ),
        );
      case LevelsPhase.idle:
      case LevelsPhase.loading:
        body = _Message(text: live ? 'Loading the levels…' : '');
      case LevelsPhase.ready:
        body = tracks.isEmpty
            ? _Message(
                text: live ? 'This player has no output tracks to adjust.' : '',
              )
            : ListView.builder(
                padding: const EdgeInsets.fromLTRB(18, 0, 18, 8),
                itemCount: tracks.length,
                itemBuilder: (context, index) => Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: _TrackRow(
                    track: tracks[index],
                    levels: levels,
                    enabled: live,
                  ),
                ),
              );
    }
    final showing = levels.phase == LevelsPhase.ready && tracks.isNotEmpty;
    return Column(
      children: [
        PlayerBar(
          trailing: showing
              ? '${tracks.length} ${tracks.length == 1 ? 'track' : 'tracks'}'
              : null,
        ),
        const ReconnectBanner(),
        if (showing)
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 12, 18, 8),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                'OUTPUT TRACKS',
                style: AppText.eyebrow.copyWith(color: colors.dim),
              ),
            ),
          ),
        Expanded(
          child: Opacity(opacity: live ? 1 : 0.55, child: body),
        ),
        if (levels.phase == LevelsPhase.ready)
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 4, 18, 14),
            child: Text(
              'Levels are saved on the player. A mute lasts until the player '
              'restarts.',
              style: AppText.small.copyWith(color: colors.muted),
            ),
          ),
      ],
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.text, this.action});

  final String text;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              text,
              textAlign: TextAlign.center,
              style: AppText.body.copyWith(color: context.colors.muted),
            ),
            if (action != null) ...[
              const SizedBox(height: 16),
              SizedBox(width: 200, child: action),
            ],
          ],
        ),
      ),
    );
  }
}

class _TrackRow extends StatelessWidget {
  const _TrackRow({
    required this.track,
    required this.levels,
    required this.enabled,
  });

  final TrackLevel track;
  final Levels levels;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final muted = track.muted;
    final valueWidth = MediaQuery.textScalerOf(context).scale(12.5) * 4.9;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: colors.line.withAlpha(120)),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 2, 8, 4),
        child: Column(
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    track.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: AppText.bodyStrong.copyWith(
                      color: muted ? colors.dim : colors.text,
                    ),
                  ),
                ),
                _MuteButton(track: track, levels: levels, enabled: enabled),
              ],
            ),
            Row(
              children: [
                Expanded(
                  child: _Fader(track: track, levels: levels, enabled: enabled),
                ),
                // The value is a button: a fader is coarse under a thumb,
                // and typing sets a level to the tenth.
                Semantics(
                  button: true,
                  enabled: enabled,
                  label: 'Type the ${track.name} level',
                  excludeSemantics: true,
                  child: InkWell(
                    borderRadius: BorderRadius.circular(8),
                    onTap: enabled
                        ? () => _typeLevel(context, track, levels)
                        : null,
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(minHeight: 48),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        child: Center(
                          widthFactor: 1,
                          child: SizedBox(
                            width: valueWidth,
                            child: Text(
                              formatDb(track.gainDb),
                              maxLines: 1,
                              softWrap: false,
                              textAlign: TextAlign.end,
                              style: AppText.time(12.5, weight: 500).copyWith(
                                color: muted ? colors.dim : colors.muted,
                                decoration: enabled
                                    ? TextDecoration.underline
                                    : null,
                                decorationStyle: TextDecorationStyle.dotted,
                                decorationColor: colors.dim,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// Asks for a level in dB and sets the track to it.
Future<void> _typeLevel(
  BuildContext context,
  TrackLevel track,
  Levels levels,
) async {
  final value = await showDialog<double>(
    context: context,
    builder: (_) => _LevelDialog(track: track),
  );
  if (value != null) {
    levels.setTo(track.name, value);
  }
}

class _LevelDialog extends StatefulWidget {
  const _LevelDialog({required this.track});

  final TrackLevel track;

  @override
  State<_LevelDialog> createState() => _LevelDialogState();
}

class _LevelDialogState extends State<_LevelDialog> {
  late final TextEditingController _text = TextEditingController(
    text: widget.track.gainDb.toStringAsFixed(1),
  );
  bool _refused = false;

  @override
  void initState() {
    super.initState();
    // Selected, so typing replaces what is there.
    _text.selection = TextSelection(
      baseOffset: 0,
      extentOffset: _text.text.length,
    );
  }

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _submit() {
    final value = parseGain(_text.text);
    if (value == null) {
      setState(() => _refused = true);
      return;
    }
    Navigator.of(context).pop(value);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.track.name),
      content: TextField(
        controller: _text,
        autofocus: true,
        keyboardType: const TextInputType.numberWithOptions(
          signed: true,
          decimal: true,
        ),
        textInputAction: TextInputAction.done,
        onSubmitted: (_) => _submit(),
        onChanged: (_) {
          if (_refused) {
            setState(() => _refused = false);
          }
        },
        decoration: InputDecoration(
          labelText: 'Level',
          suffixText: 'dB',
          helperText: 'From ${formatDb(minGainDb)} to ${formatDb(maxGainDb)}',
          errorText: _refused
              ? 'A number from ${minGainDb.toInt()} to ${maxGainDb.toInt()}'
              : null,
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(onPressed: _submit, child: const Text('Set')),
      ],
    );
  }
}

class _MuteButton extends StatelessWidget {
  const _MuteButton({
    required this.track,
    required this.levels,
    required this.enabled,
  });

  final TrackLevel track;
  final Levels levels;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final muted = track.muted;
    final usable = enabled && !track.busy;
    final label = muted ? 'Unmute ${track.name}' : 'Mute ${track.name}';
    return Semantics(
      button: true,
      enabled: usable,
      toggled: muted,
      label: label,
      excludeSemantics: true,
      onTap: usable ? () => levels.toggleMute(track.name) : null,
      child: InkResponse(
        onTap: usable ? () => levels.toggleMute(track.name) : null,
        radius: 24,
        child: SizedBox(
          width: 48,
          height: 48,
          child: Center(
            child: Container(
              width: 40,
              height: 30,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: muted ? colors.stop : Colors.transparent,
                borderRadius: BorderRadius.circular(8),
                border: Border.all(
                  color: muted ? colors.stop : colors.line,
                  width: 1.5,
                ),
              ),
              child: Text(
                'M',
                style: AppText.button(12)
                    .copyWith(color: muted ? colors.onStop : colors.dim),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Fader extends StatelessWidget {
  const _Fader({
    required this.track,
    required this.levels,
    required this.enabled,
  });

  final TrackLevel track;
  final Levels levels;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final muted = track.muted;
    final name = track.name;
    final value = formatDb(track.gainDb);
    final step = track.gainDb;
    return Semantics(
      slider: true,
      enabled: enabled,
      label: '$name level',
      value: value,
      increasedValue: formatDb(step + 1),
      decreasedValue: formatDb(step - 1),
      excludeSemantics: true,
      onIncrease: enabled
          ? () => levels.endDrag(name, step + 1, snap: false)
          : null,
      onDecrease: enabled
          ? () => levels.endDrag(name, step - 1, snap: false)
          : null,
      child: SizedBox(
        height: 48,
        child: SliderTheme(
          data: SliderThemeData(
            trackHeight: 8,
            trackShape: _NotchedTrack(notch: colors.dim),
            activeTrackColor: muted ? colors.dim : colors.cyan,
            inactiveTrackColor: colors.raised,
            disabledActiveTrackColor: muted ? colors.dim : colors.cyan,
            disabledInactiveTrackColor: colors.raised,
            thumbColor: colors.text,
            disabledThumbColor: colors.text,
            thumbShape: const RoundSliderThumbShape(
              enabledThumbRadius: 11,
              disabledThumbRadius: 11,
              elevation: 0,
              pressedElevation: 0,
            ),
            overlayColor: colors.cyan.withAlpha(40),
            overlayShape: const RoundSliderOverlayShape(overlayRadius: 24),
          ),
          child: Slider(
            value: track.gainDb.clamp(minGainDb, maxGainDb),
            min: minGainDb,
            max: maxGainDb,
            onChangeStart: enabled ? (_) => levels.beginDrag(name) : null,
            onChanged: enabled ? (v) => levels.dragTo(name, v) : null,
            onChangeEnd: enabled ? (v) => levels.endDrag(name, v) : null,
          ),
        ),
      ),
    );
  }
}

/// The fader's track, with a notch at 0 dB.
class _NotchedTrack extends RoundedRectSliderTrackShape {
  const _NotchedTrack({required this.notch});

  final Color notch;

  @override
  void paint(
    PaintingContext context,
    Offset offset, {
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required Animation<double> enableAnimation,
    required TextDirection textDirection,
    required Offset thumbCenter,
    Offset? secondaryOffset,
    bool isDiscrete = false,
    bool isEnabled = false,
    double additionalActiveTrackHeight = 2,
  }) {
    super.paint(
      context,
      offset,
      parentBox: parentBox,
      sliderTheme: sliderTheme,
      enableAnimation: enableAnimation,
      textDirection: textDirection,
      thumbCenter: thumbCenter,
      secondaryOffset: secondaryOffset,
      isDiscrete: isDiscrete,
      isEnabled: isEnabled,
      additionalActiveTrackHeight: additionalActiveTrackHeight,
    );
    final track = getPreferredRect(
      parentBox: parentBox,
      offset: offset,
      sliderTheme: sliderTheme,
      isEnabled: isEnabled,
      isDiscrete: isDiscrete,
    );
    final x =
        track.left + track.width * (0 - minGainDb) / (maxGainDb - minGainDb);
    context.canvas.drawRect(
      Rect.fromCenter(
        center: Offset(x, track.center.dy),
        width: 2,
        height: track.height + 8,
      ),
      Paint()..color = notch,
    );
  }
}
