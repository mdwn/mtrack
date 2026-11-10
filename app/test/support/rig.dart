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
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import 'fake_link.dart';
import 'fake_player.dart';

/// A [pb.Duration] of [seconds], possibly fractional.
pb.Duration seconds(num seconds) =>
    pb.Duration.fromDart(Duration(milliseconds: (seconds * 1000).round()));

/// A beat grid of [measures] measures of [perMeasure] beats at [bpm], from
/// [offset] seconds in.
pp.BeatGrid grid(
  int measures, {
  int perMeasure = 4,
  double bpm = 120,
  double offset = 0,
}) {
  final beat = 60 / bpm;
  final result = pp.BeatGrid();
  for (var i = 0; i < measures * perMeasure; i++) {
    result.beats.add(offset + i * beat);
  }
  for (var m = 0; m < measures; m++) {
    result.measureStarts.add(m * perMeasure);
  }
  return result;
}

/// A section from measure [start] to [end] (exclusive).
pp.Section section(
  String name,
  int start,
  int end, {
  double? startBeat,
  double? endBeat,
}) => pp.Section(
  name: name,
  startMeasure: start,
  endMeasure: end,
  startBeat: startBeat,
  endBeat: endBeat,
);

/// A song of the fake rig.
class RigSong {
  /// A song [name] that is [lengthSeconds] long.
  RigSong(
    this.name, {
    this.lengthSeconds = 120,
    this.sections = const [],
    this.grid,
  });

  /// The song's name.
  final String name;

  /// Its length in seconds.
  final int lengthSeconds;

  /// The song's sections.
  final List<pp.Section> sections;

  /// The song's beat grid.
  final pp.BeatGrid? grid;

  /// Its length.
  Duration get length => Duration(seconds: lengthSeconds);

  /// The song as the player describes it.
  pp.Song toProto() => pp.Song(
    name: name,
    duration: seconds(lengthSeconds),
    beatGrid: grid,
    sections: sections,
  );
}

/// A player with songs and playlists that answers the way the real one does
/// and tells the phone what changed.
/// An output track as the player holds it.
class RigTrack {
  /// A track [name] at [gainDb].
  RigTrack(this.name, [this.gainDb = 0, this.muted = false]);

  /// The track's name.
  final String name;

  /// Its gain in dB.
  double gainDb;

  /// Whether it is muted.
  bool muted;
}

class Rig {
  /// A rig with [songs] in its first playlist.
  Rig({List<RigSong>? songs, this.playlist = 'Set'})
    : songs = songs ?? defaultSongs() {
    player = FakePlayer();
    link = FakeLink(player);
    player.log = link.calls;
    playlists = {playlist: List.of(this.songs)};
    _install();
    publish();
  }

  /// Songs with sections on a grid, for most tests.
  static List<RigSong> defaultSongs() => [
    RigSong(
      'First',
      lengthSeconds: 100,
      sections: [section('Intro', 1, 3), section('Verse', 3, 7)],
      grid: grid(50),
    ),
    RigSong('Second', lengthSeconds: 90),
    RigSong('Third', lengthSeconds: 200),
  ];

  late final FakePlayer player;
  late final FakeLink link;

  /// The active playlist's name.
  String playlist;

  /// Every playlist's songs, by name.
  late Map<String, List<RigSong>> playlists;

  /// The active playlist's songs.
  List<RigSong> songs;

  /// The index of the current entry.
  int index = 0;

  /// Whether a song is playing.
  bool playing = false;

  /// How far into the song, as of the last status.
  Duration elapsed = Duration.zero;

  /// The section a loop is set on.
  String? loop;

  /// A song waiting to start by itself, and when.
  String? autoPlaySong;
  Duration autoPlayIn = Duration.zero;

  /// Whether the player is locked (it refuses what a locked player refuses).
  bool locked = false;

  /// The output tracks, in the order the player lists them.
  List<RigTrack> tracks = [
    RigTrack('Click'),
    RigTrack('Cue', -3),
    RigTrack('Keys', -6),
  ];

  /// The counter the player bumps on any gain or mute change.
  int gainsRevision = 1;

  /// False for a player that refuses the level calls as not allowed.
  bool levelsAllowed = true;

  /// The ambient light's mode, or null for a player without a lighting
  /// engine (or an older one): the status then carries no ambient fields.
  String? ambientMode = 'auto';

  /// What the ambient light is doing.
  String ambientState = 'off';

  /// False when the ambient light has no show to play.
  bool ambientShow = true;

  /// The player's current song.
  RigSong get current => songs[index];

  /// The status the player would send now.
  ble.Status get status => ble.Status(
    playlistName: playlist,
    songName: songs.isEmpty ? '' : current.name,
    songDuration: songs.isEmpty ? null : seconds(current.length.inSeconds),
    playing: playing,
    elapsed: seconds(elapsed.inMilliseconds / 1000),
    position: songs.isEmpty ? null : index,
    autoPlay: autoPlaySong == null
        ? null
        : pp.AutoPlayStatus(
            song: autoPlaySong,
            startsIn: seconds(autoPlayIn.inMilliseconds / 1000),
          ),
    loopSection: loop,
    gainsRevision: levelsAllowed ? gainsRevision : null,
    ambientMode: ambientMode,
    ambientState: ambientMode == null ? null : ambientState,
  );

  /// What a change by another surface (the web UI) does: moves a gain and
  /// bumps the revision.
  void setGainElsewhere(String track, double gainDb) {
    tracks.firstWhere((t) => t.name == track).gainDb = gainDb;
    gainsRevision++;
    publish();
  }

  void _setAmbient(String mode) {
    ambientMode = mode;
    ambientState = switch (mode) {
      'off' => 'off',
      'on' => ambientShow ? 'playing' : 'unavailable',
      _ => playing ? 'off' : (ambientShow ? 'playing' : 'unavailable'),
    };
  }

  /// Makes the status readable and tells the phone.
  void publish() {
    player.statusBytes = status.writeToBuffer();
    if (player.hasStatusListener) {
      player.emitStatus(status);
    }
  }

  void _install() {
    final h = player.handlers;
    FakeReply guard(FakeReply Function() body) => locked
        ? FakeReply.error(9, 'Player is locked. Unlock to make changes.')
        : body();
    h['Status'] = (_) => FakeReply.ok(
      pp.StatusResponse(
        playlistName: playlist,
        currentSong: songs.isEmpty ? null : current.toProto(),
        playing: playing,
      ),
    );
    h['GetPlaylist'] = (_) => FakeReply.ok(
      pp.GetPlaylistResponse(
        name: playlist,
        entries: [
          for (final s in songs)
            pp.PlaylistEntry(
              songName: s.name,
              duration: seconds(s.length.inSeconds),
            ),
        ],
        position: index,
      ),
    );
    h['ListPlaylists'] = (_) => FakeReply.ok(
      pp.ListPlaylistsResponse(
        names: playlists.keys.toList()..sort(),
        active: playlist,
      ),
    );
    FakeReply levelGuard(FakeReply Function() body) =>
        levelsAllowed ? body() : FakeReply.error(7, 'method is not allowed');
    h['GetTrackGains'] = (_) => levelGuard(
      () => FakeReply.ok(
        pp.GetTrackGainsResponse(
          gains: [
            for (final t in tracks)
              pp.TrackGain(track: t.name, gainDb: t.gainDb, muted: t.muted),
          ],
        ),
      ),
    );
    h['SetTrackGain'] = (request) => levelGuard(() {
      final req = pp.SetTrackGainRequest.fromBuffer(request.payload);
      final at = tracks.indexWhere((t) => t.name == req.track);
      if (at < 0) {
        return FakeReply.error(5, "Track '${req.track}' not found");
      }
      tracks[at].gainDb = req.gainDb.clamp(-60.0, 12.0);
      gainsRevision++;
      publish();
      return FakeReply.ok(
        pp.SetTrackGainResponse(appliedGainDb: tracks[at].gainDb),
      );
    });
    h['SetTrackMute'] = (request) => levelGuard(() {
      final req = pp.SetTrackMuteRequest.fromBuffer(request.payload);
      final at = tracks.indexWhere((t) => t.name == req.track);
      if (at < 0) {
        return FakeReply.error(5, "Track '${req.track}' not found");
      }
      tracks[at].muted = req.muted;
      gainsRevision++;
      publish();
      return FakeReply.ok(pp.SetTrackMuteResponse(muted: req.muted));
    });
    h['SetAmbientMode'] = (request) {
      final mode = pp.SetAmbientModeRequest.fromBuffer(request.payload).mode;
      if (!const {'auto', 'on', 'off'}.contains(mode)) {
        return FakeReply.error(3, "Unknown ambient mode '$mode'");
      }
      if (ambientMode == null) {
        return FakeReply.ok(pp.SetAmbientModeResponse());
      }
      _setAmbient(mode);
      publish();
      return FakeReply.ok(
        pp.SetAmbientModeResponse(
          ambient: pp.AmbientStatus(mode: ambientMode, state: ambientState),
        ),
      );
    };
    h['Play'] = (_) => guard(() {
      playing = true;
      publish();
      return FakeReply.ok(pp.PlayResponse(song: current.toProto()));
    });
    h['Stop'] = (_) => guard(() {
      playing = false;
      loop = null;
      publish();
      return FakeReply.ok(pp.StopResponse(song: current.toProto()));
    });
    h['Next'] = (_) => guard(() {
      if (!playing && index + 1 < songs.length) {
        index++;
        elapsed = Duration.zero;
        publish();
      }
      return FakeReply.ok(pp.NextResponse(song: current.toProto()));
    });
    h['Previous'] = (_) => guard(() {
      if (!playing && index > 0) {
        index--;
        elapsed = Duration.zero;
        publish();
      }
      return FakeReply.ok(pp.PreviousResponse(song: current.toProto()));
    });
    h['NavigateToSong'] = (request) => guard(() {
      final req = pp.NavigateToSongRequest.fromBuffer(request.payload);
      final at = req.position;
      if (at >= songs.length) {
        return FakeReply.error(11, 'the playlist has no entry $at');
      }
      if (req.songName.isNotEmpty && songs[at].name != req.songName) {
        return FakeReply.error(
          9,
          'the playlist has changed: entry $at is ${songs[at].name}, '
          'not ${req.songName}',
        );
      }
      if (!playing && at != index) {
        index = at;
        elapsed = Duration.zero;
        publish();
      }
      return FakeReply.ok(pp.NavigateToSongResponse(song: current.toProto()));
    });
    h['SwitchToPlaylist'] = (request) => guard(() {
      final name = pp.SwitchToPlaylistRequest.fromBuffer(request.payload)
          .playlistName;
      final list = playlists[name];
      if (list == null) {
        return FakeReply.error(5, "Playlist '$name' not found");
      }
      if (playing) {
        return FakeReply.error(9, 'Cannot switch playlist while playing');
      }
      playlist = name;
      songs = list;
      index = 0;
      publish();
      return FakeReply.ok(pp.SwitchToPlaylistResponse());
    });
    h['SeekToSection'] = (request) => guard(() {
      final name = pp.SeekToSectionRequest.fromBuffer(request.payload)
          .sectionName;
      final at = current.sections.indexWhere((s) => s.name == name);
      if (at < 0) {
        return FakeReply.error(9, "Section '$name' not found");
      }
      publish();
      return FakeReply.ok(pp.SeekResponse());
    });
    h['LoopSection'] = (request) => guard(() {
      if (!playing) {
        return FakeReply.error(9, 'Cannot loop section: no song is playing');
      }
      loop = pp.LoopSectionRequest.fromBuffer(request.payload).sectionName;
      publish();
      return FakeReply.ok(pp.LoopSectionResponse());
    });
    h['StopSectionLoop'] = (_) => guard(() {
      loop = null;
      publish();
      return FakeReply.ok(pp.StopSectionLoopResponse());
    });
    h['CancelAutoPlay'] = (_) => guard(() {
      final had = autoPlaySong != null;
      autoPlaySong = null;
      publish();
      return FakeReply.ok(pp.CancelAutoPlayResponse(cancelled: had));
    });
  }
}
