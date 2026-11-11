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
import 'package:mtrack_remote/src/app/playlist_settings.dart';
import 'package:mtrack_remote/src/app/song_edit.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';
import 'package:mtrack_remote/src/ui/song_page.dart';
import 'package:mtrack_remote/src/ui/songs_screen.dart' show SongsScreen;
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';
import '../support/rig.dart';

Future<void> pumpFor(AppHarness h, [int ms = 400]) async {
  await h.settle();
  await h.tester.pump(Duration(milliseconds: ms));
}

/// Opens the Songs screen from the Playlist tab.
Future<void> openSongs(AppHarness h) async {
  await h.openTab('Playlist');
  await h.tester.tap(find.text('Songs'));
  await pumpFor(h);
}

/// Opens the page of [song] from the Songs screen.
Future<void> openSong(AppHarness h, String song) async {
  await openSongs(h);
  await h.tester.tap(
    find.descendant(of: find.byType(SongsScreen), matching: find.text(song)),
  );
  await pumpFor(h);
}

/// Brings [target] into view: built by scrolling the song's page down to
/// it, when the page has not built it yet.
Future<void> reveal(AppHarness h, Finder target) async {
  if (target.evaluate().isEmpty) {
    final page = find
        .descendant(
          of: find.byType(SongPage),
          matching: find.byType(Scrollable),
        )
        .first;
    // From the top, down to it.
    h.tester.state<ScrollableState>(page).position.jumpTo(0);
    await h.tester.pump();
    await h.tester.scrollUntilVisible(target, 120, scrollable: page);
  }
  await h.tester.ensureVisible(target);
  await h.tester.pumpAndSettle();
}

/// Taps [target], scrolled into view first.
Future<void> tapShown(AppHarness h, Finder target) async {
  await reveal(h, target);
  await h.tester.tap(target);
  await pumpFor(h);
}

/// Expects [text] once on the song's page, scrolling to it.
Future<void> expectShown(AppHarness h, String text) async {
  await reveal(h, find.text(text));
  expect(find.text(text), findsOneWidget);
}

pp.UpdateSongSettingsRequest lastUpdate(AppHarness h) =>
    pp.UpdateSongSettingsRequest.fromBuffer(
      h.rig.player.received
          .lastWhere((r) => r.method == 'UpdateSongSettings')
          .payload,
    );

void withFull(Rig rig) => rig.songSettings['First'] = fullSongSettings();

void main() {
  group('the library', () {
    appTest('opens from the Playlist tab, with a search', (h) async {
      await openSongs(h);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('loops · samples'), findsOneWidget);
      await h.tester.enterText(find.byType(TextField), 'enc');
      await pumpFor(h, 50);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('First'), findsNothing);
      await h.tester.tap(find.byTooltip('Back'));
      await h.tester.pumpAndSettle();
      expect(find.byType(SongsScreen), findsNothing);
      expect(find.text('Songs'), findsOneWidget);
    }, setup: withFull);

    appTest('is offered while locked: reading is open', (h) async {
      await h.openTab('Playlist');
      expect(find.text('Unlock to edit'), findsOneWidget);
      await h.tester.tap(find.text('Songs'));
      await pumpFor(h);
      expect(find.text('Encore'), findsOneWidget);
    }, setup: (rig) => rig.locked = true);

    appTest('is not offered by a player without the lock', (h) async {
      await h.openTab('Playlist');
      expect(find.text('Songs'), findsNothing);
    }, setup: (rig) => rig.oldPlayer = true);

    appTest('of an older player says so, and the settings list it', (h) async {
      await openSongs(h);
      expect(find.text(olderPlayerSongsNote), findsOneWidget);
      await h.tester.tap(find.byTooltip('Back'));
      await h.tester.pumpAndSettle();
      appVersion = () async => '0.1.0 (1)';
      await h.tester.tap(find.byTooltip('Settings'));
      await pumpFor(h);
      expect(find.textContaining('the song library'), findsOneWidget);
    }, setup: (rig) => rig.songsTooOld = true);
  });

  group('a song', () {
    appTest('shows what it has, and saves a loop change', (h) async {
      await openSong(h, 'First');
      await expectShown(h, 'Tempo: 120 bpm, 4/4');
      await expectShown(h, 'Loop Playback');
      await expectShown(h, 'keys.wav · channel 2');
      await expectShown(h, 'Note On, channel 10, key 36 → kick');
      // Nothing to save yet.
      ChunkyButton saveButton() => h.tester.widget<ChunkyButton>(
        find.widgetWithText(ChunkyButton, 'SAVE'),
      );
      expect(saveButton().onPressed, isNull);
      await tapShown(h, find.byType(Switch));
      expect(saveButton().onPressed, isNotNull);
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 100);
      final sent = lastUpdate(h);
      expect(sent.looping.loopPlayback, isFalse);
      expect(sent.hasTracks(), isFalse);
      expect(h.rig.songSettings['First']!.looping.loopPlayback, isFalse);
    }, setup: withFull);

    appTest('a track is changed, added and removed', (h) async {
      await openSong(h, 'First');
      // Point keys at its first channel.
      await tapShown(h, find.text('keys'));
      await h.tester.enterText(find.widgetWithText(TextField, 'Channel'), '1');
      await tapShown(h, find.text('DONE'));
      await expectShown(h, 'keys.wav · channel 1');
      // A new one, named after the file chosen.
      await tapShown(h, find.text('+ ADD TRACK'));
      await tapShown(h, find.text('bass.wav · mono'));
      await tapShown(h, find.text('DONE'));
      await expectShown(h, 'bass');
      // Removing asks first.
      await tapShown(h, find.text('click'));
      await tapShown(h, find.text('REMOVE TRACK'));
      expect(find.text('Remove the track click?'), findsOneWidget);
      await h.tester.tap(find.text('Remove'));
      await pumpFor(h);
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 100);
      final sent = lastUpdate(h);
      expect(sent.tracks.tracks.map((t) => (t.name, t.fileChannel)), [
        ('keys', 1),
        ('bass', 0),
      ]);
      expect(sent.tracks.tracks[1].hasFileChannel(), isFalse);
      // The click was cut at the crossfade's midpoint; it went with it.
      expect(sent.looping.cut, isEmpty);
    }, setup: withFull);

    appTest('a sample is changed, its velocity shown and kept', (h) async {
      await openSong(h, 'First');
      await tapShown(h, find.text('kick'));
      expect(
        find.textContaining('Velocity: 2 layers, scaled by velocity'),
        findsOneWidget,
      );
      await tapShown(h, find.widgetWithText(ChoiceChip, 'Fade'));
      await tapShown(h, find.widgetWithText(ChoiceChip, 'Polyphonic'));
      await h.tester.enterText(
        find.widgetWithText(TextField, 'Max Voices'),
        '4',
      );
      await tapShown(h, find.text('DONE'));
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 100);
      final sample = lastUpdate(h).samples.samples.single;
      expect(sample.originalName, 'kick');
      expect(sample.releaseBehavior, 'fade');
      expect(sample.retrigger, 'polyphonic');
      expect(sample.maxVoices, 4);
      expect(sample.outputChannels, [1, 2]);
    }, setup: withFull);

    appTest('a crossfade that cannot be a length is caught on Done', (h) async {
      await openSong(h, 'First');
      await tapShown(h, find.text('Loop crossfade'));
      await h.tester.enterText(
        find.widgetWithText(TextField, '2measures'),
        '2 bananas',
      );
      await tapShown(h, find.text('DONE'));
      expect(find.text(notALengthMessage), findsOneWidget);
      await h.tester.enterText(
        find.widgetWithText(TextField, '2 bananas'),
        '6s',
      );
      await tapShown(h, find.text('DONE'));
      expect(find.textContaining('6s'), findsOneWidget);
    }, setup: withFull);

    appTest('a refusal is shown in the player\'s words', (h) async {
      await openSong(h, 'First');
      await tapShown(h, find.text('Loop crossfade'));
      // Looks like a length to the phone; the player says otherwise.
      await h.tester.enterText(
        find.widgetWithText(TextField, '2measures'),
        '0.5.5s',
      );
      await tapShown(h, find.text('DONE'));
      if (find.text(notALengthMessage).evaluate().isNotEmpty) {
        // The lenient check caught it; one it lets through is sent.
        await h.tester.enterText(
          find.widgetWithText(TextField, '0.5.5s'),
          '2m30s',
        );
        await tapShown(h, find.text('DONE'));
      }
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 100);
      expect(find.textContaining('The player would not save this'), findsOne);
      expect(find.textContaining('loop_crossfade'), findsOneWidget);
    }, setup: withFull);

    appTest('changed elsewhere offers save anyway and discard', (h) async {
      await openSong(h, 'First');
      await tapShown(h, find.byType(Switch));
      h.rig.editSongElsewhere('First', (song) => song.looping.cut.clear());
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 100);
      expect(find.text(songChangedMessage), findsOneWidget);
      await tapShown(h, find.text('DISCARD MY CHANGES'));
      expect(find.text(songChangedMessage), findsNothing);
      expect(h.rig.songUpdates, isEmpty);
      // As the player has it now.
      expect(find.textContaining('switch at the midpoint'), findsNothing);
    }, setup: withFull);

    appTest(
      'is read only while locked, with the way to unlock',
      (h) async {
        await openSong(h, 'First');
        expect(find.text(lockedSongNote), findsOneWidget);
        expect(find.text('+ ADD TRACK'), findsNothing);
        final toggle = h.tester.widget<Switch>(find.byType(Switch));
        expect(toggle.onChanged, isNull);
        appVersion = () async => '0.1.0 (1)';
        await h.tester.tap(find.text('Unlock'));
        await pumpFor(h);
        expect(find.text('Player lock'), findsOneWidget);
      },
      setup: (rig) {
        withFull(rig);
        rig.locked = true;
      },
    );

    appTest(
      'playing, it says when a change takes effect',
      (h) async {
        await openSong(h, 'First');
        expect(find.text(playingSongNote), findsOneWidget);
      },
      setup: (rig) {
        withFull(rig);
        rig.playing = true;
      },
    );

    appTest('leaving with changes asks first', (h) async {
      await openSong(h, 'First');
      await tapShown(h, find.byType(Switch));
      await h.tester.tap(find.byTooltip('Back'));
      await pumpFor(h);
      expect(find.text('Discard your changes?'), findsOneWidget);
      expect(
        find.text('The song on the player has not been changed.'),
        findsOneWidget,
      );
      await h.tester.tap(find.text('Discard'));
      await pumpFor(h);
      expect(find.text('Encore'), findsOneWidget);
    }, setup: withFull);
  });

  group('on a small phone', () {
    for (final look in Look.all.followedBy([
      const Look(Brightness.dark, 2.0),
    ])) {
      appTest(
        'every part fits above the navigation bar and the keyboard, $look',
        (h) async {
          h.tester.view.viewPadding = const FakeViewPadding(bottom: 48 * 3.0);
          h.tester.view.padding = const FakeViewPadding(bottom: 48 * 3.0);
          addTearDown(h.tester.view.resetViewPadding);
          addTearDown(h.tester.view.resetPadding);
          final screen =
              h.tester.view.physicalSize.height /
              h.tester.view.devicePixelRatio;
          final bar = 48 * 3.0 / h.tester.view.devicePixelRatio;

          Future<void> reachable(String text) async {
            final button = find.text(text);
            await reveal(h, button);
            final rect = h.tester.getRect(button);
            expect(rect.top, greaterThanOrEqualTo(0), reason: text);
            expect(rect.bottom, lessThanOrEqualTo(screen - bar), reason: text);
          }

          await openSongs(h);
          // Searching, with the keyboard up.
          await h.tester.tap(find.byType(TextField));
          h.tester.view.viewInsets = const FakeViewPadding(bottom: 260 * 3.0);
          await h.tester.pumpAndSettle();
          await h.tester.enterText(find.byType(TextField), 'fir');
          await h.tester.pumpAndSettle();
          expect(
            find.descendant(
              of: find.byType(SongsScreen),
              matching: find.text('First'),
            ),
            findsOneWidget,
          );
          h.tester.view.resetViewInsets();
          FocusManager.instance.primaryFocus?.unfocus();
          await h.tester.pumpAndSettle();
          expect(h.tester.takeException(), isNull);
          await h.tester.tap(
            find.descendant(
              of: find.byType(SongsScreen),
              matching: find.text('First'),
            ),
          );
          await pumpFor(h);
          await tapShown(h, find.byType(Switch));
          await reachable('SAVE');
          await reachable('+ ADD SAMPLE');

          // Each sheet, with the keyboard up over the bottom of it.
          for (final open in ['Loop crossfade', 'keys', 'kick']) {
            await tapShown(h, find.text(open));
            final field = find.byType(TextField).first;
            await h.tester.tap(field);
            h.tester.view.viewInsets = const FakeViewPadding(bottom: 260 * 3.0);
            await h.tester.pumpAndSettle();
            const keyboard = 260.0;
            final done = find.text('DONE');
            await h.tester.ensureVisible(done);
            await h.tester.pumpAndSettle();
            expect(
              h.tester.getRect(done).bottom,
              lessThanOrEqualTo(screen - keyboard),
              reason: open,
            );
            h.tester.view.resetViewInsets();
            await h.tester.pumpAndSettle();
            await reachable('DONE');
            await h.tester.tap(find.text('CANCEL'));
            await pumpFor(h);
          }
          expect(h.tester.takeException(), isNull);
        },
        setup: withFull,
        look: look,
        size: const Size(320, 480),
      );
    }
  });
}
