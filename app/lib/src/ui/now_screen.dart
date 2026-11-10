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
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../app/sections.dart';
import '../app/session.dart';
import 'format.dart';
import 'hold_to_stop.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// The Now tab: what is playing, where it is, and the transport.
class NowScreen extends StatefulWidget {
  /// The Now tab.
  const NowScreen({super.key});

  @override
  State<NowScreen> createState() => _NowScreenState();
}

class _NowScreenState extends State<NowScreen> {
  static const Duration _hintFlash = Duration(seconds: 2);

  bool _hintShown = false;
  Timer? _hintTimer;

  void _flashHint() {
    setState(() => _hintShown = true);
    _hintTimer?.cancel();
    _hintTimer = Timer(_hintFlash, () {
      if (mounted) {
        setState(() => _hintShown = false);
      }
    });
  }

  @override
  void dispose() {
    _hintTimer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final status = session.status;
    final song = status?.songName ?? '';
    final live = session.isReady;
    final playing = session.playing;
    final auto = session.autoPlaySong;

    return Column(
      children: [
        PlayerBar(trailing: status?.playlistName),
        const ReconnectBanner(),
        Expanded(
          child: Opacity(
            opacity: live ? 1 : 0.55,
            child: ScrollingColumn(
              children: [
                _Eyebrow(session: session),
                const SizedBox(height: 8),
                FittedTitle(song.isEmpty ? 'No song' : song),
                const SizedBox(height: 12),
                _Time(session: session),
                const SizedBox(height: 14),
                _Progress(session: session),
                const SizedBox(height: 8),
                _SectionLine(session: session),
                const Spacer(),
                const SizedBox(height: 14),
                if (auto != null) ...[
                  _AutoPlayCard(session: session, song: auto),
                  const SizedBox(height: 12),
                ],
                _Transport(session: session, onEarlyRelease: _flashHint),
                const SizedBox(height: 8),
                Text(
                  playing ? 'press and hold to stop' : '',
                  textAlign: TextAlign.center,
                  style: AppText.small.copyWith(
                    color: _hintShown ? colors.text : colors.dim,
                    fontWeight: _hintShown ? FontWeight.w700 : null,
                  ),
                ),
                const SizedBox(height: 10),
                _NextCard(session: session),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

class _Eyebrow extends StatelessWidget {
  const _Eyebrow({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final playing = session.playing;
    final at = session.position;
    final total = session.entries.length;
    final place = at == null
        ? ''
        : (total > 0 ? ' · ${at + 1} of $total' : ' · ${at + 1}');
    return Text(
      '${playing ? 'Playing' : 'Ready'}$place'.toUpperCase(),
      style: AppText.eyebrow.copyWith(
        color: playing ? colors.green : colors.dim,
      ),
    );
  }
}

class _Time extends StatelessWidget {
  const _Time({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return LiveClock(
      running: session.isReady && session.playing,
      builder: (context) {
        final position = session.clock.position;
        final total = session.clock.duration;
        return Semantics(
          label:
              'Time ${formatClock(position)}'
              '${total > Duration.zero ? ' of ${formatClock(total)}' : ''}',
          excludeSemantics: true,
          child: Text.rich(
            TextSpan(
              style: AppText.time(40).copyWith(color: colors.text),
              children: [
                TextSpan(text: formatClock(position)),
                if (total > Duration.zero)
                  TextSpan(
                    text: ' / ${formatClock(total)}',
                    style: AppText.time(
                      16,
                      weight: 500,
                    ).copyWith(color: colors.muted),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _Progress extends StatelessWidget {
  const _Progress({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final total = session.clock.duration;
    final ticks = <double>[];
    final details = session.details;
    if (details != null && total > Duration.zero) {
      for (final span in details.sections) {
        if (span.hasTimes && span.start! > Duration.zero) {
          ticks.add(span.start!.inMicroseconds / total.inMicroseconds);
        }
      }
    }
    return LiveClock(
      running: session.isReady && session.playing,
      builder: (context) {
        final fraction = total > Duration.zero
            ? (session.clock.position.inMicroseconds / total.inMicroseconds)
                  .clamp(0.0, 1.0)
            : 0.0;
        return Semantics(
          label: 'Song progress',
          value: '${(fraction * 100).round()} percent',
          excludeSemantics: true,
          child: SongProgressBar(fraction: fraction, ticks: ticks),
        );
      },
    );
  }
}

/// The progress bar, with a tick at each section's start.
class SongProgressBar extends StatelessWidget {
  /// A bar filled to [fraction], with ticks at [ticks], all from 0 to 1.
  const SongProgressBar({
    required this.fraction,
    required this.ticks,
    super.key,
  });

  /// How much of the bar is filled.
  final double fraction;

  /// Where the section boundaries fall.
  final List<double> ticks;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return SizedBox(
      height: 10,
      child: CustomPaint(
        painter: _ProgressPainter(
          fraction: fraction,
          ticks: ticks,
          track: colors.raised,
          fill: colors.cyan,
          gap: colors.page,
        ),
      ),
    );
  }
}

class _ProgressPainter extends CustomPainter {
  _ProgressPainter({
    required this.fraction,
    required this.ticks,
    required this.track,
    required this.fill,
    required this.gap,
  });

  final double fraction;
  final List<double> ticks;
  final Color track;
  final Color fill;
  final Color gap;

  @override
  void paint(Canvas canvas, Size size) {
    final round = Radius.circular(size.height / 2);
    final bar = RRect.fromRectAndRadius(Offset.zero & size, round);
    canvas.drawRRect(bar, Paint()..color = track);
    canvas.save();
    canvas.clipRRect(bar);
    canvas.drawRect(
      Rect.fromLTWH(0, 0, size.width * fraction, size.height),
      Paint()..color = fill,
    );
    final mark = Paint()..color = gap;
    for (final tick in ticks) {
      canvas.drawRect(
        Rect.fromLTWH(size.width * tick - 1, 0, 2, size.height),
        mark,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_ProgressPainter old) =>
      old.fraction != fraction ||
      old.track != track ||
      old.fill != fill ||
      !_same(old.ticks, ticks);

  static bool _same(List<double> a, List<double> b) {
    if (a.length != b.length) {
      return false;
    }
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) {
        return false;
      }
    }
    return true;
  }
}

class _SectionLine extends StatelessWidget {
  const _SectionLine({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final spans = session.details?.sections ?? const <SectionSpan>[];
    if (spans.isEmpty) {
      return const SizedBox.shrink();
    }
    final colors = context.colors;
    return LiveClock(
      running: session.isReady && session.playing,
      builder: (context) {
        final position = session.clock.position;
        final here = sectionAt(spans, position);
        final next = sectionAfter(spans, position);
        final playing = session.playing;
        final String left = here?.name ?? (playing ? '' : spans.first.name);
        final String right = playing && next != null
            ? 'next: ${next.name} in ${formatClock(next.start! - position)}'
            : '${spans.length} ${spans.length == 1 ? 'section' : 'sections'}';
        return Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Flexible(
              child: Text(
                left,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: AppText.small.copyWith(
                  color: playing ? colors.cyan : colors.muted,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
            const SizedBox(width: 12),
            Flexible(
              child: Text(
                right,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: AppText.small.copyWith(color: colors.muted),
              ),
            ),
          ],
        );
      },
    );
  }
}

class _Transport extends StatelessWidget {
  const _Transport({required this.session, required this.onEarlyRelease});

  final Session session;
  final VoidCallback onEarlyRelease;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final live = session.isReady;
    final playing = session.playing;
    final hasSong = (session.status?.songName ?? '').isNotEmpty;
    final canStep = live && !playing;
    final scale = MediaQuery.textScalerOf(context);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        SizedBox(
          width: 64,
          child: ChunkyButton(
            semanticLabel: 'Previous song',
            onPressed: canStep && !session.isBusy(ActionKind.previous)
                ? () => session.previous()
                : null,
            child: Icon(
              Icons.chevron_left,
              size: scale.scale(32),
              color: colors.text,
            ),
          ),
        ),
        const SizedBox(width: 12),
        Expanded(
          child: playing
              ? HoldToStop(
                  enabled: live && !session.isBusy(ActionKind.stop),
                  onStop: () => session.stop(),
                  onEarlyRelease: onEarlyRelease,
                )
              : ChunkyButton(
                  height: 108,
                  fontSize: 26,
                  shadow: true,
                  label: 'PLAY',
                  semanticLabel: 'Play',
                  fill: colors.play,
                  foreground: colors.onPlay,
                  borderColor: colors.shadow,
                  onPressed: live && hasSong && !session.isBusy(ActionKind.play)
                      ? () {
                          HapticFeedback.lightImpact();
                          session.play();
                        }
                      : null,
                ),
        ),
        const SizedBox(width: 12),
        SizedBox(
          width: 64,
          child: ChunkyButton(
            semanticLabel: 'Next song',
            onPressed: canStep && !session.isBusy(ActionKind.next)
                ? () => session.next()
                : null,
            child: Icon(
              Icons.chevron_right,
              size: scale.scale(32),
              color: colors.text,
            ),
          ),
        ),
      ],
    );
  }
}

class _NextCard extends StatelessWidget {
  const _NextCard({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final entries = session.entries;
    final at = session.position;
    if (at == null || entries.isEmpty) {
      return const SizedBox.shrink();
    }
    final colors = context.colors;
    final next = session.nextEntry;
    final String text;
    if (next == null) {
      text = 'Last song in the playlist';
    } else {
      text = next.hasDuration()
          ? '${next.songName} · ${formatClock(next.duration.toDart())}'
          : next.songName;
    }
    return NoteBox(
      child: Row(
        children: [
          Text('Next', style: AppText.small.copyWith(color: colors.dim)),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              text,
              textAlign: TextAlign.end,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: AppText.bodyStrong.copyWith(
                color: colors.text,
                fontSize: 13.5,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _AutoPlayCard extends StatelessWidget {
  const _AutoPlayCard({required this.session, required this.song});

  final Session session;
  final String song;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return LiveClock(
      running: session.isReady,
      builder: (context) {
        final left = session.autoPlayRemaining ?? Duration.zero;
        return NoteBox(
          tint: colors.loopTint,
          child: Row(
            children: [
              Expanded(
                child: Text(
                  '$song starts in ${formatClock(left)}',
                  style: AppText.bodyStrong.copyWith(color: colors.text),
                ),
              ),
              const SizedBox(width: 8),
              TextButton(
                onPressed: session.isReady
                    ? () => session.cancelAutoPlay()
                    : null,
                child: Text(
                  'Cancel',
                  style: AppText.bodyStrong.copyWith(color: colors.amber),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
