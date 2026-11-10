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
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';
import '../support/fake_player.dart';
import '../support/rig.dart';

Finder row(String name) =>
    find.ancestor(of: find.text(name), matching: find.byType(RowCard));

List<String> requests(AppHarness h, String method) => h.rig.player.received
    .where((r) => r.method == method)
    .map((r) => r.method)
    .toList();

void main() {
  group('the list', () {
    appTest('lists the songs with their numbers and lengths', (h) async {
      await h.openTab('Playlist');
      expect(find.text('First'), findsWidgets);
      expect(find.text('Second'), findsOneWidget);
      expect(find.text('Third'), findsOneWidget);
      expect(find.text('1:40'), findsOneWidget);
      expect(find.text('1:30'), findsOneWidget);
      expect(find.text('3:20'), findsOneWidget);
      expect(find.text('3 songs · 6 min'), findsOneWidget);
      expect(find.text('Set'), findsWidgets);
      expect(find.text('change'), findsOneWidget);
    });

    appTest('marks the current song', (h) async {
      final handle = h.tester.ensureSemantics();
      await h.openTab('Playlist');
      expect(
        find.bySemanticsLabel(RegExp(r'^1, First, 1:40, current song$')),
        findsOneWidget,
      );
      expect(find.bySemanticsLabel(RegExp(r'current song')), findsOneWidget);
      handle.dispose();
    });

    appTest('a tap makes a song current without playing it', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('Third'));
      await h.settle();
      expect(h.rig.index, 2);
      expect(h.rig.playing, isFalse);
      expect(h.session.position, 2);
      expect(requests(h, 'NavigateToSong'), hasLength(1));
      expect(requests(h, 'Play'), isEmpty);
    });

    appTest('a tap on the current song sends nothing', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('First').last);
      await h.settle();
      expect(requests(h, 'NavigateToSong'), isEmpty);
    });

    appTest('sends the entry and the name it shows', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('Second'));
      await h.settle();
      final request = pp.NavigateToSongRequest.fromBuffer(
        h.rig.player.received
            .firstWhere((r) => r.method == 'NavigateToSong')
            .payload,
      );
      expect(request.position, 1);
      expect(request.songName, 'Second');
    });

    appTest('a playlist that changed is shown as such and refreshed', (
      h,
    ) async {
      await h.openTab('Playlist');
      h.rig.songs[2] = RigSong('Swapped', lengthSeconds: 30);
      await h.tester.tap(find.text('Third'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.textContaining('playlist has changed'), findsOneWidget);
      await h.settle();
      expect(find.text('Swapped'), findsOneWidget);
      expect(find.text('Third'), findsNothing);
    });

    appTest('says why when the player refuses', (h) async {
      h.rig.locked = true;
      await h.openTab('Playlist');
      await h.tester.tap(find.text('Third'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(
        find.text('Player is locked. Unlock to make changes.'),
        findsOneWidget,
      );
    });

    appTest(
      'is inert while a song plays',
      (h) async {
        await h.openTab('Playlist');
        expect(find.text('Stop the song to choose another.'), findsOneWidget);
        final card = h.tester.widget<RowCard>(row('Third'));
        expect(card.onTap, isNull);
        expect(card.enabled, isFalse);
        await h.tester.tap(find.text('Third'));
        await h.settle();
        expect(requests(h, 'NavigateToSong'), isEmpty);
        expect(h.rig.index, 0);
        // The playlist chooser is just as shut.
        await h.tester.tap(find.text('change'));
        await h.settle();
        expect(find.text('Choose a playlist'), findsNothing);
      },
      setup: (rig) {
        rig.playing = true;
      },
    );

    appTest('shows an empty playlist as such', (h) async {
      h.rig.songs = [];
      h.rig.publish();
      await h.settle();
      await h.openTab('Playlist');
      expect(find.text('This playlist is empty'), findsOneWidget);
    });
  });

  group('a long list', () {
    List<RigSong> many() => [
      for (var i = 1; i <= 80; i++) RigSong('Song $i', lengthSeconds: 60 + i),
    ];

    appTest(
      'opens with the current song in view',
      (h) async {
        await h.openTab('Playlist');
        final current = find.text('Song 61');
        expect(current, findsOneWidget);
        final rect = h.tester.getRect(current);
        final view = h.tester.getRect(find.byType(ListView));
        expect(view.contains(rect.center), isTrue);
        expect(find.text('Song 1'), findsNothing);
      },
      setup: (rig) {
        rig.songs = many();
        rig.playlists['Set'] = rig.songs;
        rig.index = 60;
      },
    );

    appTest(
      'keeps the current song in view when it changes',
      (h) async {
        await h.openTab('Playlist');
        expect(find.text('Song 1'), findsOneWidget);
        h.rig.index = 40;
        h.rig.publish();
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 50));
        final rect = h.tester.getRect(find.text('Song 41'));
        expect(
          h.tester.getRect(find.byType(ListView)).contains(rect.center),
          isTrue,
        );
      },
      setup: (rig) {
        rig.songs = many();
        rig.playlists['Set'] = rig.songs;
      },
    );

    appTest(
      'keeps its scroll position while another tab shows',
      (h) async {
        await h.openTab('Playlist');
        await h.tester.drag(find.byType(ListView), const Offset(0, -600));
        await h.tester.pump();
        final before = h.tester.state<ScrollableState>(
          find.byType(Scrollable).first,
        );
        final offset = before.position.pixels;
        expect(offset, greaterThan(0));
        await h.openTab('Now');
        await h.openTab('Sections');
        await h.openTab('Now');
        expect(find.text('READY · 1 OF 80'), findsOneWidget);
      },
      setup: (rig) {
        rig.songs = many();
        rig.playlists['Set'] = rig.songs;
      },
    );
  });

  group('changing playlist', () {
    void others(Rig rig) {
      rig.playlists['Encore'] = [RigSong('Solo', lengthSeconds: 30)];
      rig.playlists['Acoustic'] = [
        RigSong('Hush', lengthSeconds: 70),
        RigSong('Low', lengthSeconds: 80),
      ];
    }

    appTest('lists the playlists, the active one marked', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('change'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Choose a playlist'), findsOneWidget);
      expect(find.text('Acoustic'), findsOneWidget);
      expect(find.text('Encore'), findsOneWidget);
      final active = h.tester.widget<RowCard>(row('Set').last);
      expect(active.mark, isNotNull);
      expect(h.tester.widget<RowCard>(row('Encore')).mark, isNull);
      expect(requests(h, 'ListPlaylists'), hasLength(1));
    }, setup: others);

    appTest('switches to the one tapped and closes', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('change'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      await h.tester.tap(find.text('Acoustic'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Choose a playlist'), findsNothing);
      expect(h.rig.playlist, 'Acoustic');
      expect(find.text('Hush'), findsWidgets);
      expect(find.text('2 songs · 2 min'), findsOneWidget);
    }, setup: others);

    appTest('stays open and says so when the switch is refused', (h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('change'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      h.rig.player.handlers['SwitchToPlaylist'] = (_) =>
          FakeReply.error(9, 'Cannot switch playlist while playing');
      await h.tester.tap(find.text('Acoustic'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Choose a playlist'), findsOneWidget);
      expect(find.text('Cannot switch playlist while playing'), findsOneWidget);
    }, setup: others);

    appTest('says when the playlists could not be loaded', (h) async {
      h.rig.player.handlers['ListPlaylists'] = (_) => FakeReply.error(14, 'x');
      await h.openTab('Playlist');
      await h.tester.tap(find.text('change'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Could not load the playlists.'), findsOneWidget);
    });

    appTest('knows a playlist by what the player said', (h) async {
      // The status names the playlist; entries come from GetPlaylist.
      expect(pp.GetPlaylistResponse().entries, isEmpty);
      await h.openTab('Playlist');
      expect(find.text('Set'), findsWidgets);
    });
  });
}
