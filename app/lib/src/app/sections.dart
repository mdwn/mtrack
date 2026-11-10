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
import 'dart:math';

import '../../gen/player/v1/player.pb.dart' as player;

/// A song's section placed on its beat grid.
///
/// [start] and [end] are null when the section cannot be placed: the song has
/// no beat grid, or the section's measures or beats fall outside it. Such a
/// section is still listed, by its bars, and a player cannot seek to or loop
/// it either.
class SectionSpan {
  /// A section with its resolved times, if it has them.
  const SectionSpan(this.section, this.start, this.end);

  /// The section as the player describes it.
  final player.Section section;

  /// Where the section starts in the song, or null when it cannot be placed.
  final Duration? start;

  /// Where the section ends (exclusive), or null when it cannot be placed.
  final Duration? end;

  /// The section's name.
  String get name => section.name;

  /// True when [start] and [end] are known.
  bool get hasTimes => start != null && end != null;

  /// The section's length, or null when it cannot be placed.
  Duration? get length => hasTimes ? end! - start! : null;

  /// The bars the section covers, such as "bars 9–24" or "bar 3". The end
  /// measure is exclusive, so a section that ends on a measure line stops at
  /// the bar before it; one that ends part way into a bar includes that bar.
  String get bars {
    final first = section.startMeasure;
    final lastBar = section.hasEndBeat()
        ? section.endMeasure
        : section.endMeasure - 1;
    final last = max(first, lastBar);
    return last == first ? 'bar $first' : 'bars $first–$last';
  }
}

/// Places [sections] on [grid], the way the player does when it seeks to or
/// loops one.
///
/// Measures count from 1 and a section's end is exclusive. `start_beat` and
/// `end_beat` are 1-based within their measure and may be fractional; absent,
/// they mean the measure line. A section ending past the grid's last measure
/// ends one beat after the last beat, no later than [songLength], so a section
/// can end on the song's end. A section the grid cannot place, or one that
/// does not run forward, has no times.
List<SectionSpan> resolveSections(
  List<player.Section> sections,
  player.BeatGrid? grid,
  Duration songLength,
) {
  return [for (final section in sections) _resolve(section, grid, songLength)];
}

SectionSpan _resolve(
  player.Section section,
  player.BeatGrid? grid,
  Duration songLength,
) {
  if (grid == null || section.startMeasure < 1 || section.endMeasure < 1) {
    return SectionSpan(section, null, null);
  }
  final startMeasure = section.startMeasure - 1;
  final endMeasure = section.endMeasure - 1;

  final start = _beatTime(
    grid,
    startMeasure,
    section.hasStartBeat() ? section.startBeat : 1.0,
  );
  if (start == null) {
    return SectionSpan(section, null, null);
  }
  final double? end;
  if (endMeasure < grid.measureStarts.length) {
    end = _beatTime(
      grid,
      endMeasure,
      section.hasEndBeat() ? section.endBeat : 1.0,
    );
  } else if (grid.beats.isEmpty) {
    end = null;
  } else {
    final last = grid.beats.last;
    final beat = grid.beats.length >= 2
        ? last - grid.beats[grid.beats.length - 2]
        : 0.0;
    final tail = last + max(beat, 0.0);
    final length = songLength.inMicroseconds / Duration.microsecondsPerSecond;
    end = length > last ? min(tail, length) : tail;
  }
  if (end == null || start >= end) {
    return SectionSpan(section, null, null);
  }
  return SectionSpan(section, _seconds(start), _seconds(end));
}

Duration _seconds(double s) =>
    Duration(microseconds: (s * Duration.microsecondsPerSecond).round());

/// The time in seconds of a (possibly fractional) 1-based [beat] of the
/// 0-indexed [measure], or null when it lies outside the grid. A beat must
/// stay inside its own measure, though the last fractional beat may run
/// towards the next measure's downbeat.
double? _beatTime(player.BeatGrid grid, int measure, double beat) {
  if (!beat.isFinite || beat < 1.0 || measure >= grid.measureStarts.length) {
    return null;
  }
  final offset = beat - 1.0;
  final whole = offset.floor();
  final start = grid.measureStarts[measure];
  final end = measure + 1 < grid.measureStarts.length
      ? grid.measureStarts[measure + 1]
      : grid.beats.length;
  if (end < start || whole >= end - start) {
    return null;
  }
  final base = start + whole;
  if (base >= grid.beats.length) {
    return null;
  }
  final t0 = grid.beats[base];
  final fraction = offset - whole;
  if (fraction == 0.0) {
    return t0;
  }
  if (base + 1 >= grid.beats.length) {
    return null;
  }
  final t1 = grid.beats[base + 1];
  return t0 + (t1 - t0) * fraction;
}

/// The section [position] is in: the first placed section that starts at or
/// before it and ends after it. Sections that overlap are not expected; the
/// first listed wins.
SectionSpan? sectionAt(List<SectionSpan> spans, Duration position) {
  for (final span in spans) {
    if (span.hasTimes && span.start! <= position && position < span.end!) {
      return span;
    }
  }
  return null;
}

/// The placed section that starts soonest after [position], or null when none
/// does.
SectionSpan? sectionAfter(List<SectionSpan> spans, Duration position) {
  SectionSpan? best;
  for (final span in spans) {
    if (!span.hasTimes || span.start! <= position) {
      continue;
    }
    if (best == null || span.start! < best.start!) {
      best = span;
    }
  }
  return best;
}
