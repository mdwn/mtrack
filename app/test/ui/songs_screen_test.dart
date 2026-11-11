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
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/playlist_settings.dart';
import 'package:mtrack_remote/src/app/song_edit.dart';
import 'package:mtrack_remote/src/ui/library_screen.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';
import 'package:mtrack_remote/src/ui/song_page.dart';
import 'package:mtrack_remote/src/ui/songs_screen.dart' show SongsView;
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';
import '../support/rig.dart';

Future<void> pumpFor(AppHarness h, [int ms = 400]) async {
  await h.settle();
  await h.tester.pump(Duration(milliseconds: ms));
}

/// Opens the Library tab's Songs side.
Future<void> openSongs(AppHarness h) async {
  await h.openTab('Library');
  await h.tester.tap(find.text('Songs'));
  await pumpFor(h);
}

/// Opens the page of [song] from the Songs side.
Future<void> openSong(AppHarness h, String song) async {
  await openSongs(h);
  await h.tester.tap(
    find.descendant(of: find.byType(SongsView), matching: find.text(song)),
  );
  await pumpFor(h);
}

/// The scroll position of the list showing (not the search field's).
ScrollPosition listScroll(AppHarness h) => h.tester
    .state<ScrollableState>(
      find
          .descendant(
            of: find.byType(ListView),
            matching: find.byType(Scrollable),
          )
          .first,
    )
    .position;

/// Which side of the Library tab the switch says is showing.
LibrarySide shownSide(AppHarness h) => h.tester
    .widget<ViewSwitch<LibrarySide>>(find.byType(ViewSwitch<LibrarySide>))
    .selected;

/// A library in folders, as a player sends it: two songs at the top (one
/// with a folder of its own, one in the songs directory itself), two
/// under "Live", one two folders down, and a folder that sorts before
/// "Live" only ignoring case.
void inFolders(Rig rig) {
  rig.library
    ..clear()
    ..addAll([
      RigSong('Alone', baseDir: 'Alone'),
      RigSong('Bare', baseDir: ''),
      RigSong('Encore', baseDir: 'Live/Encore'),
      RigSong('First', baseDir: 'Live/First'),
      RigSong('Deep', baseDir: 'Live/2026 tour/Deep'),
      RigSong('Acoustic', baseDir: 'acoustic/Acoustic'),
    ])
    ..sort((a, b) => a.name.compareTo(b.name));
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
    appTest('is the Songs side of the Library tab, with a search', (h) async {
      await openSongs(h);
      expect(shownSide(h), LibrarySide.songs);
      // In the tab, not over it.
      expect(find.text('Library'), findsOneWidget);
      expect(find.byTooltip('Back'), findsNothing);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('loops · samples'), findsOneWidget);
      await h.tester.enterText(find.byType(TextField), 'enc');
      await pumpFor(h, 50);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('First'), findsNothing);
      await h.tester.tap(find.byTooltip('Clear the search'));
      await pumpFor(h, 50);
      expect(find.text('First'), findsOneWidget);
      // A song's page is pushed over the tab, and Back comes back to it.
      await h.tester.tap(find.text('Encore'));
      await pumpFor(h);
      expect(find.byType(SongPage), findsOneWidget);
      await h.tester.tap(find.byTooltip('Back'));
      await h.tester.pumpAndSettle();
      expect(find.byType(SongPage), findsNothing);
      expect(shownSide(h), LibrarySide.songs);
    }, setup: withFull);

    appTest('is open while locked: reading is open', (h) async {
      await h.openTab('Library');
      expect(find.text('Unlock to edit'), findsOneWidget);
      await h.tester.tap(find.text('Songs'));
      await pumpFor(h);
      expect(find.text('Encore'), findsOneWidget);
    }, setup: (rig) => rig.locked = true);

    appTest('of a player without the lock says it is older', (h) async {
      await openSongs(h);
      expect(find.text(olderPlayerSongsNote), findsOneWidget);
      // The switch stays, and the playlist is a tap away.
      await h.tester.tap(find.text('Playlist'));
      await pumpFor(h);
      expect(find.text('Third'), findsOneWidget);
    }, setup: (rig) => rig.oldPlayer = true);

    appTest('of an older player says so, and the settings list it', (h) async {
      await openSongs(h);
      expect(find.text(olderPlayerSongsNote), findsOneWidget);
      expect(find.byType(ViewSwitch<LibrarySide>), findsOneWidget);
      appVersion = () async => '0.1.0 (1)';
      await h.tester.tap(find.byTooltip('Settings'));
      await pumpFor(h);
      expect(find.textContaining('the song library'), findsOneWidget);
    }, setup: (rig) => rig.songsTooOld = true);
  });

  group('the Library switch', () {
    appTest('starts on the playlist, without the Songs button', (h) async {
      await h.openTab('Library');
      expect(shownSide(h), LibrarySide.playlist);
      expect(find.text('Third'), findsOneWidget);
      // Edit is beside the playlist's name again, the only action there.
      final edit = h.tester.getRect(find.text('Edit'));
      final pick = h.tester.getRect(find.text('change'));
      expect(edit.center.dy, closeTo(pick.center.dy, 4));
      expect(find.text('Songs'), findsOneWidget);
      expect(h.views.values, isEmpty);
    });

    appTest('keeps the side chosen for the next launch', (h) async {
      await openSongs(h);
      expect(h.views.values[libraryViewKey], 'songs');
      await h.tester.tap(find.text('Playlist'));
      await pumpFor(h);
      expect(h.views.values[libraryViewKey], 'playlist');
    });

    appTest('opens on the side kept from the last launch', (h) async {
      await h.openTab('Library');
      await pumpFor(h);
      expect(shownSide(h), LibrarySide.songs);
      expect(find.text('Encore'), findsOneWidget);
      expect(
        h.rig.player.received.where((r) => r.method == 'ListSongs'),
        isNotEmpty,
      );
    }, views: {libraryViewKey: 'songs'});

    appTest('ignores a kept side it does not know', (h) async {
      await h.openTab('Library');
      expect(shownSide(h), LibrarySide.playlist);
    }, views: {libraryViewKey: 'sideways'});

    appTest(
      'keeps each side where it was scrolled to',
      (h) async {
        await h.openTab('Library');
        await h.tester.drag(find.byType(ListView), const Offset(0, -500));
        await pumpFor(h, 1000);
        final playlistAt = listScroll(h).pixels;
        expect(playlistAt, greaterThan(0));

        await h.tester.tap(find.text('Songs'));
        await pumpFor(h);
        await h.tester.drag(find.byType(ListView), const Offset(0, -700));
        await pumpFor(h, 1000);
        final songsAt = listScroll(h).pixels;
        expect(songsAt, greaterThan(0));

        await h.tester.tap(find.text('Playlist'));
        await pumpFor(h);
        expect(listScroll(h).pixels, playlistAt);
        await h.tester.tap(find.text('Songs'));
        await pumpFor(h);
        // Fetched again on the way back, and still where it was.
        expect(listScroll(h).pixels, songsAt);

        // Another tab and back: as left, too.
        await h.openTab('Now');
        await h.openTab('Library');
        expect(shownSide(h), LibrarySide.songs);
        expect(listScroll(h).pixels, songsAt);
      },
      setup: (rig) {
        rig.songs = [
          for (var i = 1; i <= 60; i++) RigSong('Song $i', lengthSeconds: 60),
        ];
        rig.playlists['Set'] = rig.songs;
        rig.library
          ..clear()
          ..addAll(rig.songs);
      },
    );

    appTest('is not shown while the playlist is edited', (h) async {
      await h.openTab('Library');
      await h.tester.tap(find.text('Edit'));
      await pumpFor(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.byType(ViewSwitch<LibrarySide>), findsNothing);
      await h.tester.tap(find.text('CANCEL'));
      await pumpFor(h);
      expect(shownSide(h), LibrarySide.playlist);
      expect(find.text('Edit'), findsOneWidget);
    });

    appTest('names each side to a screen reader, the one showing selected', (
      h,
    ) async {
      final handle = h.tester.ensureSemantics();
      await h.openTab('Library');
      final playlist = h.tester.getSemantics(
        find.bySemanticsLabel('Playlist').first,
      );
      expect(playlist, isSemantics(isSelected: true, isButton: true));
      final songs = h.tester.getSemantics(find.bySemanticsLabel('Songs'));
      expect(songs, isSemantics(isSelected: false, isButton: true));
      expect(find.bySemanticsLabel('Library'), findsOneWidget);
      handle.dispose();
    });
  });

  group('folders', () {
    appTest('group songs as the web UI does, the top first', (h) async {
      final handle = h.tester.ensureSemantics();
      await openSongs(h);
      final order = [
        find.text('Alone'),
        find.text('Bare'),
        find.text('ACOUSTIC'),
        find.text('Acoustic'),
        find.text('LIVE'),
        find.text('Encore'),
        find.text('First'),
        find.text('LIVE/2026 TOUR'),
        find.text('Deep'),
      ];
      for (final f in order) {
        expect(f, findsOneWidget, reason: '$f');
      }
      final tops = [for (final f in order) h.tester.getRect(f).top];
      expect([...tops]..sort(), tops);
      // The top has no heading of its own.
      expect(find.bySemanticsLabel(RegExp('^Folder ')), findsNWidgets(3));
      expect(
        find.bySemanticsLabel('Folder Live/2026 tour, 1 song'),
        findsOneWidget,
      );
      handle.dispose();
    }, setup: inFolders);

    appTest('close and open, and stay closed across the sides', (h) async {
      await openSongs(h);
      await h.tester.tap(find.text('LIVE'));
      await pumpFor(h, 50);
      expect(find.text('Encore'), findsNothing);
      expect(find.text('First'), findsNothing);
      // Its nested folder is a folder of its own.
      expect(find.text('Deep'), findsOneWidget);
      await h.tester.tap(find.text('Playlist'));
      await pumpFor(h);
      await h.tester.tap(find.text('Songs'));
      await pumpFor(h);
      expect(find.text('Encore'), findsNothing);
      await h.tester.tap(find.text('LIVE'));
      await pumpFor(h, 50);
      expect(find.text('Encore'), findsOneWidget);
    }, setup: inFolders);

    appTest('flatten in a search, each song saying its folder', (h) async {
      await openSongs(h);
      await h.tester.tap(find.text('LIVE'));
      await pumpFor(h, 50);
      await h.tester.enterText(find.byType(TextField), 'e');
      await pumpFor(h, 50);
      // Closed or not, a match is found, under no heading.
      expect(find.text('LIVE'), findsNothing);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('Live'), findsNWidgets(1));
      expect(find.text('Live/2026 tour'), findsOneWidget);
      expect(find.text('Deep'), findsOneWidget);
      // A song at the top says no folder.
      expect(find.text('Alone'), findsOneWidget);
      final handle = h.tester.ensureSemantics();
      expect(
        find.bySemanticsLabel(RegExp(r'^Deep, in Live/2026 tour')),
        findsOneWidget,
      );
      handle.dispose();
      await h.tester.enterText(find.byType(TextField), '');
      await pumpFor(h, 50);
      expect(find.text('LIVE'), findsOneWidget);
      expect(find.text('Encore'), findsNothing);
    }, setup: inFolders);

    appTest('are not drawn for a player that does not say them', (h) async {
      final handle = h.tester.ensureSemantics();
      await openSongs(h);
      expect(find.bySemanticsLabel(RegExp('^Folder ')), findsNothing);
      handle.dispose();
      for (final song in ['Encore', 'First', 'Opener', 'Second', 'Third']) {
        expect(find.text(song), findsOneWidget);
      }
    });

    appTest('say where a song is on its page', (h) async {
      await openSongs(h);
      await h.tester.enterText(find.byType(TextField), 'deep');
      await pumpFor(h, 50);
      await h.tester.tap(find.text('Deep'));
      await pumpFor(h);
      expect(find.text('Live/2026 tour/Deep'), findsOneWidget);
    }, setup: inFolders);
  });

  group('the library on a small phone', () {
    for (final scale in [1.3, 2.0]) {
      appTest(
        'fits the switch, folders and search at 320 wide and ${scale}x text',
        (h) async {
          await openSongs(h);
          expect(h.tester.takeException(), isNull);
          final width =
              h.tester.view.physicalSize.width / h.tester.view.devicePixelRatio;
          for (final label in ['Playlist', 'Songs']) {
            final text = find.text(label);
            final rect = h.tester.getRect(text);
            expect(rect.right, lessThanOrEqualTo(width), reason: label);
            final paragraph = h.tester.renderObject<RenderParagraph>(text);
            expect(paragraph.didExceedMaxLines, isFalse, reason: label);
          }
          final long = find.text(
            'LIVE/A FOLDER WITH A NAME FAR TOO LONG FOR A PHONE',
          );
          await h.tester.scrollUntilVisible(
            long,
            100,
            scrollable: find
                .descendant(
                  of: find.byType(ListView),
                  matching: find.byType(Scrollable),
                )
                .first,
          );
          await pumpFor(h, 50);
          expect(h.tester.getRect(long).right, lessThanOrEqualTo(width));
          expect(h.tester.takeException(), isNull);
          await h.tester.enterText(find.byType(TextField), 'deep');
          await pumpFor(h, 50);
          expect(find.text('Live/2026 tour'), findsOneWidget);
          await h.tester.tap(find.text('Playlist'));
          await pumpFor(h);
          expect(find.text('change'), findsOneWidget);
          expect(h.tester.takeException(), isNull);
        },
        setup: (rig) {
          inFolders(rig);
          rig.library.add(
            RigSong(
              'A song with a name far too long for a phone held upright',
              baseDir:
                  'Live/A folder with a name far too long for a phone/'
                  'A song with a name far too long for a phone held upright',
            ),
          );
        },
        look: Look(Brightness.dark, scale),
        size: const Size(320, 480),
      );
    }
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
              of: find.byType(SongsView),
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
              of: find.byType(SongsView),
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
