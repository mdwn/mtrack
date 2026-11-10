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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/ui/settings_sheet.dart';

import '../support/app_harness.dart';
import '../support/fake_player.dart';

int calls(AppHarness h, String method) =>
    h.rig.player.received.where((r) => r.method == method).length;

Future<void> openSettings(AppHarness h) async {
  appVersion = () async => '0.1.0 (1)';
  await h.tester.tap(find.byTooltip('Settings'));
  await h.settle();
  await h.tester.pump(const Duration(milliseconds: 400));
}

void main() {
  group('the tab bar', () {
    appTest('has four tabs, in order', (h) async {
      final centres = <double>[];
      for (final label in ['Now', 'Playlist', 'Sections', 'Levels']) {
        final bar = find
            .text(label)
            .evaluate()
            .map((e) => h.tester.getCenter(find.byWidget(e.widget)))
            .where((c) => c.dy > 560);
        expect(bar, hasLength(1), reason: label);
        centres.add(bar.single.dx);
      }
      expect([...centres]..sort(), centres);
      expect(centres.toSet(), hasLength(4));
    });
  });

  group('the levels', () {
    appTest('are not fetched until the tab is opened', (h) async {
      expect(calls(h, 'GetTrackGains'), 0);
      await h.openTab('Levels');
      expect(calls(h, 'GetTrackGains'), 1);
    });

    appTest('show a row for each track', (h) async {
      await h.openTab('Levels');
      for (final name in ['Click', 'Cue', 'Keys']) {
        expect(find.text(name), findsOneWidget);
      }
      expect(find.text('0.0 dB'), findsOneWidget);
      expect(find.text('−3.0 dB'), findsOneWidget);
      expect(find.text('−6.0 dB'), findsOneWidget);
      expect(find.byType(Slider), findsNWidgets(3));
      expect(
        find.text(
          'Levels are saved on the player. A mute lasts until the player '
          'restarts.',
        ),
        findsOneWidget,
      );
    });

    appTest('say what each control is to a screen reader', (h) async {
      final handle = h.tester.ensureSemantics();
      await h.openTab('Levels');
      final fader = h.tester.getSemantics(find.bySemanticsLabel('Cue level'));
      expect(fader.value, '−3.0 dB');
      expect(find.bySemanticsLabel('Mute Cue'), findsOneWidget);
      expect(find.bySemanticsLabel('Mute Click'), findsOneWidget);
      handle.dispose();
    });

    appTest('keep their controls at least 48 px', (h) async {
      await h.openTab('Levels');
      final mute = h.tester.getSize(find.bySemanticsLabel('Mute Cue'));
      expect(mute.width, greaterThanOrEqualTo(48));
      expect(mute.height, greaterThanOrEqualTo(48));
      final fader = h.tester.getSize(find.byType(Slider).first);
      expect(fader.height, greaterThanOrEqualTo(48));
    });

    appTest('move a gain by dragging its fader', (h) async {
      await h.openTab('Levels');
      await h.tester.drag(find.byType(Slider).first, const Offset(-120, 0));
      await h.advance(const Duration(milliseconds: 300));
      await h.settle();
      expect(calls(h, 'SetTrackGain'), greaterThan(0));
      final gain = h.rig.tracks[0].gainDb;
      expect(gain, lessThan(0));
      expect(
        find.text(gain < 0 ? '−${(-gain).toStringAsFixed(1)} dB' : ''),
        findsOneWidget,
      );
    });

    appTest('take a level typed to the tenth', (h) async {
      await h.openTab('Levels');
      await h.tester.tap(find.bySemanticsLabel('Type the Cue level'));
      await h.tester.pumpAndSettle();
      await h.tester.enterText(find.byType(TextField), '-7,3');
      await h.tester.tap(find.text('Set'));
      await h.tester.pumpAndSettle();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 200));
      expect(h.rig.tracks[1].gainDb, -7.3);
      expect(find.text('−7.3 dB'), findsOneWidget);
    });

    appTest('refuse a typed level out of range and keep the dialog', (h) async {
      await h.openTab('Levels');
      final before = h.rig.tracks[1].gainDb;
      await h.tester.tap(find.bySemanticsLabel('Type the Cue level'));
      await h.tester.pumpAndSettle();
      await h.tester.enterText(find.byType(TextField), '40');
      await h.tester.tap(find.text('Set'));
      await h.tester.pumpAndSettle();
      expect(find.text('A number from -60 to 12'), findsOneWidget);
      await h.tester.tap(find.text('Cancel'));
      await h.tester.pumpAndSettle();
      expect(h.rig.tracks[1].gainDb, before);
    });

    appTest('mute a track and show it muted', (h) async {
      await h.openTab('Levels');
      await h.tester.tap(find.bySemanticsLabel('Mute Cue'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(h.rig.tracks[1].muted, isTrue);
      expect(find.bySemanticsLabel('Unmute Cue'), findsOneWidget);
      await h.tester.tap(find.bySemanticsLabel('Unmute Cue'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(h.rig.tracks[1].muted, isFalse);
      expect(find.bySemanticsLabel('Mute Cue'), findsOneWidget);
    });

    appTest('follow a change made elsewhere while the tab is open', (h) async {
      await h.openTab('Levels');
      h.rig.setGainElsewhere('Keys', 4);
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(find.text('+4.0 dB'), findsOneWidget);
    });

    appTest('say when the player has no tracks', (h) async {
      await h.openTab('Levels');
      expect(
        find.text('This player has no output tracks to adjust.'),
        findsOneWidget,
      );
      expect(find.byType(Slider), findsNothing);
    }, setup: (rig) => rig.tracks = []);

    appTest('say when the player is too old', (h) async {
      await h.openTab('Levels');
      expect(
        find.text(
          'This player is too old for levels. Update mtrack to use them.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('is locked'), findsNothing);
      expect(find.byType(Slider), findsNothing);
    }, setup: (rig) => rig.levelsAllowed = false);

    var failing = true;
    appTest(
      'offer a retry when they cannot be read',
      (h) async {
        await h.openTab('Levels');
        expect(
          find.text('Could not get the levels from the player.'),
          findsOneWidget,
        );
        expect(find.text('the mixer is gone'), findsOneWidget);
        failing = false;
        await h.tester.tap(find.text('Retry'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 50));
        expect(find.text('Cue'), findsOneWidget);
        expect(find.text('Retry'), findsNothing);
      },
      setup: (rig) {
        failing = true;
        final good = rig.player.handlers['GetTrackGains']!;
        rig.player.handlers['GetTrackGains'] = (r) =>
            failing ? FakeReply.error(13, 'the mixer is gone') : good(r);
      },
    );

    appTest('stay on screen, dimmed and inert, without the player', (h) async {
      await h.openTab('Levels');
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(find.text('Cue'), findsOneWidget);
      await h.tester.tap(find.bySemanticsLabel('Mute Cue'));
      await h.settle();
      expect(calls(h, 'SetTrackMute'), 0);
      await h.advance(const Duration(seconds: 30));
    });

    appTest('are blank before the player has answered, without it', (h) async {
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.openTab('Levels');
      expect(find.text('Loading the levels…'), findsNothing);
      expect(find.byType(Slider), findsNothing);
      await h.advance(const Duration(seconds: 30));
    });
  });

  group('the ambient light', () {
    appTest('shows the mode and what it is doing', (h) async {
      await openSettings(h);
      expect(find.text('Ambient light'), findsOneWidget);
      expect(find.text('playing'), findsOneWidget);
      final auto = find.bySemanticsLabel('Ambient light Auto');
      expect(auto, findsOneWidget);
    }, setup: (rig) => rig.ambientState = 'playing');

    appTest('says waiting for a pending show', (h) async {
      await openSettings(h);
      expect(find.text('waiting'), findsOneWidget);
    }, setup: (rig) => rig.ambientState = 'pending');

    appTest('is hidden when the player has none', (h) async {
      await openSettings(h);
      expect(find.text('Forget this player'), findsOneWidget);
      expect(find.text('Ambient light'), findsNothing);
    }, setup: (rig) => rig.ambientMode = null);

    appTest('changes the mode when a segment is tapped', (h) async {
      await openSettings(h);
      await h.tester.tap(find.text('On'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      final request = pp.SetAmbientModeRequest.fromBuffer(
        h.rig.player.received
            .lastWhere((r) => r.method == 'SetAmbientMode')
            .payload,
      );
      expect(request.mode, 'on');
      expect(h.rig.ambientMode, 'on');
      expect(find.text('playing'), findsOneWidget);
    });

    appTest('says there is no ambient show when there is none', (h) async {
      await openSettings(h);
      await h.tester.tap(find.text('On'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(find.text('no ambient show'), findsOneWidget);
      await h.tester.tap(find.text('Off'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(find.text('no ambient show'), findsNothing);
      expect(find.text('off'), findsOneWidget);
    }, setup: (rig) => rig.ambientShow = false);

    appTest('cannot be changed without the player', (h) async {
      await openSettings(h);
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      await h.tester.tap(find.text('Off'));
      await h.settle();
      expect(calls(h, 'SetAmbientMode'), 0);
      await h.advance(const Duration(seconds: 30));
    });
  });
}
