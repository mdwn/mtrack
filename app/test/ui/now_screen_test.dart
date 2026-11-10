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
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/ui/hold_to_stop.dart';
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';
import '../support/fake_player.dart';
import '../support/rig.dart';

Finder button(String label) => find.byWidgetPredicate(
  (w) => w is ChunkyButton && w.semanticLabel == label,
);

bool enabled(WidgetTester tester, String label) =>
    tester.widget<ChunkyButton>(button(label)).onPressed != null;

void main() {
  group('stopped', () {
    appTest('shows the song, the place, the time and the next song', (h) async {
      expect(find.text('READY · 1 OF 3'), findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      expect(find.textContaining('0:00'), findsWidgets);
      expect(find.textContaining('1:40'), findsOneWidget);
      expect(find.text('PLAY'), findsOneWidget);
      expect(find.text('STOP'), findsNothing);
      expect(find.text('Second · 1:30'), findsOneWidget);
      expect(find.text('Intro'), findsOneWidget);
      expect(find.text('2 sections'), findsOneWidget);
      expect(find.text('stage'), findsOneWidget);
      expect(find.text('Set'), findsWidgets);
    });

    appTest('can step to the previous and next song', (h) async {
      expect(enabled(h.tester, 'Previous song'), isTrue);
      expect(enabled(h.tester, 'Next song'), isTrue);
      await h.tester.tap(button('Next song'));
      await h.settle();
      expect(find.text('Second'), findsWidgets);
      expect(find.text('READY · 2 OF 3'), findsOneWidget);
      expect(h.rig.index, 1);
      expect(h.rig.playing, isFalse);
      await h.tester.tap(button('Previous song'));
      await h.settle();
      expect(h.rig.index, 0);
    });

    appTest('plays on a tap', (h) async {
      await h.tester.tap(button('Play'));
      await h.settle();
      expect(h.rig.playing, isTrue);
      expect(find.text('STOP'), findsOneWidget);
      expect(find.text('PLAYING · 1 OF 3'), findsOneWidget);
    });

    appTest('says plainly why a song cannot be played', (h) async {
      h.rig.locked = true;
      await h.tester.tap(button('Play'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(
        find.text('Player is locked. Unlock to make changes.'),
        findsOneWidget,
      );
      expect(find.text('PLAY'), findsOneWidget);
    });

    appTest('does not tick while stopped', (h) async {
      await h.tester.pump(const Duration(seconds: 1));
      expect(h.tester.binding.hasScheduledFrame, isFalse);
    });

    appTest('shows no section line for a song without sections', (h) async {
      await h.tester.tap(button('Next song'));
      await h.settle();
      expect(find.textContaining('section'), findsNothing);
    });

    appTest('wraps a long title and shrinks it before cutting it off', (
      h,
    ) async {
      h.rig.songs[0] = RigSong(
        'A Very Long Song Title That Goes On And On And On',
        lengthSeconds: 100,
      );
      h.rig.publish();
      await h.settle();
      final title = find.text(
        'A Very Long Song Title That Goes On And On And On',
      );
      expect(title, findsOneWidget);
      final text = h.tester.widget<Text>(title);
      expect(text.style!.fontSize, lessThan(34));
      expect(text.style!.fontSize, greaterThanOrEqualTo(20));
      expect(h.tester.getSize(title).height, lessThan(80));
    }, setup: null);

    appTest('has a Play control that keeps its shape with no song', (h) async {
      h.rig.songs = [];
      h.rig.publish();
      await h.settle();
      expect(find.text('No song'), findsOneWidget);
      expect(enabled(h.tester, 'Play'), isFalse);
    });
  });

  group('playing', () {
    void playing(Rig rig) {
      rig.playing = true;
      rig.elapsed = const Duration(seconds: 8);
    }

    appTest('shows Stop, and dims Previous and Next', (h) async {
      expect(find.text('STOP'), findsOneWidget);
      expect(find.text('PLAY'), findsNothing);
      expect(find.text('PLAYING · 1 OF 3'), findsOneWidget);
      expect(find.text('press and hold to stop'), findsOneWidget);
      expect(enabled(h.tester, 'Previous song'), isFalse);
      expect(enabled(h.tester, 'Next song'), isFalse);
    }, setup: playing);

    appTest('runs the clock between updates, and only while playing', (
      h,
    ) async {
      expect(find.textContaining('0:08'), findsOneWidget);
      await h.advance(const Duration(seconds: 3));
      await h.advance(const Duration(milliseconds: 200));
      expect(find.textContaining('0:11'), findsOneWidget);
      expect(h.tester.binding.hasScheduledFrame, isTrue);
      h.rig.playing = false;
      h.rig.elapsed = const Duration(seconds: 11);
      h.rig.publish();
      await h.settle();
      await h.advance(const Duration(seconds: 5));
      expect(find.textContaining('0:11'), findsOneWidget);
      await h.tester.pump(const Duration(seconds: 1));
      expect(h.tester.binding.hasScheduledFrame, isFalse);
    }, setup: playing);

    appTest(
      'names the section and the next one with a countdown',
      (h) async {
        // Verse is measures 3-6 (4 s to 12 s at 120 bpm); Intro runs 0 s to 4 s.
        expect(find.text('Intro'), findsOneWidget);
        expect(find.text('next: Verse in 0:04'), findsOneWidget);
      },
      setup: (rig) {
        rig.playing = true;
        rig.elapsed = Duration.zero;
      },
    );

    appTest('is in the second section after the first ends', (h) async {
      expect(find.text('Verse'), findsOneWidget);
      expect(find.textContaining('next:'), findsNothing);
    }, setup: playing);

    appTest('a tap on Stop does not stop', (h) async {
      await h.tester.tap(find.text('STOP'));
      await h.settle();
      expect(h.rig.playing, isTrue);
      expect(h.rig.player.received.where((r) => r.method == 'Stop'), isEmpty);
    }, setup: playing);

    appTest('an early release does not stop and shows the hint', (h) async {
      final press = await h.tester.startGesture(
        h.tester.getCenter(find.text('STOP')),
      );
      // The hold is timed from the first frame after the press.
      await h.tester.pump();
      await h.advance(const Duration(milliseconds: 599));
      await press.up();
      await h.settle();
      await h.advance(const Duration(seconds: 1));
      expect(h.rig.playing, isTrue);
      expect(h.rig.player.received.where((r) => r.method == 'Stop'), isEmpty);
      final hint = h.tester.widget<Text>(find.text('press and hold to stop'));
      expect(hint.style!.fontWeight, FontWeight.w700);
      await h.advance(const Duration(seconds: 3));
      final calm = h.tester.widget<Text>(find.text('press and hold to stop'));
      expect(calm.style!.fontWeight, isNot(FontWeight.w700));
    }, setup: playing);

    appTest('a full hold stops, once', (h) async {
      final press = await h.tester.startGesture(
        h.tester.getCenter(find.text('STOP')),
      );
      // The hold is timed from the first frame after the press.
      await h.tester.pump();
      await h.advance(const Duration(milliseconds: 599));
      expect(h.rig.player.received.where((r) => r.method == 'Stop'), isEmpty);
      await h.advance(const Duration(milliseconds: 2));
      await h.settle();
      expect(
        h.rig.player.received.where((r) => r.method == 'Stop'),
        hasLength(1),
      );
      await press.up();
      await h.advance(const Duration(seconds: 1));
      expect(
        h.rig.player.received.where((r) => r.method == 'Stop'),
        hasLength(1),
      );
      expect(h.rig.playing, isFalse);
      expect(find.text('PLAY'), findsOneWidget);
    }, setup: playing);

    appTest('the hold fills the button as it goes', (h) async {
      final press = await h.tester.startGesture(
        h.tester.getCenter(find.text('STOP')),
      );
      // The hold is timed from the first frame after the press.
      await h.tester.pump();
      await h.advance(holdToStopDuration ~/ 2);
      final fill = h.tester
          .widgetList<FractionallySizedBox>(find.byType(FractionallySizedBox))
          .first;
      expect(fill.widthFactor, closeTo(0.5, 0.05));
      await press.up();
    }, setup: playing);

    appTest('a screen reader can stop with its long-press action', (h) async {
      final handle = h.tester.ensureSemantics();
      h.tester.semantics.performAction(
        find.semantics.byLabel('Stop'),
        SemanticsAction.longPress,
      );
      await h.settle();
      expect(h.rig.playing, isFalse);
      handle.dispose();
    }, setup: playing);

    appTest('a screen reader hears how to stop', (h) async {
      final handle = h.tester.ensureSemantics();
      final node = h.tester.semantics.find(find.bySemanticsLabel('Stop'));
      expect(node.hint, 'Press and hold to stop');
      expect(
        node.getSemanticsData().hasAction(SemanticsAction.longPress),
        isTrue,
      );
      handle.dispose();
    }, setup: playing);

    appTest(
      'shows a countdown to the song that starts by itself',
      (h) async {
        expect(find.text('Second starts in 0:09'), findsOneWidget);
        await h.advance(const Duration(seconds: 4));
        expect(find.text('Second starts in 0:05'), findsOneWidget);
        await h.tester.tap(find.text('Cancel'));
        await h.settle();
        expect(
          h.rig.player.received.where((r) => r.method == 'CancelAutoPlay'),
          hasLength(1),
        );
        expect(find.textContaining('starts in'), findsNothing);
      },
      setup: (rig) {
        rig.autoPlaySong = 'Second';
        rig.autoPlayIn = const Duration(seconds: 9);
      },
    );

    appTest('does not send a second stop while the first is out', (h) async {
      h.rig.player.handlers['Stop'] = (_) =>
          FakeReply.ok(pp.StopResponse(), delay: const Duration(seconds: 2));
      final press = await h.tester.startGesture(
        h.tester.getCenter(find.text('STOP')),
      );
      // The hold is timed from the first frame after the press.
      await h.tester.pump();
      await h.advance(const Duration(milliseconds: 700));
      await press.up();
      await h.advance(const Duration(milliseconds: 100));
      expect(h.session.isBusy(ActionKind.stop), isTrue);
      final again = await h.tester.startGesture(
        h.tester.getCenter(find.text('STOP')),
      );
      await h.tester.pump();
      await h.advance(const Duration(milliseconds: 700));
      await again.up();
      await h.advance(const Duration(seconds: 2));
      expect(
        h.rig.player.received.where((r) => r.method == 'Stop'),
        hasLength(1),
      );
    }, setup: playing);
  });

  group('without the player', () {
    appTest('shows a banner, dims the last state and disables the controls', (
      h,
    ) async {
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      expect(enabled(h.tester, 'Play'), isFalse);
      expect(enabled(h.tester, 'Next song'), isFalse);
      await h.advance(const Duration(seconds: 30));
    });

    appTest('the banner goes when the link comes back', (h) async {
      h.rig.player.disconnect();
      await h.settle();
      await h.advance(const Duration(seconds: 2));
      await h.settle();
      expect(find.text('Reconnecting…'), findsNothing);
      expect(enabled(h.tester, 'Play'), isTrue);
    });
  });
}
