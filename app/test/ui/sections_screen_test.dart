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
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';
import '../support/rig.dart';

Finder row(String name) =>
    find.ancestor(of: find.text(name), matching: find.byType(RowCard));

pp.LoopSectionRequest loopRequest(AppHarness h) =>
    pp.LoopSectionRequest.fromBuffer(
      h.rig.player.received.lastWhere((r) => r.method == 'LoopSection').payload,
    );

void main() {
  void withSections(Rig rig) {
    rig.songs[0] = RigSong(
      'First',
      lengthSeconds: 100,
      sections: [
        section('Intro', 1, 5),
        section('Verse', 5, 13),
        section('Chorus', 13, 21),
        section('Lost', 90, 95),
      ],
      grid: grid(40),
    );
  }

  group('the sections', () {
    appTest('lists them with their bars and times', (h) async {
      await h.openTab('Sections');
      expect(find.text('Intro'), findsWidgets);
      expect(find.text('bars 1–4'), findsOneWidget);
      expect(find.text('0:00–0:08'), findsOneWidget);
      expect(find.text('Verse'), findsOneWidget);
      expect(find.text('bars 5–12'), findsOneWidget);
      expect(find.text('0:08–0:24'), findsOneWidget);
      expect(find.text('Chorus'), findsOneWidget);
      expect(find.text('bars 13–20'), findsOneWidget);
      expect(
        find.text(
          'Tap a section to jump to it. Hold one to loop it until you stop the loop.',
        ),
        findsOneWidget,
      );
      expect(find.text('Stop looping'), findsNothing);
    }, setup: withSections);

    appTest('lists a section the grid does not cover, without times', (
      h,
    ) async {
      await h.openTab('Sections');
      expect(find.text('Lost'), findsOneWidget);
      expect(find.text('bars 90–94'), findsOneWidget);
      final lost = h.tester.widget<RowCard>(row('Lost'));
      expect(lost.onTap, isNull);
      expect(lost.onLongPress, isNull);
      expect(lost.enabled, isFalse);
    }, setup: withSections);

    appTest('a tap jumps to the section', (h) async {
      await h.openTab('Sections');
      await h.tester.tap(find.text('Chorus'));
      await h.settle();
      final seek = h.rig.player.received.where(
        (r) => r.method == 'SeekToSection',
      );
      expect(seek, hasLength(1));
      expect(
        pp.SeekToSectionRequest.fromBuffer(seek.single.payload).sectionName,
        'Chorus',
      );
      expect(
        h.rig.player.received.where((r) => r.method == 'LoopSection'),
        isEmpty,
      );
    }, setup: withSections);

    appTest(
      'a hold loops the section, and marks it',
      (h) async {
        await h.openTab('Sections');
        await h.tester.longPress(find.text('Verse'));
        await h.settle();
        expect(loopRequest(h).sectionName, 'Verse');
        expect(find.text('Looping Verse'), findsOneWidget);
        expect(find.text('Stop looping'), findsOneWidget);
        final marked = h.tester.widget<RowCard>(row('Verse').last);
        expect(marked.mark, isNotNull);
        expect(h.tester.widget<RowCard>(row('Chorus')).mark, isNull);
        expect(
          h.rig.player.received.where((r) => r.method == 'SeekToSection'),
          isEmpty,
        );
      },
      setup: (rig) {
        withSections(rig);
        rig.playing = true;
      },
    );

    appTest(
      'Stop looping ends the loop',
      (h) async {
        await h.openTab('Sections');
        expect(find.text('Looping Chorus'), findsOneWidget);
        await h.tester.tap(find.text('Stop looping'));
        await h.settle();
        expect(h.rig.loop, isNull);
        expect(find.text('Stop looping'), findsNothing);
        expect(find.text('Looping Chorus'), findsNothing);
      },
      setup: (rig) {
        withSections(rig);
        rig.playing = true;
        rig.loop = 'Chorus';
      },
    );

    appTest('says why a loop was refused', (h) async {
      await h.openTab('Sections');
      await h.tester.longPress(find.text('Verse'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(
        find.text('Cannot loop section: no song is playing'),
        findsOneWidget,
      );
      expect(find.text('Stop looping'), findsNothing);
    }, setup: withSections);

    appTest(
      'has an empty state for a song without sections',
      (h) async {
        await h.openTab('Sections');
        expect(find.text('This song has no sections'), findsOneWidget);
        expect(find.textContaining('Tap a section'), findsNothing);
      },
      setup: (rig) {
        rig.songs[0] = RigSong('First', lengthSeconds: 100);
      },
    );

    appTest('follows the song when it changes', (h) async {
      await h.openTab('Sections');
      expect(find.text('Verse'), findsOneWidget);
      h.rig.index = 1;
      h.rig.publish();
      await h.settle();
      expect(find.text('This song has no sections'), findsOneWidget);
    }, setup: withSections);

    appTest('is inert when the player is out of reach', (h) async {
      await h.openTab('Sections');
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(h.tester.widget<RowCard>(row('Verse')).onTap, isNull);
      await h.advance(const Duration(seconds: 30));
    }, setup: withSections);
  });
}
