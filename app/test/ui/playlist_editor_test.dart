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
import 'package:mtrack_remote/src/ui/hold_to_stop.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';

import '../support/app_harness.dart';
import '../support/rig.dart';

int calls(AppHarness h, String method) =>
    h.rig.player.received.where((r) => r.method == method).length;

pp.SavePlaylistRequest lastSave(AppHarness h) =>
    pp.SavePlaylistRequest.fromBuffer(
      h.rig.player.received
          .lastWhere((r) => r.method == 'SavePlaylist')
          .payload,
    );

Future<void> pumpFor(AppHarness h, [int ms = 400]) async {
  await h.settle();
  await h.tester.pump(Duration(milliseconds: ms));
}

Future<void> openSettings(AppHarness h) async {
  appVersion = () async => '0.1.0 (1)';
  await h.tester.tap(find.byTooltip('Settings'));
  await pumpFor(h);
}

Future<void> hold(AppHarness h, Finder target, {bool full = true}) async {
  final press = await h.tester.startGesture(h.tester.getCenter(target));
  await h.tester.pump();
  await h.advance(
    full
        ? holdToStopDuration + const Duration(milliseconds: 100)
        : holdToStopDuration ~/ 3,
  );
  await press.up();
  await pumpFor(h, 50);
}

/// Opens the Playlist tab and starts editing.
Future<void> startEditing(AppHarness h) async {
  await h.openTab('Playlist');
  await h.tester.tap(find.text('Edit'));
  await pumpFor(h);
}

Finder remove(String song) => find.byTooltip('Remove $song');

Future<void> save(AppHarness h) async {
  await h.tester.tap(find.text('SAVE'));
  await pumpFor(h, 100);
}

void main() {
  group('the lock pill', () {
    appTest('says Unlocked on every screen', (h) async {
      for (final tab in ['Now', 'Playlist', 'Sections', 'Levels']) {
        await h.openTab(tab);
        expect(find.text('Unlocked'), findsOneWidget, reason: tab);
        expect(find.text('Locked'), findsNothing, reason: tab);
      }
    });

    appTest('says Locked, and follows the player', (h) async {
      expect(find.text('Locked'), findsOneWidget);
      h.rig.locked = false;
      h.rig.publish();
      await pumpFor(h, 50);
      expect(find.text('Unlocked'), findsOneWidget);
      expect(find.text('Locked'), findsNothing);
    }, setup: (rig) => rig.locked = true);

    appTest('is not there for a player that has no lock', (h) async {
      expect(find.text('Locked'), findsNothing);
      expect(find.text('Unlocked'), findsNothing);
    }, setup: (rig) => rig.oldPlayer = true);

    appTest('opens the settings, where the lock is', (h) async {
      appVersion = () async => '0.1.0 (1)';
      await h.tester.tap(find.text('Unlocked'));
      await pumpFor(h);
      expect(find.text('Player lock'), findsOneWidget);
    });

    appTest('is a 48 px target and says what it is', (h) async {
      final handle = h.tester.ensureSemantics();
      final size = h.tester.getSize(find.bySemanticsLabel('Player unlocked'));
      expect(size.height, greaterThanOrEqualTo(48));
      handle.dispose();
    });
  });

  group('the lock card', () {
    appTest('unlocks on a hold', (h) async {
      await openSettings(h);
      expect(find.text('Player lock'), findsOneWidget);
      expect(find.text('locked'), findsOneWidget);
      await hold(h, find.text('HOLD TO UNLOCK'));
      expect(h.rig.locked, isFalse);
      expect(calls(h, 'SetLock'), 1);
      expect(find.text('unlocked'), findsOneWidget);
      expect(find.text('LOCK'), findsOneWidget);
    }, setup: (rig) => rig.locked = true);

    appTest('does not unlock on a tap or a short press', (h) async {
      await openSettings(h);
      await h.tester.tap(find.text('HOLD TO UNLOCK'));
      await pumpFor(h, 50);
      expect(find.text('KEEP HOLDING'), findsOneWidget);
      await hold(h, find.text('KEEP HOLDING'), full: false);
      expect(calls(h, 'SetLock'), 0);
      expect(h.rig.locked, isTrue);
    }, setup: (rig) => rig.locked = true);

    appTest('locks on a tap', (h) async {
      await openSettings(h);
      expect(find.text('unlocked'), findsOneWidget);
      await h.tester.tap(find.text('LOCK'));
      await pumpFor(h, 50);
      expect(h.rig.locked, isTrue);
      expect(find.text('HOLD TO UNLOCK'), findsOneWidget);
    });

    appTest('stays unlocked: nothing relocks it', (h) async {
      await h.advance(const Duration(hours: 1));
      expect(h.rig.locked, isFalse);
      expect(calls(h, 'SetLock'), 0);
    });

    appTest('is left out for a player that has no lock', (h) async {
      await openSettings(h);
      expect(find.text('Player lock'), findsNothing);
      expect(find.text('Ambient light'), findsOneWidget);
    }, setup: (rig) => rig.oldPlayer = true);

    appTest('says what each button is to a screen reader', (h) async {
      final handle = h.tester.ensureSemantics();
      await openSettings(h);
      final unlock = h.tester.semantics.find(
        find.bySemanticsLabel('Hold to unlock the player'),
      );
      expect(
        unlock.getSemanticsData().hasAction(SemanticsAction.longPress),
        isTrue,
      );
      expect(
        h.tester
            .getSize(find.bySemanticsLabel('Hold to unlock the player'))
            .height,
        greaterThanOrEqualTo(48),
      );
      handle.dispose();
    }, setup: (rig) => rig.locked = true);

    appTest('names the lock button when unlocked', (h) async {
      final handle = h.tester.ensureSemantics();
      await openSettings(h);
      expect(find.bySemanticsLabel('Lock the player'), findsOneWidget);
      handle.dispose();
    });
  });

  group('the way to edit', () {
    appTest('is Edit while unlocked', (h) async {
      await h.openTab('Playlist');
      expect(find.text('Edit'), findsOneWidget);
      expect(find.text('Unlock to edit'), findsNothing);
      expect(find.text('change'), findsOneWidget);
    });

    appTest('is "Unlock to edit" while locked, and opens the settings', (
      h,
    ) async {
      await h.openTab('Playlist');
      expect(find.text('Edit'), findsNothing);
      appVersion = () async => '0.1.0 (1)';
      await h.tester.tap(find.text('Unlock to edit'));
      await pumpFor(h);
      expect(find.text('Player lock'), findsOneWidget);
    }, setup: (rig) => rig.locked = true);

    appTest('is not there for a player that has no lock', (h) async {
      await h.openTab('Playlist');
      expect(find.text('Edit'), findsNothing);
      expect(find.text('Unlock to edit'), findsNothing);
      expect(find.text('change'), findsOneWidget);
    }, setup: (rig) => rig.oldPlayer = true);

    appTest(
      'is not there for all_songs, locked or not',
      (h) async {
        await h.openTab('Playlist');
        expect(find.text('Edit'), findsNothing);
        expect(find.text('Unlock to edit'), findsNothing);
      },
      setup: (rig) {
        rig.playlist = 'all_songs';
        rig.songs = rig.library;
      },
    );

    appTest('appears when the player is unlocked', (h) async {
      await h.openTab('Playlist');
      expect(find.text('Unlock to edit'), findsOneWidget);
      h.rig.locked = false;
      h.rig.publish();
      await pumpFor(h, 50);
      expect(find.text('Edit'), findsOneWidget);
    }, setup: (rig) => rig.locked = true);

    appTest('is a 48 px target', (h) async {
      final handle = h.tester.ensureSemantics();
      await h.openTab('Playlist');
      final size = h.tester.getSize(find.bySemanticsLabel('Edit playlist'));
      expect(size.height, greaterThanOrEqualTo(48));
      handle.dispose();
    });
  });

  group('the editor', () {
    appTest('covers the tabs, and Cancel without changes just leaves', (
      h,
    ) async {
      expect(find.text('Levels'), findsOneWidget);
      await startEditing(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('3 songs · 6 min'), findsOneWidget);
      for (final tab in ['Now', 'Sections', 'Levels']) {
        expect(find.text(tab), findsNothing, reason: tab);
      }
      await h.tester.tap(find.text('CANCEL'));
      await pumpFor(h);
      expect(find.text('Discard your changes?'), findsNothing);
      expect(find.text('Editing'), findsNothing);
      expect(find.text('Levels'), findsOneWidget);
      expect(calls(h, 'SavePlaylist'), 0);
    });

    appTest('shows the songs, the current one marked, and a live total', (
      h,
    ) async {
      final handle = h.tester.ensureSemantics();
      await startEditing(h);
      for (final name in ['First', 'Second', 'Third']) {
        expect(find.text(name), findsOneWidget);
        expect(find.bySemanticsLabel('Reorder $name'), findsOneWidget);
      }
      expect(find.bySemanticsLabel('First, current song'), findsOneWidget);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
      expect(find.text('Second'), findsNothing);
      handle.dispose();
    });

    appTest('keeps its controls at least 48 px', (h) async {
      await startEditing(h);
      final handle = h.tester.getSize(find.byIcon(Icons.drag_indicator).first);
      expect(handle.width, greaterThanOrEqualTo(48));
      expect(handle.height, greaterThanOrEqualTo(48));
      final x = h.tester.getSize(remove('First'));
      expect(x.width, greaterThanOrEqualTo(48));
      expect(x.height, greaterThanOrEqualTo(48));
    });

    appTest('Save is off until something changes', (h) async {
      await startEditing(h);
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 50);
      expect(calls(h, 'SavePlaylist'), 0);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await save(h);
      expect(calls(h, 'SavePlaylist'), 1);
    });

    appTest('Save sends the list and returns to the saved playlist', (h) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await save(h);
      expect(lastSave(h).songs, ['First', 'Third']);
      expect(lastSave(h).expected, ['First', 'Second', 'Third']);
      expect(find.text('Editing'), findsNothing);
      expect(find.text('Levels'), findsOneWidget);
      expect(find.text('Second'), findsNothing);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
    });

    appTest('a drag on the handle reorders', (h) async {
      await startEditing(h);
      await h.tester.drag(
        find.byIcon(Icons.drag_indicator).first,
        const Offset(0, 130),
      );
      await pumpFor(h, 400);
      await h.tester.tap(remove('Third'));
      await pumpFor(h, 50);
      await save(h);
      expect(lastSave(h).songs, ['Second', 'First']);
    });

    appTest('can be reordered without dragging, by a screen reader', (h) async {
      final handle = h.tester.ensureSemantics();
      await startEditing(h);
      // The list gives each row its move actions; they sit on a node above
      // the row's own.
      SemanticsNode? node = h.tester.getSemantics(find.text('First'));
      Map<String, int> actions = {};
      while (node != null) {
        final ids = node.getSemanticsData().customSemanticsActionIds ?? [];
        actions = {
          for (final id in ids)
            CustomSemanticsAction.getAction(id)?.label ?? '?': id,
        };
        if (actions.containsKey('Move down')) {
          break;
        }
        node = node.parent;
      }
      expect(node, isNotNull, reason: 'no row offers "Move down"');
      expect(actions.keys, containsAll(['Move down']));
      expect(actions.keys, isNot(contains('Move up')), reason: 'first row');
      node!.owner!.performAction(
        node.id,
        SemanticsAction.customAction,
        actions['Move down'],
      );
      await pumpFor(h, 50);
      await save(h);
      expect(lastSave(h).songs, ['Second', 'First', 'Third']);
      handle.dispose();
    });

    appTest('back with changes asks, and keeping goes on editing', (h) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await h.tester.binding.handlePopRoute();
      await pumpFor(h);
      expect(find.text('Discard your changes?'), findsOneWidget);
      await h.tester.tap(find.text('Keep editing'));
      await pumpFor(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('Second'), findsNothing);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
    });

    appTest('back with changes, then Discard, leaves without saving', (
      h,
    ) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await h.tester.binding.handlePopRoute();
      await pumpFor(h);
      await h.tester.tap(find.text('Discard'));
      await pumpFor(h);
      expect(find.text('Editing'), findsNothing);
      expect(calls(h, 'SavePlaylist'), 0);
      expect(find.text('Second'), findsOneWidget);
    });

    appTest('back without changes leaves at once', (h) async {
      await startEditing(h);
      await h.tester.binding.handlePopRoute();
      await pumpFor(h);
      expect(find.text('Discard your changes?'), findsNothing);
      expect(find.text('Editing'), findsNothing);
    });

    appTest('Cancel with changes asks too', (h) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await h.tester.tap(find.text('CANCEL'));
      await pumpFor(h);
      expect(find.text('Discard your changes?'), findsOneWidget);
    });

    appTest('says the playlist changed, and offers both ways on', (h) async {
      await startEditing(h);
      h.rig.editElsewhere('Set', ['Third', 'First']);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await save(h);
      expect(
        find.text(
          'This playlist was changed on the player while you were editing.',
        ),
        findsOneWidget,
      );
      expect(find.text('2 songs · 5 min'), findsOneWidget);
      expect(find.text('Editing'), findsOneWidget);
      await h.tester.tap(find.text('SAVE ANYWAY'));
      await pumpFor(h, 100);
      expect(lastSave(h).expected, ['Third', 'First']);
      expect(lastSave(h).songs, ['First', 'Third']);
      expect(find.text('Editing'), findsNothing);
    });

    appTest('Discard my changes loads what the player has', (h) async {
      await startEditing(h);
      h.rig.editElsewhere('Set', ['Third', 'First']);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      await save(h);
      await h.tester.tap(find.text('DISCARD MY CHANGES'));
      await pumpFor(h, 100);
      expect(find.textContaining('was changed on the player'), findsNothing);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
      expect(find.text('Third'), findsOneWidget);
      expect(calls(h, 'SavePlaylist'), 1);
    });

    appTest('says so when the player is locked, and keeps the edits', (
      h,
    ) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      h.rig.locked = true;
      h.rig.publish();
      await pumpFor(h, 50);
      expect(
        find.text('The player is locked. Unlock it to save.'),
        findsOneWidget,
      );
      expect(find.text('Locked'), findsOneWidget);
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 50);
      expect(calls(h, 'SavePlaylist'), 0);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
      // Unlocking from the pill brings Save back.
      appVersion = () async => '0.1.0 (1)';
      await h.tester.tap(find.text('Locked'));
      await pumpFor(h);
      await hold(h, find.text('HOLD TO UNLOCK'));
      await h.tester.binding.handlePopRoute();
      await pumpFor(h);
      expect(
        find.text('The player is locked. Unlock it to save.'),
        findsNothing,
      );
      await save(h);
      expect(lastSave(h).songs, ['First', 'Third']);
    });

    appTest('a save the player refuses as locked shows the same words', (
      h,
    ) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      h.rig.locked = true; // no status yet
      await save(h);
      expect(
        find.text('The player is locked. Unlock it to save.'),
        findsOneWidget,
      );
      expect(find.text('Editing'), findsOneWidget);
    });

    appTest('keeps the edits when the link drops, and says why Save waits', (
      h,
    ) async {
      await startEditing(h);
      await h.tester.tap(remove('Second'));
      await pumpFor(h, 50);
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await pumpFor(h, 100);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('Not connected to a player.'), findsOneWidget);
      expect(find.text('2 songs · 5 min'), findsOneWidget);
      await h.tester.tap(find.text('SAVE'));
      await pumpFor(h, 50);
      expect(calls(h, 'SavePlaylist'), 0);
      expect(find.text('Editing'), findsOneWidget);
    });
  });

  group('adding songs', () {
    Future<void> openAdd(AppHarness h) async {
      await startEditing(h);
      await h.tester.tap(find.text('+ ADD SONGS'));
      // The page transition has to finish before the editor underneath is
      // off screen.
      await pumpFor(h, 1000);
    }

    appTest('lists the library, with the ones in the playlist dimmed', (
      h,
    ) async {
      await openAdd(h);
      expect(find.text('Add songs'), findsOneWidget);
      for (final name in ['Encore', 'First', 'Opener', 'Second', 'Third']) {
        expect(find.text(name), findsOneWidget, reason: name);
      }
      expect(find.text('in playlist'), findsNWidgets(3));
      expect(find.text('Search 5 songs'), findsOneWidget);
      expect(find.text('ADD SONGS'), findsOneWidget);
      // Alphabetical, as the player gives them.
      final tops = [
        for (final n in ['Encore', 'First', 'Opener', 'Second', 'Third'])
          h.tester.getTopLeft(find.text(n)).dy,
      ];
      expect([...tops]..sort(), tops);
    });

    appTest('searches in any case', (h) async {
      await openAdd(h);
      await h.tester.enterText(find.byType(TextField), 'ENC');
      await pumpFor(h, 50);
      expect(find.text('Encore'), findsOneWidget);
      expect(find.text('Opener'), findsNothing);
      await h.tester.enterText(find.byType(TextField), 'zzz');
      await pumpFor(h, 50);
      expect(find.text('No song matches.'), findsOneWidget);
    });

    appTest('counts the chosen and adds them in the order chosen', (h) async {
      await openAdd(h);
      await h.tester.tap(find.text('Opener'));
      await pumpFor(h, 50);
      await h.tester.tap(find.text('Encore'));
      await pumpFor(h, 50);
      expect(find.text('2 chosen'), findsOneWidget);
      expect(find.text('ADD 2 SONGS'), findsOneWidget);
      await h.tester.tap(find.text('Opener'));
      await pumpFor(h, 50);
      expect(find.text('1 chosen'), findsOneWidget);
      await h.tester.tap(find.text('Opener'));
      await pumpFor(h, 50);
      await h.tester.tap(find.text('ADD 2 SONGS'));
      await pumpFor(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('5 songs · 10 min'), findsOneWidget);
      await save(h);
      expect(lastSave(h).songs, [
        'First',
        'Second',
        'Third',
        'Encore',
        'Opener',
      ]);
    });

    appTest('can choose a song that is already in the playlist', (h) async {
      await openAdd(h);
      await h.tester.tap(find.text('First'));
      await pumpFor(h, 50);
      await h.tester.tap(find.text('ADD 1 SONG'));
      await pumpFor(h);
      await save(h);
      expect(lastSave(h).songs, ['First', 'Second', 'Third', 'First']);
    });

    appTest('Cancel adds nothing', (h) async {
      await openAdd(h);
      await h.tester.tap(find.text('First'));
      await pumpFor(h, 50);
      await h.tester.tap(find.text('CANCEL'));
      await pumpFor(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('3 songs · 6 min'), findsOneWidget);
    });
  });

  group('new and deleted playlists', () {
    Future<void> openPicker(AppHarness h) async {
      await h.openTab('Playlist');
      await h.tester.tap(find.text('change'));
      await pumpFor(h);
    }

    appTest('New playlist asks for a name, then opens an empty editor', (
      h,
    ) async {
      await openPicker(h);
      await h.tester.tap(find.text('+ NEW PLAYLIST'));
      await pumpFor(h);
      await h.tester.enterText(find.byType(TextField), '  Gig  ');
      await h.tester.tap(find.text('Create'));
      await pumpFor(h);
      expect(find.text('Editing'), findsOneWidget);
      expect(find.text('Gig'), findsOneWidget);
      expect(find.text('0 songs'), findsOneWidget);
      expect(find.text('No songs yet.'), findsOneWidget);
      expect(find.text('Choose a playlist'), findsNothing);
      await save(h);
      final sent = lastSave(h);
      expect(
        (sent.name, sent.create_5, sent.checkExpected),
        ('Gig', true, false),
      );
      expect(h.rig.playlists, contains('Gig'));
      expect(find.text('Editing'), findsNothing);
    });

    appTest('refuses a bad name with a plain message', (h) async {
      await openPicker(h);
      await h.tester.tap(find.text('+ NEW PLAYLIST'));
      await pumpFor(h);
      for (final bad in ['', 'a/b', 'all_songs']) {
        await h.tester.enterText(find.byType(TextField), bad);
        await h.tester.tap(find.text('Create'));
        await pumpFor(h, 50);
        expect(find.byType(AlertDialog), findsOneWidget, reason: bad);
        expect(find.textContaining(RegExp('name|/|own')), findsWidgets);
      }
      expect(find.text('Editing'), findsNothing);
    });

    appTest('a taken name is said so, and can be changed', (h) async {
      await openPicker(h);
      await h.tester.tap(find.text('+ NEW PLAYLIST'));
      await pumpFor(h);
      await h.tester.enterText(find.byType(TextField), 'Set');
      await h.tester.tap(find.text('Create'));
      await pumpFor(h);
      expect(
        find.text('A playlist with that name already exists.'),
        findsOneWidget,
      );
    });

    appTest('a name taken meanwhile is said so in the editor', (h) async {
      await openPicker(h);
      await h.tester.tap(find.text('+ NEW PLAYLIST'));
      await pumpFor(h);
      await h.tester.enterText(find.byType(TextField), 'Gig');
      await h.tester.tap(find.text('Create'));
      await pumpFor(h);
      h.rig.playlists['Gig'] = [RigSong('First')];
      await save(h);
      expect(
        find.text('A playlist with that name already exists.'),
        findsOneWidget,
      );
      await h.tester.tap(find.text('CHANGE NAME'));
      await pumpFor(h);
      await h.tester.enterText(find.byType(TextField), 'Gig two');
      await h.tester.tap(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.text('Rename'),
        ),
      );
      await pumpFor(h);
      expect(
        find.text('A playlist with that name already exists.'),
        findsNothing,
      );
      await save(h);
      expect(h.rig.playlists, contains('Gig two'));
    });

    appTest('holding a playlist asks, then deletes it', (h) async {
      await openPicker(h);
      await h.tester.longPress(find.text('Old'));
      await pumpFor(h);
      expect(find.text('Delete Old?'), findsOneWidget);
      await h.tester.tap(find.text('Cancel'));
      await pumpFor(h);
      expect(h.rig.playlists, contains('Old'));
      await h.tester.longPress(find.text('Old'));
      await pumpFor(h);
      await h.tester.tap(find.text('Delete'));
      await pumpFor(h);
      expect(h.rig.playlists, isNot(contains('Old')));
      expect(find.text('Old'), findsNothing);
    }, setup: (rig) => rig.playlists['Old'] = [RigSong('First')]);

    appTest('the playlist in use is refused here', (h) async {
      await openPicker(h);
      await h.tester.longPress(find.text('Set').last);
      await pumpFor(h);
      expect(find.text('Delete Set?'), findsNothing);
      expect(
        find.text('Switch to another playlist before deleting this one.'),
        findsOneWidget,
      );
      expect(calls(h, 'DeletePlaylist'), 0);
    });

    appTest('all_songs cannot be deleted', (h) async {
      await openPicker(h);
      await h.tester.longPress(find.text('all_songs'));
      await pumpFor(h);
      expect(find.textContaining('Delete all_songs'), findsNothing);
      expect(calls(h, 'DeletePlaylist'), 0);
    });

    appTest(
      'are not offered while locked',
      (h) async {
        await openPicker(h);
        expect(find.text('+ NEW PLAYLIST'), findsNothing);
        await h.tester.longPress(find.text('Old'));
        await pumpFor(h);
        expect(find.text('Delete Old?'), findsNothing);
      },
      setup: (rig) {
        rig.locked = true;
        rig.playlists['Old'] = [RigSong('First')];
      },
    );

    appTest('are not offered by an older player', (h) async {
      await openPicker(h);
      expect(find.text('Choose a playlist'), findsOneWidget);
      expect(find.text('+ NEW PLAYLIST'), findsNothing);
      expect(find.textContaining('Hold one to delete'), findsNothing);
    }, setup: (rig) => rig.oldPlayer = true);
  });
}
