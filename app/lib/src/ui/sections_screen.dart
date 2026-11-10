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

import '../app/sections.dart';
import '../app/session.dart';
import 'format.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// The Sections tab: the song's sections, to jump to or loop.
class SectionsScreen extends StatelessWidget {
  /// The Sections tab.
  const SectionsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final song = session.status?.songName ?? '';
    final details = session.details;
    final spans = details?.sections ?? const <SectionSpan>[];
    final looping = session.loopingSection;
    final live = session.isReady;

    final Widget body;
    if (song.isEmpty) {
      body = _Empty(text: live ? 'No song is selected' : '');
    } else if (details == null) {
      body = _Empty(text: live ? 'Loading the song…' : '');
    } else if (spans.isEmpty) {
      body = const _Empty(text: 'This song has no sections');
    } else {
      body = ListView.builder(
        padding: const EdgeInsets.fromLTRB(18, 0, 18, 8),
        itemCount: spans.length,
        itemBuilder: (context, index) {
          final span = spans[index];
          return Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: _SectionRow(
              index: index,
              span: span,
              looping: span.name == looping,
              session: session,
            ),
          );
        },
      );
    }

    return Column(
      children: [
        PlayerBar(trailing: song),
        const ReconnectBanner(),
        if (looping != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 8, 18, 4),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 11,
                  vertical: 5,
                ),
                decoration: BoxDecoration(
                  color: colors.loopTint,
                  borderRadius: BorderRadius.circular(99),
                ),
                child: Text(
                  'Looping $looping',
                  style: AppText.small.copyWith(
                    color: colors.amber,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ),
          ),
        const SizedBox(height: 8),
        Expanded(
          child: Opacity(opacity: live ? 1 : 0.55, child: body),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 4, 18, 14),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (looping != null) ...[
                ChunkyButton(
                  height: 52,
                  fontSize: 16,
                  label: 'Stop looping',
                  semanticLabel: 'Stop looping',
                  foreground: colors.amber,
                  borderColor: colors.amber,
                  onPressed: live && !session.isBusy(ActionKind.stopLoop)
                      ? () => session.stopLoop()
                      : null,
                ),
                const SizedBox(height: 12),
              ],
              if (spans.isNotEmpty)
                const NoteBox(
                  child: Text(
                    'Tap a section to jump to it. Hold one to loop it until '
                    'you stop the loop.',
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Text(
          text,
          textAlign: TextAlign.center,
          style: AppText.body.copyWith(color: context.colors.muted),
        ),
      ),
    );
  }
}

class _SectionRow extends StatelessWidget {
  const _SectionRow({
    required this.index,
    required this.span,
    required this.looping,
    required this.session,
  });

  final int index;
  final SectionSpan span;
  final bool looping;
  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final usable = session.isReady && span.hasTimes;
    final times = span.hasTimes
        ? '${formatClock(span.start!)}–${formatClock(span.end!)}'
        : null;
    return RowCard(
      tint: looping ? colors.loopTint : null,
      mark: looping ? colors.amber : null,
      enabled: span.hasTimes,
      onTap: usable && !session.isBusy(ActionKind.seekSection)
          ? () => session.seekToSection(span.name)
          : null,
      onLongPress: usable && !session.isBusy(ActionKind.loopSection)
          ? () {
              HapticFeedback.mediumImpact();
              session.loopSection(span.name);
            }
          : null,
      semanticLabel:
          '${span.name}, ${span.bars}${times == null ? '' : ', $times'}'
          '${looping ? ', looping' : ''}',
      semanticHint: usable ? 'Tap to jump here. Hold to loop.' : null,
      child: Row(
        children: [
          SizedBox(
            width: 26,
            child: Text(
              '${index + 1}',
              style: AppText.time(12, weight: 500).copyWith(color: colors.dim),
            ),
          ),
          Expanded(
            child: Text(
              span.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: (looping ? AppText.rowCurrent : AppText.row).copyWith(
                color: colors.text,
              ),
            ),
          ),
          const SizedBox(width: 8),
          Column(
            crossAxisAlignment: CrossAxisAlignment.end,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                span.bars,
                style: AppText.time(
                  12.5,
                  weight: 500,
                ).copyWith(color: colors.muted),
              ),
              if (times != null)
                Text(
                  times,
                  style: AppText.time(
                    11,
                    weight: 500,
                  ).copyWith(color: colors.dim),
                ),
            ],
          ),
        ],
      ),
    );
  }
}
