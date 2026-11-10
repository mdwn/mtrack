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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/sections.dart';

import '../support/rig.dart';

Duration ms(int v) => Duration(milliseconds: v);

void main() {
  // 4/4 at 120 bpm: a beat is half a second, a measure two seconds.
  final bpm120 = grid(10);
  const song = Duration(seconds: 20);

  group('resolveSections', () {
    test('places a section by whole measures, end exclusive', () {
      final spans = resolveSections([section('A', 2, 4)], bpm120, song);
      expect(spans.single.start, ms(2000));
      expect(spans.single.end, ms(6000));
      expect(spans.single.hasTimes, isTrue);
      expect(spans.single.length, ms(4000));
    });

    test('measure 1 starts at the first beat of the grid', () {
      final g = grid(4, offset: 1.5);
      final spans = resolveSections([section('A', 1, 2)], g, song);
      expect(spans.single.start, ms(1500));
      expect(spans.single.end, ms(3500));
    });

    test('start and end beats count within their measure from 1', () {
      final spans = resolveSections(
        [section('A', 2, 3, startBeat: 3, endBeat: 2)],
        bpm120,
        song,
      );
      // Measure 2 starts at 2 s, its beat 3 is 1 s later; measure 3 starts at
      // 4 s, its beat 2 half a second later.
      expect(spans.single.start, ms(3000));
      expect(spans.single.end, ms(4500));
    });

    test('fractional beats interpolate between grid beats', () {
      final spans = resolveSections(
        [section('A', 1, 2, startBeat: 1.5, endBeat: 2.25)],
        bpm120,
        song,
      );
      expect(spans.single.start, ms(250));
      expect(spans.single.end, ms(2625));
    });

    test('interpolation follows a changing tempo', () {
      final g = pp.BeatGrid(beats: [0, 1, 3, 3.5], measureStarts: [0, 2]);
      final spans = resolveSections(
        [section('A', 1, 2, startBeat: 2.5, endBeat: 1.5)],
        g,
        const Duration(seconds: 10),
      );
      // Beat 2.5 is half way from 1 s to 3 s; measure 2's beat 1.5 half way
      // from 3 s to 3.5 s.
      expect(spans.single.start, ms(2000));
      expect(spans.single.end, ms(3250));
    });

    test('a beat that would fall in the next measure is not placed', () {
      // Beat 5 of a 4-beat measure is the next measure's downbeat, not a
      // beat of this one.
      final spans = resolveSections(
        [section('A', 1, 3, startBeat: 5)],
        bpm120,
        song,
      );
      expect(spans.single.hasTimes, isFalse);
    });

    test('the last fractional beat may run towards the next downbeat', () {
      final spans = resolveSections(
        [section('A', 1, 3, startBeat: 4.5)],
        bpm120,
        song,
      );
      expect(spans.single.start, ms(1750));
    });

    test('a section past the end of the grid is listed without times', () {
      final short = grid(3);
      final spans = resolveSections(
        [section('Fits', 1, 3), section('Beyond', 5, 7)],
        short,
        const Duration(seconds: 6),
      );
      expect(spans[0].hasTimes, isTrue);
      expect(spans[1].hasTimes, isFalse);
      expect(spans[1].start, isNull);
      expect(spans[1].end, isNull);
      expect(spans[1].name, 'Beyond');
    });

    test('a section may end on the end of the song', () {
      // Three measures: the end measure 4 is past the last, so the section
      // ends a beat after the last beat (5.5 s + 0.5 s = 6 s), no later than
      // the song.
      final g = grid(3);
      final spans = resolveSections(
        [section('A', 3, 4)],
        g,
        const Duration(seconds: 6),
      );
      expect(spans.single.start, ms(4000));
      expect(spans.single.end, ms(6000));
    });

    test('an end past the grid is cut at the song length when shorter', () {
      final g = grid(3);
      final spans = resolveSections(
        [section('A', 3, 9)],
        g,
        const Duration(milliseconds: 5800),
      );
      expect(spans.single.end, ms(5800));
    });

    test('a grid shorter than the sections places those it covers', () {
      final g = grid(2);
      final spans = resolveSections(
        [section('A', 1, 2), section('B', 2, 3), section('C', 3, 4)],
        g,
        const Duration(seconds: 4),
      );
      expect(spans.map((s) => s.hasTimes), [true, true, false]);
    });

    test('a song with no grid lists its sections without times', () {
      final spans = resolveSections([section('A', 1, 2)], null, song);
      expect(spans.single.hasTimes, isFalse);
    });

    test('an empty grid places nothing', () {
      final spans = resolveSections([section('A', 1, 2)], pp.BeatGrid(), song);
      expect(spans.single.hasTimes, isFalse);
    });

    test('a section that does not run forward has no times', () {
      final spans = resolveSections(
        [section('Empty', 3, 3), section('Backwards', 4, 2)],
        bpm120,
        song,
      );
      expect(spans.map((s) => s.hasTimes), [false, false]);
    });

    test('measure 0 is not a measure', () {
      final spans = resolveSections([section('A', 0, 2)], bpm120, song);
      expect(spans.single.hasTimes, isFalse);
    });

    test('keeps the sections in the order given', () {
      final spans = resolveSections(
        [section('B', 3, 4), section('A', 1, 2)],
        bpm120,
        song,
      );
      expect(spans.map((s) => s.name), ['B', 'A']);
    });
  });

  group('bars', () {
    test('a range ends on the bar before the exclusive end', () {
      expect(SectionSpan(section('A', 9, 25), null, null).bars, 'bars 9–24');
    });

    test('one bar is "bar"', () {
      expect(SectionSpan(section('A', 3, 4), null, null).bars, 'bar 3');
    });

    test('an end part way into a bar includes that bar', () {
      expect(
        SectionSpan(section('A', 1, 3, endBeat: 2), null, null).bars,
        'bars 1–3',
      );
    });
  });

  group('sectionAt and sectionAfter', () {
    final spans = resolveSections(
      [
        section('Intro', 1, 3),
        section('Verse', 3, 5),
        section('Gap', 20, 22),
        section('Outro', 6, 8),
      ],
      grid(8),
      const Duration(seconds: 16),
    );

    test('the position is in the section that holds it', () {
      expect(sectionAt(spans, ms(0))?.name, 'Intro');
      expect(sectionAt(spans, ms(3999))?.name, 'Intro');
      expect(sectionAt(spans, ms(4000))?.name, 'Verse');
    });

    test('a section ends before its end time', () {
      expect(sectionAt(spans, ms(8000)), isNull);
      expect(sectionAt(spans, ms(10000))?.name, 'Outro');
      expect(sectionAt(spans, ms(14000)), isNull);
    });

    test('a section without times is never the one', () {
      expect(spans[2].hasTimes, isFalse);
      expect(sectionAt(spans, ms(1)), isNot(equals(spans[2])));
    });

    test('the next section is the soonest that starts after the position', () {
      expect(sectionAfter(spans, ms(0))?.name, 'Verse');
      expect(sectionAfter(spans, ms(4000))?.name, 'Outro');
      expect(sectionAfter(spans, ms(10000)), isNull);
    });

    test('nothing is next when no section is placed', () {
      final none = resolveSections([section('A', 1, 2)], null, Duration.zero);
      expect(sectionAt(none, ms(0)), isNull);
      expect(sectionAfter(none, ms(0)), isNull);
    });
  });
}
