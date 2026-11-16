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
import 'package:mtrack_remote/src/demo/demo_content.dart';
import 'package:mtrack_remote/src/demo/demo_player.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';

/// A refusal with [code], whose message has [part] in it.
Matcher refused(int code, [String part = '']) => throwsA(
  isA<PlayerException>()
      .having((e) => e.code, 'code', code)
      .having((e) => e.message, 'message', contains(part)),
);

Matcher get locked => refused(GrpcCode.failedPrecondition, 'locked');

Duration elapsedOf(DemoPlayer p) => p.status().elapsed.toDart();

void main() {
  /// Runs [body] on a demo player on fake time, and checks that the player
  /// leaves no timer behind when it is disposed of.
  void scenario(
    String name,
    void Function(DemoPlayer p, FakeAsync async) body,
  ) {
    test(name, () {
      fakeAsync((async) {
        final p = DemoPlayer(clock: () => async.elapsed);
        body(p, async);
        p.dispose();
        async.flushMicrotasks();
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  group('the library', () {
    scenario('has eight invented songs of two to five minutes', (p, async) {
      final songs = p.listSongs(0, false);
      expect(songs.songs, hasLength(8));
      expect(songs.total, 8);
      expect(songs.hasNext(), isFalse);
      for (final song in songs.songs) {
        final seconds = song.duration.toDart().inSeconds;
        expect(seconds, inInclusiveRange(120, 300), reason: song.name);
        expect(song.baseDir, endsWith(song.name));
      }
      final names = [for (final s in songs.songs) s.name];
      expect(names, [...names]..sort());
    });

    scenario('gives every song sections that sit on its beat grid', (p, async) {
      for (final listing in p.listSongs(0, true).songs) {
        final song = p.getSong(listing.name);
        expect(song.overview.sections, isNotEmpty);
        expect(listing.sections.map((s) => s.name), contains('Intro'));
        expect(listing.sections.map((s) => s.name), contains('Chorus'));
        expect(listing.beatGrid.beats, isNotEmpty);
        // The last section ends where the song does.
        final last = listing.sections.last;
        expect(last.endMeasure - 1, listing.beatGrid.measureStarts.length);
      }
    });

    scenario('has two or three songs with samples and one that loops', (
      p,
      async,
    ) {
      final withSamples = [
        for (final s in p.listSongs(0, false).songs)
          if (s.hasSamples) s.name,
      ];
      expect(withSamples.length, inInclusiveRange(2, 3));
      final loops = [
        for (final s in p.listSongs(0, false).songs)
          if (s.loops) s.name,
      ];
      expect(loops, ['Second Wind']);
      final song = p.getSong(withSamples.first);
      expect(song.samples.samples, isNotEmpty);
      expect(song.triggers, isNotEmpty);
    });

    scenario('lists the playlists as the real player does', (p, async) {
      final lists = p.listPlaylists(true);
      expect(lists.names, ['Rehearsal list', 'Short set', 'all_songs']);
      expect(lists.active, 'Short set');
      expect(lists.playlists.last.reserved, isTrue);
      expect(lists.playlists.last.songCount, 8);
      expect(p.listPlaylists(false).playlists, isEmpty);
      final all = p.getPlaylist('all_songs');
      expect(all.entries, hasLength(8));
      expect(all.revision, isNotEmpty);
      expect(() => p.getPlaylist('nope'), refused(GrpcCode.notFound));
    });

    scenario('has tracks with gains for a believable rig', (p, async) {
      final gains = p.getTrackGains().gains;
      expect(gains.map((g) => g.track), containsAll(['click', 'cues']));
      expect(gains.length, greaterThanOrEqualTo(6));
      expect(gains.any((g) => g.gainDb != 0), isTrue);
    });
  });

  group('playback', () {
    scenario('starts stopped, on the short set, unlocked', (p, async) {
      final status = p.status();
      expect(status.playing, isFalse);
      expect(status.playlistName, 'Short set');
      expect(status.songName, 'Harbor Lights');
      expect(status.position, 0);
      expect(status.locked, isFalse);
      expect(status.elapsed.toDart(), Duration.zero);
      expect(status.songDuration.toDart(), const Duration(seconds: 240));
    });

    scenario('runs the clock while a song plays', (p, async) {
      expect(p.play().song.name, 'Harbor Lights');
      async.elapse(const Duration(seconds: 12));
      final status = p.status();
      expect(status.playing, isTrue);
      expect(status.elapsed.toDart(), const Duration(seconds: 12));
    });

    scenario('pushes a status on change and every second while playing', (
      p,
      async,
    ) {
      final heard = <pp.RemoteStatus>[];
      p.statuses.listen(heard.add);
      p.play();
      async.elapse(Duration.zero);
      expect(heard, hasLength(1), reason: 'the play');
      async.elapse(const Duration(milliseconds: 5500));
      // Five more, a second apart.
      expect(heard, hasLength(6));
      expect(heard.last.elapsed.toDart(), const Duration(seconds: 5));
      p.stop();
      async.elapse(const Duration(seconds: 10));
      expect(heard, hasLength(7), reason: 'nothing while stopped');
    });

    scenario('refuses a play while a song plays, and a stop when none does', (
      p,
      async,
    ) {
      expect(
        () => p.stop(),
        refused(GrpcCode.failedPrecondition, 'not playing'),
      );
      p.play();
      expect(
        () => p.play(),
        refused(GrpcCode.failedPrecondition, 'already playing'),
      );
      expect(
        () => p.playFrom(const Duration(seconds: 3)),
        refused(GrpcCode.failedPrecondition, 'already playing'),
      );
      expect(p.stop().song.name, 'Harbor Lights');
      final status = p.status();
      expect(status.playing, isFalse);
      expect(status.elapsed.toDart(), Duration.zero);
    });

    scenario('plays a song from a position, and refuses one past its end', (
      p,
      async,
    ) {
      p.playFrom(const Duration(seconds: 100));
      async.elapse(const Duration(seconds: 5));
      expect(elapsedOf(p), const Duration(seconds: 105));
      p.stop();
      expect(
        () => p.playFrom(const Duration(seconds: 240)),
        refused(GrpcCode.failedPrecondition, 'at or beyond song length 240s'),
      );
    });

    scenario('moves on when a song ends, and starts an entry that is to', (
      p,
      async,
    ) {
      p.playFrom(const Duration(seconds: 238));
      async.elapse(const Duration(seconds: 1));
      expect(p.status().songName, 'Harbor Lights');
      async.elapse(const Duration(seconds: 1));
      var status = p.status();
      expect(status.playing, isFalse);
      expect(status.songName, 'Copper Wire');
      expect(status.position, 1);
      expect(status.elapsed.toDart(), Duration.zero);
      // Copper Wire is set to start by itself, five seconds on.
      expect(status.autoPlay.song, 'Copper Wire');
      expect(status.autoPlay.startsIn.toDart(), const Duration(seconds: 5));
      async.elapse(const Duration(seconds: 2));
      expect(p.status().autoPlay.startsIn.toDart(), const Duration(seconds: 3));
      async.elapse(const Duration(seconds: 3));
      status = p.status();
      expect(status.playing, isTrue);
      expect(status.songName, 'Copper Wire');
      expect(status.hasAutoPlay(), isFalse);
    });

    scenario('calls off a song waiting to start by itself', (p, async) {
      p.playFrom(const Duration(seconds: 239));
      async.elapse(const Duration(seconds: 1));
      expect(p.status().hasAutoPlay(), isTrue);
      expect(p.cancelAutoPlay().cancelled, isTrue);
      expect(p.cancelAutoPlay().cancelled, isFalse);
      async.elapse(const Duration(seconds: 30));
      expect(p.status().playing, isFalse);
    });

    scenario('stays on the last song of the list when it ends', (p, async) {
      p.switchToPlaylist('Rehearsal list');
      p.navigateToSong(4, 'Harbor Lights');
      p.playFrom(const Duration(seconds: 239));
      async.elapse(const Duration(seconds: 1));
      final status = p.status();
      expect(status.playing, isFalse);
      expect(status.songName, 'Harbor Lights');
      expect(status.position, 4);
      expect(status.hasAutoPlay(), isFalse);
      expect(status.elapsed.toDart(), Duration.zero);
    });

    scenario('goes next and previous while stopped, and stays at the ends', (
      p,
      async,
    ) {
      expect(p.previous().song.name, 'Harbor Lights');
      expect(p.next().song.name, 'Copper Wire');
      expect(p.next().song.name, 'Sundial');
      expect(p.next().song.name, 'Long Way Down');
      expect(p.next().song.name, 'Long Way Down');
      expect(p.status().position, 3);
      expect(p.previous().song.name, 'Sundial');
    });

    scenario('does not go next or previous while a song plays', (p, async) {
      p.play();
      expect(p.next().song.name, 'Harbor Lights');
      expect(p.previous().song.name, 'Harbor Lights');
      expect(p.status().position, 0);
      expect(p.status().playing, isTrue);
    });

    scenario('navigates to an entry the caller has seen', (p, async) {
      expect(p.navigateToSong(2, 'Sundial').song.name, 'Sundial');
      expect(p.status().position, 2);
      expect(
        () => p.navigateToSong(1, 'Sundial'),
        refused(GrpcCode.failedPrecondition, 'playlist has changed'),
      );
      expect(() => p.navigateToSong(9, ''), refused(11, 'no entry 9'));
      // While a song plays, nothing moves and the song playing is answered.
      p.play();
      expect(p.navigateToSong(0, 'Harbor Lights').song.name, 'Sundial');
      expect(p.status().position, 2);
    });

    scenario('switches playlists only while stopped', (p, async) {
      p.switchToPlaylist('Rehearsal list');
      expect(p.status().playlistName, 'Rehearsal list');
      expect(p.status().songName, 'Night Shift');
      expect(
        () => p.switchToPlaylist('nope'),
        refused(GrpcCode.notFound, "'nope'"),
      );
      p.play();
      expect(
        () => p.switchToPlaylist('Short set'),
        refused(GrpcCode.failedPrecondition, 'while playing'),
      );
    });

    scenario('plays a song from another playlist by switching to all songs', (
      p,
      async,
    ) {
      p.playSongFrom('Slow Orbit', const Duration(seconds: 4));
      final status = p.status();
      expect(status.playlistName, 'all_songs');
      expect(status.songName, 'Slow Orbit');
      expect(status.playing, isTrue);
      expect(status.elapsed.toDart(), const Duration(seconds: 4));
      p.stop();
      p.switchToPlaylist('Short set');
      // A song on the active playlist is moved to there.
      p.playSongFrom('Sundial', Duration.zero);
      expect(p.status().playlistName, 'Short set');
      expect(p.status().position, 2);
      p.stop();
      expect(
        () => p.playSongFrom('Nope', Duration.zero),
        refused(GrpcCode.failedPrecondition, 'Nope'),
      );
    });

    scenario('keeps a seek made while stopped for the next play', (p, async) {
      p.seek(const Duration(seconds: 60));
      expect(elapsedOf(p), const Duration(seconds: 60));
      expect(p.status().playing, isFalse);
      p.play();
      async.elapse(const Duration(seconds: 2));
      expect(elapsedOf(p), const Duration(seconds: 62));
      // And restarts a song that plays.
      p.seek(const Duration(seconds: 10));
      async.elapse(const Duration(seconds: 1));
      expect(elapsedOf(p), const Duration(seconds: 11));
      expect(
        () => p.seek(const Duration(seconds: 300)),
        refused(GrpcCode.failedPrecondition, 'at or beyond song length'),
      );
    });

    scenario('seeks to a section', (p, async) {
      p.seekToSection('Verse');
      // Eight bars of four beats at 104 bpm.
      expect(elapsedOf(p).inMilliseconds, closeTo(8 * 4 * 60 / 104 * 1000, 2));
      expect(
        () => p.seekToSection('Solo'),
        refused(GrpcCode.failedPrecondition, "'Solo'"),
      );
    });

    scenario('loops a section while a song plays, and ends the loop', (
      p,
      async,
    ) {
      expect(
        () => p.loopSection('Intro'),
        refused(GrpcCode.failedPrecondition, 'no song is playing'),
      );
      p.play();
      p.loopSection('Intro');
      expect(p.status().loopSection, 'Intro');
      // The intro is 18.46 s; well past it, the song is still in it.
      async.elapse(const Duration(seconds: 50));
      expect(elapsedOf(p).inSeconds, lessThan(19));
      expect(p.status().playing, isTrue);
      p.stopSectionLoop();
      expect(p.status().hasLoopSection(), isFalse);
      async.elapse(const Duration(seconds: 30));
      expect(elapsedOf(p).inSeconds, greaterThan(19));
    });

    scenario('refuses a loop on a section that has gone by', (p, async) {
      p.playFrom(const Duration(seconds: 60));
      expect(
        () => p.loopSection('Intro'),
        refused(GrpcCode.failedPrecondition, 'already passed'),
      );
    });

    scenario('arms a loop on the section on offer when it is acknowledged', (
      p,
      async,
    ) {
      expect(
        () => p.sectionAck(),
        refused(GrpcCode.failedPrecondition, 'No section currently offered'),
      );
      p.play();
      p.sectionAck();
      expect(p.status().loopSection, 'Intro');
      // Acknowledged, there is nothing more on offer in the same section.
      expect(() => p.sectionAck(), refused(GrpcCode.failedPrecondition));
      async.elapse(const Duration(seconds: 40));
      expect(elapsedOf(p).inSeconds, lessThan(19));
      p.stopSectionLoop();
      // The section that was left is not offered again until it is left.
      expect(() => p.sectionAck(), refused(GrpcCode.failedPrecondition));
    });

    scenario('goes round a song that loops until it is broken out of', (
      p,
      async,
    ) {
      p.switchToPlaylist('Rehearsal list');
      p.navigateToSong(1, 'Second Wind');
      p.play();
      final length = p.status().songDuration.toDart();
      async.elapse(length + const Duration(seconds: 20));
      var status = p.status();
      expect(status.playing, isTrue);
      expect(status.songName, 'Second Wind');
      expect(status.elapsed.toDart(), const Duration(seconds: 20));
      // Next breaks out of it and plays the one after.
      expect(p.next().song.name, 'Slow Orbit');
      status = p.status();
      expect(status.songName, 'Slow Orbit');
      expect(status.playing, isTrue);
      expect(status.elapsed.toDart(), Duration.zero);
    });

    scenario('stops a looping song at the end of the list', (p, async) {
      // Second Wind is not last in any list yet: make one that ends on it.
      p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Loop last',
          songs: ['Sundial', 'Second Wind'],
          create_5: true,
        ),
      );
      p.switchToPlaylist('Loop last');
      p.navigateToSong(1, 'Second Wind');
      p.play();
      expect(p.next().song.name, 'Second Wind');
      expect(p.status().playing, isFalse);
    });
  });

  group('the lock', () {
    scenario('refuses what the real player refuses while locked', (p, async) {
      p.setLock(true);
      expect(p.status().locked, isTrue);
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(name: 'Short set', songs: ['Sundial']),
        ),
        locked,
      );
      expect(() => p.deletePlaylist('Rehearsal list'), locked);
      expect(
        () => p.updateSongSettings(
          pp.UpdateSongSettingsRequest(
            name: 'Sundial',
            looping: pp.SongLooping(loopPlayback: true),
          ),
        ),
        locked,
      );
    });

    scenario('leaves playback, levels, ambient light and the lock open', (
      p,
      async,
    ) {
      p.setLock(true);
      p.setTrackGain('click', -6);
      p.setTrackMute('cues', true);
      p.setAmbientMode('off');
      p.next();
      p.play();
      p.stop();
      p.switchToPlaylist('all_songs');
      p.listSongs(0, false);
      p.getSong('Sundial');
      expect(p.setLock(false).locked, isFalse);
      expect(p.status().locked, isFalse);
    });
  });

  group('edits', () {
    scenario('makes, changes and deletes playlists in memory', (p, async) {
      final made = p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Encore',
          songs: ['Sundial', 'Night Shift'],
          create_5: true,
        ),
      );
      expect(made.name, 'Encore');
      expect(made.entries.map((e) => e.songName), ['Sundial', 'Night Shift']);
      expect(p.listPlaylists(false).names, contains('Encore'));
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(
            name: 'Encore',
            songs: ['Sundial'],
            create_5: true,
          ),
        ),
        refused(GrpcCode.alreadyExists),
      );
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(name: 'Missing', songs: ['Sundial']),
        ),
        refused(GrpcCode.notFound),
      );
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(name: 'all_songs', songs: ['Sundial']),
        ),
        refused(GrpcCode.invalidArgument, 'reserved'),
      );
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(name: 'Encore', songs: ['No Such Song']),
        ),
        refused(GrpcCode.invalidArgument, 'No Such Song'),
      );
      final changed = p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Encore',
          songs: ['Night Shift', 'Sundial', 'Copper Wire'],
          expected: ['Sundial', 'Night Shift'],
          checkExpected: true,
          expectedRevision: made.revision,
        ),
      );
      expect(changed.revision, isNot(made.revision));
      expect(changed.entries, hasLength(3));
      // A save made from a list that is no longer the playlist is refused.
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(
            name: 'Encore',
            songs: ['Sundial'],
            expected: ['Sundial', 'Night Shift'],
            checkExpected: true,
          ),
        ),
        refused(GrpcCode.failedPrecondition, 'playlist has changed'),
      );
      p.deletePlaylist('Encore');
      expect(p.listPlaylists(false).names, isNot(contains('Encore')));
      expect(() => p.deletePlaylist('Encore'), refused(GrpcCode.notFound));
      expect(
        () => p.deletePlaylist('all_songs'),
        refused(GrpcCode.invalidArgument),
      );
    });

    scenario('keeps an entry\'s settings with its song unless sent new', (
      p,
      async,
    ) {
      final before = p.getPlaylist('Short set');
      final saved = p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Short set',
          // Copper Wire first, with its auto play, and one added.
          songs: ['Copper Wire', 'Harbor Lights', 'Night Shift'],
        ),
      );
      expect(saved.entries[0].settings.autoPlay.gap, '5s');
      expect(saved.entries[1].settings.fadeOut.length, '4s');
      expect(saved.entries[2].settings.hasAutoPlay(), isFalse);
      expect(saved.revision, isNot(before.revision));
      // New settings replace them.
      final fresh = p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Short set',
          songs: ['Copper Wire', 'Harbor Lights'],
          setSettings: true,
          settings: [
            pp.EntrySettings(fadeIn: pp.Fade(length: '2s')),
            pp.EntrySettings(),
          ],
          setAmbient: true,
        ),
      );
      expect(fresh.entries[0].settings.fadeIn.length, '2s');
      expect(fresh.entries[0].settings.hasAutoPlay(), isFalse);
      expect(fresh.hasAmbient(), isFalse);
      expect(
        () => p.savePlaylist(
          pp.SavePlaylistRequest(
            name: 'Short set',
            songs: ['Copper Wire'],
            setSettings: true,
            settings: [pp.EntrySettings(fadeOut: pp.Fade(length: 'soon'))],
          ),
        ),
        refused(GrpcCode.invalidArgument, 'fade_out'),
      );
    });

    scenario('lets a song that plays go on when an edit takes it off', (
      p,
      async,
    ) {
      p.play();
      p.savePlaylist(
        pp.SavePlaylistRequest(
          name: 'Short set',
          songs: ['Copper Wire', 'Sundial'],
        ),
      );
      var status = p.status();
      expect(status.songName, 'Harbor Lights');
      expect(status.playing, isTrue);
      expect(status.hasPosition(), isFalse, reason: 'off the list');
      async.elapse(const Duration(seconds: 240));
      // It ended, and the entry that took its place is current.
      status = p.status();
      expect(status.playing, isFalse);
      expect(status.songName, 'Copper Wire');
      expect(status.position, 0);
    });

    scenario('changes a song\'s looping, tracks and samples', (p, async) {
      final read = p.getSong('Harbor Lights');
      expect(read.looping.loopPlayback, isFalse);
      final saved = p.updateSongSettings(
        pp.UpdateSongSettingsRequest(
          name: 'Harbor Lights',
          expectedRevision: read.revision,
          looping: pp.SongLooping(
            loopPlayback: true,
            crossfade: '1s',
            cut: ['keys'],
          ),
          tracks: pp.SongTracks(
            tracks: [
              pp.SongTrack(name: 'click', file: 'click.wav'),
              pp.SongTrack(name: 'keys', file: 'keys.wav', fileChannel: 2),
            ],
          ),
          samples: pp.SongSamples(
            samples: [
              ...read.samples.samples,
              pp.SongSample(name: 'rim', file: 'snare.wav'),
            ],
          ),
        ),
      );
      expect(saved.looping.loopPlayback, isTrue);
      expect(saved.tracks.tracks, hasLength(2));
      expect(saved.samples.samples.map((s) => s.name), [
        'kick',
        'rim',
        'snare',
      ]);
      expect(saved.revision, isNot(read.revision));
      expect(
        p
            .listSongs(0, false)
            .songs
            .firstWhere((s) => s.name == saved.name)
            .loops,
        isTrue,
      );
      // The player gives back what it was saved, and the output tracks.
      expect(p.status().songName, 'Harbor Lights');
      // A save from a stale read is refused.
      expect(
        () => p.updateSongSettings(
          pp.UpdateSongSettingsRequest(
            name: 'Harbor Lights',
            expectedRevision: read.revision,
            looping: pp.SongLooping(),
          ),
        ),
        refused(GrpcCode.failedPrecondition, 'song has changed'),
      );
    });

    scenario('says what is wrong with a song it will not save', (p, async) {
      pp.UpdateSongSettingsRequest tracks(pp.SongTrack t) =>
          pp.UpdateSongSettingsRequest(
            name: 'Sundial',
            tracks: pp.SongTracks(tracks: [t]),
          );
      expect(
        () => p.updateSongSettings(
          tracks(pp.SongTrack(name: 'x', file: 'not-here.wav')),
        ),
        refused(GrpcCode.invalidArgument, 'not an audio file'),
      );
      expect(
        () => p.updateSongSettings(
          pp.UpdateSongSettingsRequest(
            name: 'Sundial',
            samples: pp.SongSamples(
              samples: [
                pp.SongSample(name: 'a', file: 'rise.wav'),
                pp.SongSample(name: 'a', file: 'rise.wav'),
              ],
            ),
          ),
        ),
        refused(GrpcCode.invalidArgument, 'two samples'),
      );
      expect(
        () => p.updateSongSettings(
          pp.UpdateSongSettingsRequest(
            name: 'Sundial',
            looping: pp.SongLooping(crossfade: 'a while'),
          ),
        ),
        refused(GrpcCode.invalidArgument, 'crossfade'),
      );
      expect(
        () => p.updateSongSettings(pp.UpdateSongSettingsRequest(name: 'No')),
        refused(GrpcCode.notFound),
      );
      // None of those refusals changed the song.
      expect(p.getSong('Sundial').revision, '0000000000000000');
    });

    scenario('sets gains and mutes, clamped, and counts the changes', (
      p,
      async,
    ) {
      final revision = p.status().gainsRevision;
      expect(p.setTrackGain('click', -90).appliedGainDb, -60);
      expect(p.setTrackGain('click', 40).appliedGainDb, 12);
      expect(p.setTrackGain('click', -3.5).appliedGainDb, -3.5);
      expect(p.setTrackMute('bass', true).muted, isTrue);
      final gains = {for (final g in p.getTrackGains().gains) g.track: g};
      expect(gains['click']!.gainDb, -3.5);
      expect(gains['bass']!.muted, isTrue);
      expect(p.status().gainsRevision, isNot(revision));
      final settled = p.status().gainsRevision;
      p.setTrackGain('click', -3.5);
      expect(p.status().gainsRevision, settled, reason: 'no change');
      expect(
        () => p.setTrackGain('nope', 0),
        refused(GrpcCode.invalidArgument, "'nope'"),
      );
      expect(
        () => p.setTrackGain('click', double.nan),
        refused(GrpcCode.invalidArgument, 'finite'),
      );
    });

    scenario('follows the ambient mode', (p, async) {
      expect(p.status().ambientMode, 'auto');
      expect(p.status().ambientState, 'playing', reason: 'between songs');
      expect(p.setAmbientMode('off').ambient.state, 'off');
      expect(p.status().ambientState, 'off');
      p.setAmbientMode('ON');
      p.play();
      expect(p.status().ambientMode, 'on');
      expect(p.status().ambientState, 'playing');
      p.setAmbientMode('auto');
      expect(p.status().ambientState, 'off', reason: 'the song has lights');
      expect(
        () => p.setAmbientMode('dim'),
        refused(GrpcCode.invalidArgument, 'auto, on or off'),
      );
      expect(p.fullStatus().ambient.mode, 'auto');
      expect(p.listLightShows().paths, isNotEmpty);
    });
  });

  group('the network', () {
    scenario('says nothing of where it is on the network', (p, async) {
      final info = p.networkInfo();
      expect(info.addresses, isEmpty);
      expect(info.hasGrpcPort(), isFalse);
    });
  });

  group('time', () {
    test('uses the injected clock, and leaves no timer after dispose', () {
      fakeAsync((async) {
        final p = DemoPlayer(clock: () => async.elapsed);
        p.play();
        p.setAmbientMode('on');
        p.setTrackGain('click', -1);
        expect(async.pendingTimers, isNotEmpty, reason: 'a song plays');
        p.dispose();
        async.flushMicrotasks();
        expect(async.pendingTimers, isEmpty);
      });
    });

    test('keeps no timer at rest', () {
      fakeAsync((async) {
        final p = DemoPlayer(clock: () => async.elapsed);
        p.previous();
        p.setLock(true);
        expect(async.pendingTimers, isEmpty);
        p.play();
        p.stop();
        expect(async.pendingTimers, isEmpty);
        p.dispose();
      });
    });
  });

  test('the playlist of every song is called as the real player calls it', () {
    expect(demoAllSongs, 'all_songs');
  });
}
