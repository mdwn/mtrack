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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/ui/lights_screen.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';

import '../support/app_harness.dart';
import '../support/fake_stage.dart';
import '../support/rig.dart';

/// Every screen in every state, in both themes at normal and large text on a
/// small phone. A `RenderFlex overflowed` anywhere fails the test, so these
/// check the layouts hold; what the screens say is checked in their own files.
void main() {
  const longName =
      'The Extraordinarily Long Name of a Song That Nobody Would Choose';

  void sized(Rig rig) {
    rig.songs = [
      RigSong(
        longName,
        lengthSeconds: 3725,
        sections: [
          section('A section with a very long name indeed', 1, 5),
          section('Chorus', 5, 9),
          section('Beyond the grid', 400, 500),
        ],
        grid: grid(40),
      ),
      RigSong('Short', lengthSeconds: 59),
    ];
    rig.playlists['Set'] = rig.songs;
    rig.library = List.of(rig.songs);
    rig.playlist = 'A playlist with a name that runs on and on and on';
    rig.playlists[rig.playlist] = rig.songs;
  }

  const longPlaylist =
      'A playlist with a name that runs on and on and on and on and on';

  /// The Lights tab, open, with its page ready and a layout drawn.
  Future<void> showLights(AppHarness h) async {
    await h.openTab('Lights');
    h.stage.surface.ready();
    h.rig.player.streamMessage(h.rig.player.openStreams.last, stageLayout(1));
    await h.settle();
  }

  Future<void> startEditing(AppHarness h) async {
    await h.openTab('Library');
    await h.tester.tap(find.text('Edit'));
    await h.settle();
    await h.tester.pump(const Duration(milliseconds: 400));
  }

  for (final look in Look.all) {
    group('$look', () {
      appTest(
        'Now, stopped',
        (h) async {
          expect(find.text('PLAY'), findsOneWidget);
          await h.tester.pump(const Duration(seconds: 1));
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Now, playing',
        (h) async {
          expect(find.text('STOP'), findsOneWidget);
          await h.advance(const Duration(seconds: 1));
        },
        setup: (rig) {
          sized(rig);
          rig.playing = true;
          rig.elapsed = const Duration(seconds: 3601);
        },
        look: look,
      );

      appTest(
        'Now, with a song about to start',
        (h) async {
          expect(find.textContaining('starts in'), findsOneWidget);
          await h.advance(const Duration(seconds: 1));
        },
        setup: (rig) {
          sized(rig);
          rig.autoPlaySong = longName;
          rig.autoPlayIn = const Duration(seconds: 12);
        },
        look: look,
      );

      appTest(
        'Now, without the player',
        (h) async {
          h.rig.link.connectFailures = 100;
          h.rig.player.disconnect();
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.text('Reconnecting…'), findsOneWidget);
          await h.advance(const Duration(seconds: 30));
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Playlist',
        (h) async {
          await h.openTab('Library');
          expect(find.text('Short'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'the playlist picker',
        (h) async {
          await h.openTab('Library');
          await h.tester.tap(find.text('change'));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 400));
          expect(find.text('Choose a playlist'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Playlist, locked',
        (h) async {
          await h.openTab('Library');
          expect(find.text('Unlock to edit'), findsOneWidget);
          expect(find.text('Locked'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.locked = true;
        },
        look: look,
      );

      appTest(
        'the playlist picker, with a playlist to delete',
        (h) async {
          await h.openTab('Library');
          await h.tester.tap(find.text('change'));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 400));
          expect(find.text('+ NEW PLAYLIST'), findsOneWidget);
          expect(find.text(longPlaylist), findsWidgets);
        },
        setup: (rig) {
          sized(rig);
          rig.playlists[longPlaylist] = [RigSong('First')];
        },
        look: look,
      );

      appTest(
        'the editor',
        (h) async {
          await startEditing(h);
          expect(find.text('Editing'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'the editor, with the playlist changed under it',
        (h) async {
          await startEditing(h);
          h.rig.editElsewhere(h.rig.playlist, ['Short']);
          await h.tester.tap(find.byTooltip('Remove Short'));
          await h.tester.pump(const Duration(milliseconds: 50));
          await h.tester.tap(find.text('SAVE'));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.text('SAVE ANYWAY'), findsOneWidget);
          expect(find.text('DISCARD MY CHANGES'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'the editor, locked',
        (h) async {
          await startEditing(h);
          h.rig.locked = true;
          h.rig.publish();
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.textContaining('Unlock it to save'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'the editor, without the player',
        (h) async {
          await startEditing(h);
          h.rig.link.connectFailures = 100;
          h.rig.player.disconnect();
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.text('Not connected to a player.'), findsOneWidget);
          await h.advance(const Duration(seconds: 30));
        },
        setup: sized,
        look: look,
      );

      appTest(
        'a new playlist, empty',
        (h) async {
          h.session.startNewPlaylist(longPlaylist);
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.text('No songs yet.'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Add songs',
        (h) async {
          await startEditing(h);
          await h.tester.tap(find.text('+ ADD SONGS'));
          await h.settle();
          await h.tester.pump(const Duration(seconds: 1));
          await h.tester.tap(find.text('Another'));
          await h.tester.pump(const Duration(milliseconds: 50));
          expect(find.text('1 chosen'), findsOneWidget);
          expect(find.text('in playlist'), findsWidgets);
        },
        setup: (rig) {
          sized(rig);
          rig.library = [
            ...rig.songs,
            RigSong('Another', lengthSeconds: 61),
            RigSong(
              'A song whose name is far too long for one line of a phone',
            ),
          ];
        },
        look: look,
      );

      appTest(
        'Settings, unlocking',
        (h) async {
          appVersion = () async => '0.1.0 (1)';
          await h.tester.tap(find.byTooltip('Settings'));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 400));
          expect(find.text('HOLD TO UNLOCK'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.locked = true;
        },
        look: look,
      );

      appTest(
        'Sections',
        (h) async {
          await h.openTab('Sections');
          expect(find.text('Chorus'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Sections, looping',
        (h) async {
          await h.openTab('Sections');
          expect(find.text('Stop looping'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.playing = true;
          rig.loop = 'A section with a very long name indeed';
        },
        look: look,
      );

      appTest(
        'Sections, none',
        (h) async {
          await h.openTab('Sections');
          expect(find.text('This song has no sections'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.index = 1;
        },
        look: look,
      );

      appTest(
        'Levels',
        (h) async {
          await h.openTab('Levels');
          expect(find.text('Cue'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.tracks = [
            RigTrack('A track with a name that runs on and on and on', -60),
            RigTrack('Backing L/R', 12, true),
            RigTrack('Cue', -3.5),
            RigTrack('Keys', 0),
            RigTrack('Bass synth', -12),
            RigTrack('Click', 0, true),
          ];
        },
        look: look,
      );

      appTest(
        'Levels, none',
        (h) async {
          await h.openTab('Levels');
          expect(find.textContaining('no output tracks'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.tracks = [];
        },
        look: look,
      );

      appTest(
        'Levels, too old',
        (h) async {
          await h.openTab('Levels');
          expect(find.textContaining('too old'), findsOneWidget);
        },
        setup: (rig) {
          sized(rig);
          rig.levelsAllowed = false;
        },
        look: look,
      );

      void lit(Rig rig) {
        sized(rig);
        rig.ambientMode = 'on';
        rig.ambientState = 'unavailable';
      }

      appTest(
        'Lights, waiting',
        (h) async {
          await h.openTab('Lights');
          expect(find.text('Ambient light'), findsOneWidget);
          expect(find.text('no ambient show'), findsOneWidget);
          expect(find.text(lightsWaitingMessage), findsOneWidget);
        },
        setup: lit,
        look: look,
      );

      appTest(
        'Lights, showing',
        (h) async {
          await showLights(h);
          expect(find.byKey(const Key('stage-surface')), findsOneWidget);
          expect(find.text(lightsWaitingMessage), findsNothing);
          expect(
            h.tester.getSize(find.byKey(const Key('stage-surface'))).height,
            greaterThan(200),
          );
        },
        setup: lit,
        look: look,
      );

      appTest(
        'Lights, with no ambient light',
        (h) async {
          await showLights(h);
          expect(find.text('Ambient light'), findsNothing);
        },
        setup: (rig) {
          sized(rig);
          rig.ambientMode = null;
        },
        look: look,
      );

      appTest(
        'Lights, failed',
        (h) async {
          await h.openTab('Lights');
          h.stage.surface.loadError('refused');
          await h.settle();
          expect(find.text(lightsFailedMessage), findsOneWidget);
          expect(find.text('Try again'), findsOneWidget);
        },
        setup: lit,
        look: look,
      );

      appTest(
        'Lights, too old',
        (h) async {
          await h.openTab('Lights');
          expect(find.text(lightsTooOldMessage), findsOneWidget);
        },
        setup: (rig) {
          lit(rig);
          rig.player.hasStreams = false;
        },
        look: look,
      );

      appTest(
        'Lights, without the player',
        (h) async {
          await showLights(h);
          h.rig.link.connectFailures = 100;
          h.rig.player.disconnect();
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.text('Reconnecting…'), findsOneWidget);
          expect(find.byKey(const Key('stage-dimmed')), findsOneWidget);
          await h.advance(const Duration(seconds: 30));
        },
        setup: lit,
        look: look,
      );

      appTest(
        'Lights, above the navigation bar',
        (h) async {
          h.tester.view.viewPadding = const FakeViewPadding(bottom: 48 * 3.0);
          h.tester.view.padding = const FakeViewPadding(bottom: 48 * 3.0);
          addTearDown(h.tester.view.resetViewPadding);
          addTearDown(h.tester.view.resetPadding);
          await showLights(h);
          final stage = h.tester.getRect(
            find.byKey(const Key('stage-surface')),
          );
          final tab = h.tester.getRect(find.text('Lights').last);
          expect(stage.bottom, lessThan(tab.top));
          expect(tab.bottom, lessThanOrEqualTo(640 - 48));
        },
        setup: lit,
        look: look,
      );

      appTest(
        'Settings',
        (h) async {
          appVersion = () async => '0.1.0 (1)';
          await h.tester.tap(find.byTooltip('Settings'));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 400));
          expect(find.text('Forget this player'), findsOneWidget);
        },
        setup: sized,
        look: look,
      );

      appTest(
        'Connect, finding',
        (h) async {
          h.rig.link.discover(
            const DiscoveredPlayer(id: 'a', name: longName, rssi: -50),
          );
          h.rig.link.discover(
            const DiscoveredPlayer(id: 'b', name: 'other', rssi: -90),
          );
          await h.settle();
          expect(find.text('other'), findsOneWidget);
        },
        known: false,
        look: look,
      );

      appTest(
        'Connect, Bluetooth off',
        (h) async {
          h.rig.link.adapter = AdapterState.off;
          await h.session.startScan();
          await h.settle();
          expect(find.text('Bluetooth is off'), findsOneWidget);
        },
        known: false,
        look: look,
      );

      appTest(
        'Connect, pairing',
        (h) async {
          h.rig.link.paired.clear();
          h.rig.link.pairingHeld = Completer<void>();
          h.rig.link.discover(
            const DiscoveredPlayer(id: 'p1', name: longName, rssi: -50),
          );
          await h.settle();
          await h.tester.tap(find.text(longName));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(find.textContaining('Pairing with'), findsOneWidget);
          h.rig.link.pairingHeld!.complete();
          await h.settle();
        },
        known: false,
        look: look,
      );

      appTest(
        'Connect, refused',
        (h) async {
          h.rig.link.paired.clear();
          h.rig.link.pairingRefused = true;
          h.rig.link.discover(
            const DiscoveredPlayer(id: 'p1', name: longName, rssi: -50),
          );
          await h.settle();
          await h.tester.tap(find.text(longName));
          await h.settle();
          await h.tester.pump(const Duration(milliseconds: 100));
          expect(
            find.text('This player is not accepting new phones'),
            findsOneWidget,
          );
        },
        known: false,
        look: look,
      );
    });
  }

  group('a short phone', () {
    appTest(
      'Levels still fit',
      (h) async {
        await h.openTab('Levels');
        expect(find.text('Cue'), findsOneWidget);
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'the editor still fits',
      (h) async {
        await h.openTab('Library');
        await h.tester.tap(find.text('Edit'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 400));
        expect(find.text('Editing'), findsOneWidget);
        expect(find.text('SAVE'), findsOneWidget);
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'Add songs still fits',
      (h) async {
        await h.openTab('Library');
        await h.tester.tap(find.text('Edit'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 400));
        await h.tester.tap(find.text('+ ADD SONGS'));
        await h.settle();
        await h.tester.pump(const Duration(seconds: 1));
        expect(find.text('Add songs'), findsOneWidget);
        expect(find.text('ADD SONGS'), findsOneWidget);
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'the playlist header still fits when locked',
      (h) async {
        await h.openTab('Library');
        expect(find.text('Unlock to edit'), findsOneWidget);
      },
      setup: (rig) => rig.locked = true,
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'Lights still fit, with the stage under the ambient light',
      (h) async {
        await showLights(h);
        expect(find.text('Ambient light'), findsOneWidget);
        expect(
          h.tester.getSize(find.byKey(const Key('stage-surface'))).height,
          greaterThan(120),
        );
      },
      setup: (rig) {
        rig.ambientMode = 'on';
        rig.ambientState = 'unavailable';
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'Lights, failed, still fit',
      (h) async {
        await h.openTab('Lights');
        h.stage.surface.loadError('refused');
        await h.settle();
        expect(find.text('Try again'), findsOneWidget);
      },
      setup: (rig) {
        rig.ambientMode = 'on';
        rig.ambientState = 'unavailable';
        rig.locked = true;
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'Lights, too old, still fit',
      (h) async {
        await h.openTab('Lights');
        expect(find.text(lightsTooOldMessage), findsOneWidget);
      },
      setup: (rig) {
        rig.ambientMode = 'on';
        rig.ambientState = 'unavailable';
        rig.locked = true;
        rig.player.hasStreams = false;
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );

    appTest(
      'Now still fits, scrolling if it must',
      (h) async {
        expect(find.text('PLAY'), findsOneWidget);
      },
      size: const Size(320, 480),
      look: const Look(Brightness.dark, 1.3),
    );
  });
}
