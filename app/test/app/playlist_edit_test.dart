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
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/failure.dart';
import 'package:mtrack_remote/src/app/playlist_edit.dart';
import 'package:mtrack_remote/src/app/playlist_settings.dart';
import 'package:mtrack_remote/src/app/session.dart';

import '../support/fake_player.dart';
import '../support/rig.dart';
import '../support/session_harness.dart';

void main() {
  /// Runs [body] with a connected session over a rig set up by [setup].
  void scenario(
    String name,
    void Function(Harness h, Session s, List<String> notices) body, {
    void Function(Rig rig)? setup,
  }) {
    test(name, () {
      fakeAsync((async) {
        final rig = Rig();
        setup?.call(rig);
        rig.publish();
        final h = Harness(async, rig: rig);
        final notices = <String>[];
        h.session.notices.listen(notices.add);
        h.connect();
        body(h, h.session, notices);
        h.session.dispose();
        h.settle();
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  List<String> names(PlaylistEdit edit) => [for (final r in edit.rows) r.name];

  List<String> saved(Rig rig, String name) => [
    for (final s in rig.playlists[name]!) s.name,
  ];

  pp.SavePlaylistRequest lastSave(Harness h) =>
      pp.SavePlaylistRequest.fromBuffer(h.last('SavePlaylist').payload);

  /// Starts an edit of the active playlist.
  PlaylistEdit begin(Harness h) {
    expect(h.session.startEditing(), isNull);
    return h.session.editing!;
  }

  /// Loads the library into [edit], as the song picker does.
  void loadLibrary(Harness h, PlaylistEdit edit) {
    edit.loadLibrary();
    h.settle();
  }

  group('the lock', () {
    scenario('follows the status', (h, s, _) {
      expect(s.locked, isFalse);
      expect(s.lockAvailable, isTrue);
      h.rig.locked = true;
      h.rig.publish();
      h.settle();
      expect(s.locked, isTrue);
    });

    scenario('a player without one has none to show', (h, s, _) {
      expect(s.locked, isNull);
      expect(s.lockAvailable, isFalse);
      expect(s.canEditPlaylist, isFalse);
    }, setup: (rig) => rig.oldPlayer = true);

    scenario('locks and unlocks, and shows the answer at once', (h, s, _) {
      expect(h.run(() => s.setLock(true)), isNull);
      expect(h.rig.locked, isTrue);
      expect(
        pp.SetLockRequest.fromBuffer(h.last('SetLock').payload).locked,
        isTrue,
      );
      expect(s.locked, isTrue);
      // Allowed while locked: it is how a phone gets out of it.
      expect(h.run(() => s.setLock(false)), isNull);
      expect(h.rig.locked, isFalse);
      expect(s.locked, isFalse);
    });

    scenario('is left as it is: no relock by itself', (h, s, _) {
      h.run(() => s.setLock(false));
      h.elapse(const Duration(hours: 2));
      expect(s.locked, isFalse);
      expect(h.count('SetLock'), 1);
    }, setup: (rig) => rig.locked = true);

    scenario('says so when an old player refuses it', (h, s, notices) {
      expect(h.run(() => s.setLock(true)), isNotNull);
      expect(h.rig.locked, isFalse);
      expect(notices, hasLength(1));
    }, setup: (rig) => rig.oldPlayer = true);
  });

  group('who may edit', () {
    scenario('an unlocked player may', (h, s, _) {
      expect(s.canEditPlaylist, isTrue);
      expect(s.startEditing(), isNull);
      expect(s.editing, isNotNull);
      expect(names(s.editing!), ['First', 'Second', 'Third']);
    });

    scenario('a locked player may not', (h, s, notices) {
      expect(s.canEditPlaylist, isFalse);
      expect(s.startEditing(), isNotNull);
      expect(s.editing, isNull);
      expect(notices, hasLength(1));
    }, setup: (rig) => rig.locked = true);

    scenario('all_songs is never editable', (h, s, _) {
      expect(h.run(() => s.switchPlaylist('all_songs')), isNull);
      expect(s.status?.playlistName, 'all_songs');
      expect(s.canEditPlaylist, isFalse);
      expect(s.startEditing(), isNotNull);
    });

    scenario('unlocking offers it', (h, s, _) {
      expect(s.canEditPlaylist, isFalse);
      h.run(() => s.setLock(false));
      expect(s.canEditPlaylist, isTrue);
    }, setup: (rig) => rig.locked = true);
  });

  group('changing the list', () {
    scenario('nothing reaches the player until Save', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(0);
      edit.move(0, 1);
      expect(h.count('SavePlaylist'), 0);
      expect(saved(h.rig, 'Set'), ['First', 'Second', 'Third']);
    });

    scenario('reorder, remove and add are what is saved', (h, s, _) {
      final edit = begin(h);
      loadLibrary(h, edit);
      edit.removeAt(1); // Second
      expect(names(edit), ['First', 'Third']);
      edit.move(0, 1); // First after Third
      expect(names(edit), ['Third', 'First']);
      edit.addSongs(['Opener', 'Encore']);
      expect(names(edit), ['Third', 'First', 'Opener', 'Encore']);
      expect(edit.total, const Duration(seconds: 200 + 100 + 75 + 150));

      expect(h.run(() async => await edit.save() ? null : 'failed'), isNull);
      final sent = lastSave(h);
      expect(sent.name, 'Set');
      expect(sent.songs, ['Third', 'First', 'Opener', 'Encore']);
      expect(sent.checkExpected, isTrue);
      expect(sent.create_5, isFalse);
      expect(saved(h.rig, 'Set'), ['Third', 'First', 'Opener', 'Encore']);
    });

    scenario('expected is the list as loaded, not as edited', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(0);
      edit.removeAt(0);
      h.run(() async => await edit.save() ? null : 'failed');
      expect(lastSave(h).expected, ['First', 'Second', 'Third']);
      expect(lastSave(h).songs, ['Third']);
    });

    scenario('a song may be in the list twice, and a row goes alone', (
      h,
      s,
      _,
    ) {
      final edit = begin(h);
      loadLibrary(h, edit);
      edit.addSongs(['First', 'First']);
      expect(names(edit), ['First', 'Second', 'Third', 'First', 'First']);
      expect(edit.contains('First'), isTrue);
      edit.removeAt(3);
      expect(names(edit), ['First', 'Second', 'Third', 'First']);
      expect(edit.rows[0].id, isNot(edit.rows[3].id));
      h.run(() async => await edit.save() ? null : 'failed');
      expect(saved(h.rig, 'Set'), ['First', 'Second', 'Third', 'First']);
    });

    scenario('moves by a drop index, as the list reports it', (h, s, _) {
      final edit = begin(h);
      edit.move(0, 2);
      expect(names(edit), ['Second', 'Third', 'First']);
      edit.move(2, 0);
      expect(names(edit), ['First', 'Second', 'Third']);
      edit.move(1, 1);
      edit.move(5, 0);
      edit.removeAt(9);
      expect(names(edit), ['First', 'Second', 'Third']);
    });

    scenario('is dirty only while it differs from what was loaded', (h, s, _) {
      final edit = begin(h);
      expect(edit.dirty, isFalse);
      expect(edit.canSave, isFalse);
      expect(edit.hasChanges, isFalse);
      edit.move(0, 1);
      expect(edit.dirty, isTrue);
      expect(edit.canSave, isTrue);
      edit.move(1, 0);
      expect(edit.dirty, isFalse);
      expect(edit.canSave, isFalse);
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(h.count('SavePlaylist'), 0);
    });

    scenario('a removed row can be put back by adding the song', (h, s, _) {
      final edit = begin(h);
      loadLibrary(h, edit);
      edit.removeAt(2);
      edit.addSongs(['Third']);
      expect(edit.dirty, isFalse);
    });

    scenario('marks the song the player is on', (h, s, _) {
      final edit = begin(h);
      expect(s.position, 0);
      expect(edit.rows[0].originalIndex, 0);
      expect(edit.rows[2].originalIndex, 2);
      edit.move(0, 2);
      // The row that was first is still the one marked.
      expect(edit.rows.last.originalIndex, s.position);
    });

    scenario('the saved list replaces the Playlist tab at once', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(2);
      h.run(() async => await edit.save() ? null : 'failed');
      expect(s.editing, isNull);
      expect([for (final e in s.entries) e.songName], ['First', 'Second']);
    });

    scenario('an edit can be left without saving', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(0);
      s.endEditing();
      h.settle();
      expect(s.editing, isNull);
      expect(h.count('SavePlaylist'), 0);
      expect(saved(h.rig, 'Set'), ['First', 'Second', 'Third']);
    });

    scenario('another playlist can be edited after one is left', (h, s, _) {
      s.startEditing();
      s.endEditing();
      expect(s.startEditing(), isNull);
      expect(s.editing, isNotNull);
    });
  });

  group('saving', () {
    scenario('is busy while the call is out, and cannot be sent twice', (
      h,
      s,
      _,
    ) {
      final edit = begin(h);
      edit.removeAt(0);
      final answer = h.player.handlers['SavePlaylist']!;
      h.player.handlers['SavePlaylist'] = (r) => FakeReply.ok(
        pp.GetPlaylistResponse.fromBuffer(answer(r).payload),
        delay: const Duration(seconds: 1),
      );
      edit.save();
      h.settle();
      expect(edit.saving, isTrue);
      expect(edit.canSave, isFalse);
      edit.save();
      h.settle();
      expect(h.count('SavePlaylist'), 1);
      h.elapse(const Duration(seconds: 1));
      expect(edit.saving, isFalse);
      expect(s.editing, isNull);
    });
  });

  group('when the playlist changed on the player', () {
    void conflict(Harness h, PlaylistEdit edit) {
      h.rig.editElsewhere('Set', ['Third', 'First']);
      edit.removeAt(0);
      expect(
        h.run(() async => await edit.save() ? null : 'refused'),
        'refused',
      );
    }

    scenario('keeps the edits and says what happened', (h, s, notices) {
      final edit = begin(h);
      conflict(h, edit);
      expect(edit.problem, EditProblem.changed);
      expect(s.editing, same(edit));
      expect(names(edit), ['Second', 'Third']);
      expect(edit.canSave, isFalse);
      expect(notices, isEmpty);
      expect(
        playlistChangedMessage,
        'This playlist was changed on the player while you were editing.',
      );
    });

    scenario('Save anyway sends the edits over what the player has', (h, s, _) {
      final edit = begin(h);
      conflict(h, edit);
      final before = h.count('GetPlaylist');
      expect(h.run(() async => await edit.saveAnyway() ? null : 'no'), isNull);
      expect(h.count('GetPlaylist'), before + 1);
      expect(lastSave(h).expected, ['Third', 'First']);
      expect(lastSave(h).songs, ['Second', 'Third']);
      expect(saved(h.rig, 'Set'), ['Second', 'Third']);
      expect(s.editing, isNull);
    });

    scenario('Discard my changes loads what the player has', (h, s, _) {
      final edit = begin(h);
      conflict(h, edit);
      expect(
        h.run(() async => await edit.discardAndReload() ? null : 'no'),
        isNull,
      );
      expect(edit.problem, EditProblem.none);
      expect(names(edit), ['Third', 'First']);
      expect(edit.dirty, isFalse);
      expect(s.editing, same(edit));
      // A later edit is checked against what was just loaded.
      edit.removeAt(0);
      h.run(() async => await edit.save() ? null : 'no');
      expect(lastSave(h).expected, ['Third', 'First']);
      expect(saved(h.rig, 'Set'), ['First']);
    });
  });

  group('when the player is locked', () {
    scenario('a save the player refuses keeps the edit', (h, s, notices) {
      final edit = begin(h);
      edit.removeAt(0);
      // Locked after the last status: the player says so on the save.
      h.rig.locked = true;
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(edit.problem, EditProblem.locked);
      expect(lockedToSaveMessage, 'The player is locked. Unlock it to save.');
      expect(s.editing, same(edit));
      expect(names(edit), ['Second', 'Third']);
      expect(notices, isEmpty);
      // Unlocked again: the edit is still there to save.
      h.run(() => s.setLock(false));
      expect(edit.problem, EditProblem.none);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      expect(saved(h.rig, 'Set'), ['Second', 'Third']);
    });

    scenario('a status that says so stops Save before it is sent', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(0);
      h.rig.locked = true;
      h.rig.publish();
      h.settle();
      expect(edit.problem, EditProblem.locked);
      expect(edit.canSave, isFalse);
      expect(h.count('SavePlaylist'), 0);
      h.rig.locked = false;
      h.rig.publish();
      h.settle();
      expect(edit.problem, EditProblem.none);
      expect(edit.canSave, isTrue);
    });
  });

  group('a new playlist', () {
    scenario('starts empty and is made on Save', (h, s, _) {
      expect(s.startNewPlaylist('  Gig  '), isNull);
      final edit = s.editing!;
      expect(edit.isNew, isTrue);
      expect(edit.name, 'Gig');
      expect(edit.rows, isEmpty);
      expect(edit.hasChanges, isFalse);
      loadLibrary(h, edit);
      expect(edit.library!.map((e) => e.songName), [
        'Encore',
        'First',
        'Opener',
        'Second',
        'Third',
      ]);
      edit.addSongs(['Opener', 'First']);
      expect(edit.hasChanges, isTrue);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      final sent = lastSave(h);
      expect(
        (sent.name, sent.create_5, sent.checkExpected),
        ('Gig', true, false),
      );
      expect(saved(h.rig, 'Gig'), ['Opener', 'First']);
      expect(s.editing, isNull);
      // The list in use is not changed.
      expect(s.status?.playlistName, 'Set');
    });

    scenario('may be saved empty', (h, s, _) {
      s.startNewPlaylist('Blank');
      expect(s.editing!.canSave, isTrue);
      expect(h.run(() async => await s.editing!.save() ? null : 'no'), isNull);
      expect(saved(h.rig, 'Blank'), isEmpty);
    });

    test('names are checked here before the player is asked', () {
      expect(playlistNameProblem(''), isNotNull);
      expect(playlistNameProblem('a/b'), isNotNull);
      expect(playlistNameProblem(r'a\b'), isNotNull);
      expect(playlistNameProblem('all_songs'), isNotNull);
      expect(playlistNameProblem('ALL_SONGS'), isNotNull);
      expect(playlistNameProblem('Brimming Horn - 11-2026'), isNull);
    });

    scenario('refuses a bad name without starting anything', (h, s, _) {
      for (final bad in ['', '   ', 'a/b', r'a\b', 'all_songs']) {
        expect(s.startNewPlaylist(bad), isNotNull, reason: bad);
      }
      expect(s.editing, isNull);
    });

    scenario('refuses a name it knows is taken', (h, s, _) {
      h.run(s.loadPlaylists);
      expect(s.startNewPlaylist('Set'), nameTakenMessage);
      expect(s.editing, isNull);
    });

    scenario('a name taken in the meantime can be changed', (h, s, notices) {
      s.startNewPlaylist('Gig');
      final edit = s.editing!;
      h.rig.playlists['Gig'] = [RigSong('First')];
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(edit.problem, EditProblem.exists);
      expect(nameTakenMessage, 'A playlist with that name already exists.');
      expect(notices, isEmpty);
      expect(edit.rename('all_songs'), isNotNull);
      expect(edit.rename('Gig 2'), isNull);
      expect(edit.problem, EditProblem.none);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      expect(saved(h.rig, 'Gig 2'), isEmpty);
      expect(saved(h.rig, 'Gig'), ['First']);
    });

    scenario('another failure is the player\'s own words', (h, s, notices) {
      s.startNewPlaylist('Gig');
      final edit = s.editing!;
      edit.addSongs(['Not in the library']);
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      // Said in the editor, where it stays until something is changed.
      expect(notices, isEmpty);
      expect(edit.problem, EditProblem.refused);
      expect(edit.refusal, "Song 'Not in the library' not found");
      expect(s.editing, same(edit));
      expect(names(edit), ['Not in the library']);
    });
  });

  group('deleting', () {
    scenario('asks first, and does nothing if told no', (h, s, _) {
      h.rig.playlists['Old'] = [RigSong('First')];
      var asked = 0;
      expect(
        h.run(
          () => s.deletePlaylist(
            'Old',
            confirm: () async {
              asked++;
              return false;
            },
          ),
        ),
        isNull,
      );
      expect(asked, 1);
      expect(h.count('DeletePlaylist'), 0);
      expect(h.rig.playlists, contains('Old'));
    });

    scenario('deletes when told yes, and refreshes the list', (h, s, _) {
      h.rig.playlists['Old'] = [RigSong('First')];
      h.run(s.loadPlaylists);
      expect(
        h.run(() => s.deletePlaylist('Old', confirm: () async => true)),
        isNull,
      );
      expect(
        pp.DeletePlaylistRequest.fromBuffer(h.last('DeletePlaylist').payload)
            .name,
        'Old',
      );
      expect(h.rig.playlists, isNot(contains('Old')));
      expect(s.choices?.names, ['Set', 'all_songs']);
    });

    scenario('is refused here for the playlist in use', (h, s, notices) {
      var asked = false;
      final said = h.run(
        () => s.deletePlaylist(
          'Set',
          confirm: () async {
            asked = true;
            return true;
          },
        ),
      );
      expect(said, 'Switch to another playlist before deleting this one.');
      expect(notices, [said]);
      expect(asked, isFalse);
      expect(h.count('DeletePlaylist'), 0);
    });

    scenario('is refused here for all_songs', (h, s, notices) {
      final said = h.run(
        () => s.deletePlaylist('all_songs', confirm: () async => true),
      );
      expect(said, isNotNull);
      expect(h.count('DeletePlaylist'), 0);
    });

    scenario('a locked player\'s refusal is passed on', (h, s, notices) {
      h.rig.playlists['Old'] = [RigSong('First')];
      h.rig.locked = true;
      final said = h.run(
        () => s.deletePlaylist('Old', confirm: () async => true),
      );
      expect(said, 'Player is locked. Unlock to make changes.');
      expect(h.rig.playlists, contains('Old'));
    });
  });

  group('when the link drops', () {
    scenario('keeps the edits and saves once it is back', (h, s, _) {
      final edit = begin(h);
      edit.removeAt(0);
      h.link.connectFailures = 1;
      h.player.disconnect();
      h.settle();
      expect(s.phase, ConnectionPhase.lost);
      expect(s.editing, same(edit));
      expect(names(edit), ['Second', 'Third']);
      expect(edit.canSave, isFalse);
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(h.count('SavePlaylist'), 0);

      h.elapse(const Duration(seconds: 10));
      expect(s.phase, ConnectionPhase.ready);
      expect(s.editing, same(edit));
      expect(edit.canSave, isTrue);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      expect(saved(h.rig, 'Set'), ['Second', 'Third']);
    });

    scenario('says plainly why Save does nothing', (h, s, notices) {
      final edit = begin(h);
      edit.removeAt(0);
      h.link.connectFailures = 100;
      h.player.disconnect();
      h.settle();
      edit.save();
      h.settle();
      expect(edit.canSave, isFalse);
      expect(s.controlsDisabled, isTrue);
      expect(notConnectedMessage, 'Not connected to a player.');
    });

    scenario('forgetting the player drops the edit', (h, s, _) {
      begin(h);
      h.run(() async {
        await s.forget();
        return null;
      });
      expect(s.editing, isNull);
    });
  });

  group('settings', () {
    /// The fade-out every test starts Second with: more than the app edits.
    pp.Fade fullFade() => pp.Fade(
      length: '2beats',
      curve: 'linear',
      exclude: pp.TrackNames(names: ['Click']),
      lights: '0s',
    );

    void withSettings(Rig rig) {
      rig.entrySettings['Set'] = [
        pp.EntrySettings(),
        pp.EntrySettings(
          fadeOut: fullFade(),
          autoPlay: pp.AutoPlay(gap: '2s'),
        ),
        pp.EntrySettings(ambient: pp.AmbientSetting(on: false)),
      ];
      rig.playlistAmbient['Set'] = pp.AmbientSetting(
        on: true,
        show: 'lighting/ambient.light',
      );
      rig.revisions['Set'] = 7;
    }

    scenario('are read with the playlist', (h, s, _) {
      final edit = begin(h);
      expect(edit.supportsSettings, isTrue);
      expect(s.playlistSettingsAvailable, isTrue);
      expect(s.missingFromPlayer, isEmpty);
      expect(edit.revision, 'r7');
      expect(edit.rows[1].settings.fadeOut, fullFade());
      expect(edit.rows[1].settings.autoPlay.gap, '2s');
      expect(edit.rows[2].settings.ambient.on, isFalse);
      expect(edit.ambient!.show, 'lighting/ambient.light');
      expect(edit.dirty, isFalse);
    }, setup: withSettings);

    scenario('a changed length keeps the fade details the app does not edit', (
      h,
      s,
      _,
    ) {
      final edit = begin(h);
      final row = edit.rows[1];
      edit.setSettings(
        row.id,
        row.settings.deepCopy()
          ..fadeOut = withFadeLength(row.settings.fadeOut, '4beats')!,
      );
      expect(edit.dirty, isTrue);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      final sent = lastSave(h);
      expect(sent.setSettings, isTrue);
      expect(sent.expectedRevision, 'r7');
      expect(sent.setAmbient, isFalse);
      expect(sent.settings, hasLength(3));
      expect(sent.settings[1].fadeOut, fullFade()..length = '4beats');
      expect(sent.settings[1].autoPlay.gap, '2s');
      expect(sent.settings[2].ambient.on, isFalse);
      expect(h.rig.settingsAt('Set', 1).fadeOut.curve, 'linear');
      expect(h.rig.settingsAt('Set', 1).fadeOut.lights, '0s');
      expect(s.editing, isNull);
    }, setup: withSettings);

    scenario('songs and order alone leave the settings to the player', (
      h,
      s,
      _,
    ) {
      final edit = begin(h);
      edit.move(1, 0);
      h.run(() async => await edit.save() ? null : 'no');
      final sent = lastSave(h);
      expect(sent.setSettings, isFalse);
      expect(sent.settings, isEmpty);
      expect(sent.setAmbient, isFalse);
      // The player carried Second's settings with it.
      expect(h.rig.settingsAt('Set', 0).fadeOut, fullFade());
    }, setup: withSettings);

    scenario('a setting put back as it was is no change', (h, s, _) {
      final edit = begin(h);
      final row = edit.rows[0];
      edit.setSettings(row.id, pp.EntrySettings(fadeIn: pp.Fade(length: '1s')));
      expect(edit.dirty, isTrue);
      edit.setSettings(row.id, pp.EntrySettings());
      expect(edit.dirty, isFalse);
      edit.setAmbient(null);
      expect(edit.dirty, isTrue);
      edit.setAmbient(
        pp.AmbientSetting(on: true, show: 'lighting/ambient.light'),
      );
      expect(edit.dirty, isFalse);
    }, setup: withSettings);

    scenario('the playlist ambient is sent when changed', (h, s, _) {
      final edit = begin(h);
      edit.setAmbient(pp.AmbientSetting(on: false));
      h.run(() async => await edit.save() ? null : 'no');
      final sent = lastSave(h);
      expect(sent.setAmbient, isTrue);
      expect(sent.ambient, pp.AmbientSetting(on: false));
      expect(sent.setSettings, isFalse);
      expect(h.rig.playlistAmbient['Set'], pp.AmbientSetting(on: false));
    }, setup: withSettings);

    scenario('a setting changed on the player is a conflict', (h, s, _) {
      final edit = begin(h);
      // The web UI changes a setting, not a song.
      h.rig.editSettingsElsewhere(
        'Set',
        0,
        pp.EntrySettings(fadeIn: pp.Fade(length: '1s')),
      );
      edit.removeAt(2);
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(edit.problem, EditProblem.changed);
      // Save anyway checks against what is there now, and keeps the
      // setting this edit did not touch.
      expect(h.run(() async => await edit.saveAnyway() ? null : 'no'), isNull);
      expect(lastSave(h).expectedRevision, 'r8');
      expect(h.rig.settingsAt('Set', 0).fadeIn.length, '1s');
    }, setup: withSettings);

    scenario('Discard my changes loads the settings as they are now', (
      h,
      s,
      _,
    ) {
      final edit = begin(h);
      h.rig.editSettingsElsewhere(
        'Set',
        0,
        pp.EntrySettings(autoPlay: pp.AutoPlay()),
      );
      edit.removeAt(2);
      h.run(() async => await edit.save() ? null : 'no');
      h.run(() async => await edit.discardAndReload() ? null : 'no');
      expect(edit.rows[0].settings.hasAutoPlay(), isTrue);
      expect(edit.revision, 'r8');
      expect(edit.dirty, isFalse);
    }, setup: withSettings);

    scenario('what the player refuses is said in its words, until changed', (
      h,
      s,
      notices,
    ) {
      final edit = begin(h);
      final row = edit.rows[0];
      edit.setSettings(
        row.id,
        pp.EntrySettings(fadeOut: pp.Fade(length: 'soon')),
      );
      expect(h.run(() async => await edit.save() ? null : 'no'), 'no');
      expect(edit.problem, EditProblem.refused);
      expect(edit.refusal, contains("fade_out: 'soon' is not a length"));
      expect(notices, isEmpty);
      expect(s.editing, same(edit));
      expect(edit.canSave, isTrue);
      edit.setSettings(
        row.id,
        pp.EntrySettings(fadeOut: pp.Fade(length: '2s')),
      );
      expect(edit.problem, EditProblem.none);
      expect(edit.refusal, isNull);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
    }, setup: withSettings);

    scenario('the light shows are fetched once', (h, s, _) {
      final edit = begin(h);
      expect(edit.lightShows, isNull);
      edit.loadLightShows();
      h.settle();
      edit.loadLightShows();
      h.settle();
      expect(edit.lightShows, ['lighting/ambient.light', 'shows/encore.light']);
      expect(h.count('ListLightShows'), 1);
    });

    scenario('an older player edits songs and order only', (h, s, _) {
      final edit = begin(h);
      expect(edit.supportsSettings, isFalse);
      expect(s.missingFromPlayer, contains('playlist settings'));
      // Nothing to set, and nothing sent but names.
      edit.setSettings(
        edit.rows[0].id,
        pp.EntrySettings(autoPlay: pp.AutoPlay()),
      );
      edit.setAmbient(pp.AmbientSetting(on: false));
      expect(edit.dirty, isFalse);
      edit.move(2, 0);
      expect(h.run(() async => await edit.save() ? null : 'no'), isNull);
      final sent = lastSave(h);
      expect(sent.setSettings, isFalse);
      expect(sent.setAmbient, isFalse);
      expect(sent.expectedRevision, isEmpty);
      expect(saved(h.rig, 'Set'), ['Third', 'First', 'Second']);
    }, setup: (rig) => rig.settingsTooOld = true);
  });

  group('an older player', () {
    scenario('offers nothing to edit', (h, s, notices) {
      expect(s.lockAvailable, isFalse);
      expect(s.canEditPlaylist, isFalse);
      expect(s.startEditing(), isNotNull);
      expect(s.editing, isNull);
      expect(h.count('SavePlaylist'), 0);
    }, setup: (rig) => rig.oldPlayer = true);
  });

  test('songs are found by any part of the name, in any case', () {
    final library = [
      for (final n in ['Afar', 'Battle Scarred', 'Beornulf', 'Jotunheim'])
        pp.PlaylistEntry(songName: n),
    ];
    List<String> found(String q) => [
      for (final e in filterSongs(library, q)) e.songName,
    ];
    expect(found(''), hasLength(4));
    expect(found('  '), hasLength(4));
    expect(found('AR'), ['Afar', 'Battle Scarred']);
    expect(found('scar'), ['Battle Scarred']);
    expect(found('nothing'), isEmpty);
  });
}
