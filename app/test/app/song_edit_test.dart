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
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/song_edit.dart';

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
        rig.songSettings['First'] = fullSongSettings();
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

  /// An edit of [name], read.
  SongEdit open(Harness h, [String name = 'First']) {
    final edit = h.session.editSong(name);
    edit.load();
    h.settle();
    expect(edit.song, isNotNull, reason: edit.loadFailure);
    return edit;
  }

  pp.UpdateSongSettingsRequest lastUpdate(Harness h) =>
      pp.UpdateSongSettingsRequest.fromBuffer(
        h.last('UpdateSongSettings').payload,
      );

  group('the library', () {
    scenario('is read whole, page after page', (h, s, _) {
      s.songLibrary.load();
      h.settle();
      final names = [for (final song in s.songLibrary.songs!) song.name];
      expect(names, [for (final song in h.rig.library) song.name]);
      expect(h.count('ListSongs'), greaterThan(1));
      final first = s.songLibrary.songs!.firstWhere((x) => x.name == 'First');
      expect(listingFlags(first), 'loops · samples');
      expect(s.missingFromPlayer, isEmpty);
    }, setup: (rig) => rig.songsPage = 2);

    scenario('of a player from before says so', (h, s, _) {
      s.songLibrary.load();
      h.settle();
      expect(s.songLibrary.tooOld, isTrue);
      expect(s.songLibrary.songs, isNull);
      expect(s.missingFromPlayer, contains('the song library'));
      // Forgetting the player forgets that too.
      s.forget();
      h.settle();
      expect(s.songLibrary.tooOld, isFalse);
    }, setup: (rig) => rig.songsTooOld = true);

    scenario('of a player without the lock is older still', (h, s, _) {
      s.songLibrary.load();
      h.settle();
      expect(s.songLibrary.tooOld, isTrue);
    }, setup: (rig) => rig.oldPlayer = true);

    test('is searched by name, any case', () {
      final songs = [
        pp.SongListing(name: 'Opener'),
        pp.SongListing(name: 'Encore'),
      ];
      expect(filterListings(songs, 'ENC').single.name, 'Encore');
      expect(filterListings(songs, '  '), songs);
      expect(
        listingFlags(pp.SongListing(name: 'x', failure: 'gone.wav')),
        'failed to load',
      );
      expect(
        listingFlags(
          pp.SongListing(name: 'x', hasLighting: true, hasMidi: true),
        ),
        'lights · MIDI',
      );
    });
  });

  group('folders', () {
    pp.SongListing song(String name, [String? dir]) {
      final listing = pp.SongListing(name: name);
      if (dir != null) {
        listing.baseDir = dir;
      }
      return listing;
    }

    test('are the directory less the song\'s own, as the web UI has it', () {
      expect(songFolder(song('a', 'Live/2026/Encore')), 'Live/2026');
      expect(songFolder(song('a', 'Encore')), '');
      expect(songFolder(song('a', '')), '');
      // A player that does not say: the top.
      expect(songFolder(song('a')), '');
    });

    test('group the top first, then by path ignoring case, nested apart', () {
      final groups = groupListings([
        song('A', 'Live/A'),
        song('B', 'B'),
        song('C', 'acoustic/C'),
        song('D', 'Live/2026/D'),
        song('E', 'Live/E'),
        song('F'),
      ]);
      expect(groups.map((g) => g.folder), [
        '',
        'acoustic',
        'Live',
        'Live/2026',
      ]);
      expect(groups.first.songs.map((s) => s.name), ['B', 'F']);
      expect(groups[2].songs.map((s) => s.name), ['A', 'E']);
    });

    test('are one group, the top, for a player that does not say them', () {
      final groups = groupListings([song('A'), song('B')]);
      expect(groups, hasLength(1));
      expect(groups.single.folder, '');
    });

    scenario('are closed and opened, and forgotten with the player', (h, s, _) {
      final library = s.songLibrary;
      expect(library.isCollapsed('Live'), isFalse);
      library.toggleFolder('Live');
      expect(library.isCollapsed('Live'), isTrue);
      library.toggleFolder('Live');
      expect(library.isCollapsed('Live'), isFalse);
      library.toggleFolder('Live');
      library.clear();
      expect(library.isCollapsed('Live'), isFalse);
    });
  });

  group('a song', () {
    scenario('reads as the player has it, with nothing to save', (h, s, _) {
      final edit = open(h);
      expect(edit.looping.crossfade, '2measures');
      expect(edit.tracks, hasLength(2));
      expect(edit.samples.single.name, 'kick');
      expect(edit.dirty, isFalse);
      expect(edit.canSave, isFalse);
      expect(edit.editable, isTrue);
      edit.dispose();
    });

    scenario('saves only the groups changed', (h, s, notices) {
      s.songLibrary.load();
      h.settle();
      final edit = open(h);
      edit.setLoopPlayback(false);
      expect(edit.dirty, isTrue);
      expect(edit.save(), completion(isTrue));
      h.settle();
      final sent = lastUpdate(h);
      expect(sent.expectedRevision, 's0');
      expect(sent.looping.loopPlayback, isFalse);
      // The crossfade, as written, goes with it.
      expect(sent.looping.crossfade, '2measures');
      expect(sent.hasTracks(), isFalse);
      expect(sent.hasSamples(), isFalse);
      expect(edit.dirty, isFalse);
      expect(edit.song!.revision, 's1');
      expect(notices.last, 'Saved First.');
      // The library's row says it no longer loops.
      final row = s.songLibrary.songs!.firstWhere((x) => x.name == 'First');
      expect(row.loops, isFalse);
      edit.dispose();
    });

    scenario('a renamed track is renamed among the cut tracks', (h, s, _) {
      final edit = open(h);
      edit.setTrack(0, pp.SongTrack(name: 'clicks', file: 'click.wav'));
      expect(edit.looping.cut, ['clicks']);
      edit.addTrack(pp.SongTrack(name: 'bass', file: 'bass.wav'));
      edit.removeTrack(0);
      expect(edit.looping.cut, isEmpty);
      expect(edit.trackNames, ['keys', 'bass']);
      edit.save();
      h.settle();
      final sent = lastUpdate(h);
      expect(sent.tracks.tracks.map((t) => t.name), ['keys', 'bass']);
      expect(sent.looping.cut, isEmpty);
      expect(h.rig.songSettings['First']!.tracks.tracks, hasLength(2));
      edit.dispose();
    });

    scenario('a sample keeps the name it was read under', (h, s, _) {
      final edit = open(h);
      final kick = edit.samples.single;
      edit.setSample(0, kick.deepCopy()..name = 'bass drum');
      edit.addSample(
        pp.SongSample(name: 'clap', originalName: 'stale', file: 'bass.wav'),
      );
      expect(edit.sampleNameTaken('clap'), isTrue);
      expect(edit.sampleNameTaken('clap', index: 1), isFalse);
      edit.save();
      h.settle();
      final sent = lastUpdate(h).samples.samples;
      expect(sent[0].name, 'bass drum');
      expect(sent[0].originalName, 'kick');
      // The velocity read goes back untouched (the player ignores it).
      expect(sent[0].velocity.layers, 2);
      expect(sent[1].originalName, isEmpty);
      edit.dispose();
    });

    scenario('changed elsewhere: save anyway keeps theirs', (h, s, _) {
      final edit = open(h);
      edit.setCrossfade('6s');
      h.rig.editSongElsewhere('First', (song) {
        song.tracks.tracks.add(pp.SongTrack(name: 'bass', file: 'bass.wav'));
      });
      edit.save();
      h.settle();
      expect(edit.problem, SongProblem.changed);
      expect(edit.canSave, isFalse);
      expect(h.rig.songUpdates, isEmpty);
      edit.saveAnyway();
      h.settle();
      expect(edit.problem, SongProblem.none);
      final sent = lastUpdate(h);
      expect(sent.expectedRevision, 's1');
      expect(sent.hasTracks(), isFalse);
      final now = h.rig.songSettings['First']!;
      expect(now.looping.crossfade, '6s');
      expect(now.tracks.tracks.map((t) => t.name), contains('bass'));
      edit.dispose();
    });

    scenario('changed elsewhere: discard reads it again', (h, s, _) {
      final edit = open(h);
      edit.setCrossfade('6s');
      h.rig.editSongElsewhere('First', (song) {
        song.looping.crossfade = '1measure';
      });
      edit.save();
      h.settle();
      expect(edit.problem, SongProblem.changed);
      edit.discardAndReload();
      h.settle();
      expect(edit.problem, SongProblem.none);
      expect(edit.looping.crossfade, '1measure');
      expect(edit.dirty, isFalse);
      edit.dispose();
    });

    scenario('a refusal is shown in the player\'s words until an edit', (
      h,
      s,
      _,
    ) {
      final edit = open(h);
      edit.setCrossfade('lots');
      edit.save();
      h.settle();
      expect(edit.problem, SongProblem.refused);
      expect(edit.refusal, contains('loop_crossfade'));
      expect(edit.dirty, isTrue);
      edit.setCrossfade('2s');
      expect(edit.problem, SongProblem.none);
      edit.dispose();
    });

    scenario('is read only while the player is locked', (h, s, _) {
      final edit = open(h);
      expect(edit.editable, isTrue);
      edit.setLoopPlayback(false);
      h.rig.locked = true;
      h.rig.publish();
      h.settle();
      expect(edit.editable, isFalse);
      expect(edit.problem, SongProblem.locked);
      expect(edit.canSave, isFalse);
      h.rig.locked = false;
      h.rig.publish();
      h.settle();
      expect(edit.problem, SongProblem.none);
      expect(edit.canSave, isTrue);
      edit.dispose();
    });

    scenario('a save the lock refuses says so', (h, s, _) {
      final edit = open(h);
      edit.setLoopPlayback(false);
      // Locked on the player before the phone heard of it.
      h.rig.locked = true;
      edit.save();
      h.settle();
      expect(edit.problem, SongProblem.locked);
      expect(h.rig.songUpdates, isEmpty);
      // The status says so, and then that it is unlocked again.
      h.rig.publish();
      h.settle();
      expect(edit.problem, SongProblem.locked);
      h.rig.locked = false;
      h.rig.publish();
      h.settle();
      expect(edit.problem, SongProblem.none);
      expect(edit.canSave, isTrue);
      edit.dispose();
    });

    scenario('playing, a save says when it takes effect', (h, s, notices) {
      final edit = open(h);
      edit.setLoopPlayback(false);
      edit.save();
      h.settle();
      expect(
        notices.last,
        'Saved First. It takes effect the next time it plays.',
      );
      edit.dispose();
    }, setup: (rig) => rig.playing = true);

    scenario(
      'one whose file does not read cannot be changed',
      (h, s, _) {
        final edit = open(h, 'Second');
        expect(edit.cannotEdit, contains('does not read'));
        expect(edit.editable, isFalse);
        edit.dispose();
      },
      setup: (rig) => rig.songSettings['Second'] = pp.GetSongResponse(
        name: 'Second',
        readOnly: "The song's file does not read, so it is not changed here",
      ),
    );
  });

  group('words', () {
    test('a sample in short, and its velocity', () {
      final sample = fullSongSettings().samples.samples.single;
      expect(
        sampleSummary(sample),
        'kick.wav · to channels 1, 2 · release: stop',
      );
      expect(velocitySummary(sample), 'Velocity: 2 layers, scaled by velocity');
      expect(velocitySummary(pp.SongSample(name: 'x')), isNull);
      expect(
        velocitySummary(
          pp.SongSample(
            velocity: pp.SampleVelocity(mode: 'ignore', defaultVelocity: 90),
          ),
        ),
        'Velocity: ignore, at 90',
      );
      expect(
        sampleSummary(
          pp.SongSample(outputTrack: 'drums', retrigger: 'polyphonic'),
        ),
        'no file · to drums · polyphonic',
      );
    });

    test('output channels as typed', () {
      expect(parseChannels('1, 2'), [1, 2]);
      expect(parseChannels(' 3 4 '), [3, 4]);
      expect(parseChannels(''), isEmpty);
      expect(parseChannels('0'), isNull);
      expect(parseChannels('1, two'), isNull);
    });

    test('what a song has besides', () {
      expect(overviewLines(fullSongSettings().overview), [
        'Tempo: 120 bpm, 4/4',
        '2 sections: verse, chorus',
        'Metronome',
        'Light show: show.light',
      ]);
      expect(
        trackSummary(pp.SongTrack(name: 'k', file: 'keys.wav', fileChannel: 2)),
        'keys.wav · channel 2',
      );
    });
  });
}
