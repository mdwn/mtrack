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

/// The song First with every group set, as `GetSong` answers it.
pp.GetSongResponse fullSongSettings() => pp.GetSongResponse(
  name: 'First',
  duration: seconds(100),
  looping: pp.SongLooping(
    loopPlayback: true,
    crossfade: '2measures',
    cut: ['click'],
  ),
  tracks: pp.SongTracks(
    tracks: [
      pp.SongTrack(name: 'click', file: 'click.wav'),
      pp.SongTrack(name: 'keys', file: 'keys.wav', fileChannel: 2),
    ],
  ),
  samples: pp.SongSamples(
    samples: [
      pp.SongSample(
        name: 'kick',
        originalName: 'kick',
        file: 'kick.wav',
        outputChannels: [1, 2],
        releaseBehavior: 'stop',
        retrigger: 'cut',
        fadeTimeMs: 50,
        velocity: pp.SampleVelocity(mode: 'layers', layers: 2, scale: true),
      ),
    ],
  ),
  triggers: [
    pp.SongTrigger(event: 'Note On, channel 10, key 36', sample: 'kick'),
  ],
  overview: pp.SongOverview(
    tempo: '120 bpm, 4/4',
    sections: ['verse', 'chorus'],
    metronome: true,
    lighting: ['show.light'],
  ),
  files: [
    pp.SongFile(name: 'click.wav', channels: 1),
    pp.SongFile(name: 'keys.wav', channels: 2),
    pp.SongFile(name: 'bass.wav', channels: 1),
  ],
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
    library = [
      ...this.songs,
      RigSong('Encore', lengthSeconds: 150),
      RigSong('Opener', lengthSeconds: 75),
    ]..sort((a, b) => a.name.compareTo(b.name));
    playlists = {playlist: List.of(this.songs), 'all_songs': library};
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

  /// Every song the player has, as `all_songs` lists them.
  late List<RigSong> library;

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

  /// True for a player from before a phone could lock it or edit playlists:
  /// its status has no lock, and it refuses the three calls as not allowed.
  bool oldPlayer = false;

  /// True for a player from before playlist settings: its playlists carry
  /// no settings and no revision, a save's settings are unknown fields it
  /// skips, and it refuses `ListLightShows` as not allowed. [oldPlayer]
  /// implies it.
  bool settingsTooOld = false;

  /// Each playlist's entries' settings, in step with [playlists]; missing
  /// or short means none.
  final Map<String, List<pp.EntrySettings>> entrySettings = {};

  /// Each playlist's own ambient light; missing is the profile's.
  final Map<String, pp.AmbientSetting> playlistAmbient = {};

  /// Bumped on every change to a playlist, its songs or its settings.
  final Map<String, int> revisions = {};

  /// The light shows `ListLightShows` names.
  List<String> lightShows = ['lighting/ambient.light', 'shows/encore.light'];

  bool get _noSettings => settingsTooOld || oldPlayer;

  /// True for a player from before the song library: it refuses
  /// `ListSongs`, `GetSong` and `UpdateSongSettings` as not allowed.
  /// [settingsTooOld] and [oldPlayer] imply it.
  bool songsTooOld = false;

  bool get _noSongs => songsTooOld || _noSettings;

  /// How many songs a page of `ListSongs` holds.
  int songsPage = 1000;

  /// Each song's settings as `GetSong` answers them, by name; a song not
  /// here has a click track and nothing else.
  final Map<String, pp.GetSongResponse> songSettings = {};

  /// Bumped on every change to a song's file.
  final Map<String, int> songRevisions = {};

  /// Every `UpdateSongSettings` the player took, in order.
  final List<pp.UpdateSongSettingsRequest> songUpdates = [];

  /// The song [name] as `GetSong` answers it now.
  pp.GetSongResponse songOf(String name) {
    final song = (songSettings[name] ?? _defaultSong(name)).deepCopy()
      ..revision = 's${songRevisions[name] ?? 0}'
      ..playing = playing && current.name == name;
    return song;
  }

  pp.GetSongResponse _defaultSong(String name) => pp.GetSongResponse(
    name: name,
    duration: seconds(
      library.where((s) => s.name == name).firstOrNull?.lengthSeconds ?? 0,
    ),
    looping: pp.SongLooping(),
    tracks: pp.SongTracks(
      tracks: [pp.SongTrack(name: 'click', file: 'click.wav')],
    ),
    samples: pp.SongSamples(),
    overview: pp.SongOverview(),
    files: [
      pp.SongFile(name: 'click.wav', channels: 1),
      pp.SongFile(name: 'keys.wav', channels: 2),
    ],
  );

  /// What another surface (the web UI) does: changes the song [name]'s
  /// file.
  void editSongElsewhere(String name, void Function(pp.GetSongResponse) edit) {
    final song = (songSettings[name] ?? _defaultSong(name)).deepCopy();
    edit(song);
    songSettings[name] = song;
    songRevisions[name] = (songRevisions[name] ?? 0) + 1;
  }

  /// The settings of the playlist [name]'s entry at [index].
  pp.EntrySettings settingsAt(String name, int index) {
    final list = entrySettings[name] ?? const [];
    return index < list.length ? list[index] : pp.EntrySettings();
  }

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
    locked: oldPlayer ? null : locked,
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

  static bool _sameNames(List<String> a, List<String> b) {
    if (a.length != b.length) {
      return false;
    }
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) {
        return false;
      }
    }
    return true;
  }

  pp.GetPlaylistResponse _playlistResponse(String name, List<RigSong> list) =>
      pp.GetPlaylistResponse(
        name: name,
        entries: [
          for (var i = 0; i < list.length; i++)
            pp.PlaylistEntry(
              songName: list[i].name,
              duration: seconds(list[i].length.inSeconds),
              settings: _noSettings ? null : settingsAt(name, i),
            ),
        ],
        position: name == playlist ? index : null,
        ambient: _noSettings ? null : playlistAmbient[name],
        revision: _noSettings ? null : 'r${revisions[name] ?? 0}',
      );

  /// The settings [chosen] would have after a save of names alone: each
  /// song's carried over from the first entry left of it, as the player's
  /// does.
  List<pp.EntrySettings> _carried(String name, List<RigSong> chosen) {
    final before = playlists[name] ?? const <RigSong>[];
    final taken = List<bool>.filled(before.length, false);
    return [
      for (final song in chosen)
        () {
          for (var i = 0; i < before.length; i++) {
            if (!taken[i] && before[i].name == song.name) {
              taken[i] = true;
              return settingsAt(name, i);
            }
          }
          return pp.EntrySettings();
        }(),
    ];
  }

  /// What another surface (the web UI) does: sets the settings of the
  /// playlist [name]'s entry at [index].
  void editSettingsElsewhere(
    String name,
    int index,
    pp.EntrySettings settings,
  ) {
    final list = [
      for (var i = 0; i < playlists[name]!.length; i++) settingsAt(name, i),
    ];
    list[index] = settings;
    entrySettings[name] = list;
    revisions[name] = (revisions[name] ?? 0) + 1;
    publish();
  }

  static final RegExp _length = RegExp(r'^\d+(\.\d+)?(ms|s|beats?|measures?)$');

  /// What the player would say of [settings] for [song], or null when it
  /// would take them: only lengths are checked here.
  static String? _refusal(String song, pp.EntrySettings settings) {
    String? bad(String field, String text) => _length.hasMatch(text.trim())
        ? null
        : "Invalid transition: song $song: $field: '$text' is not a length "
              '(e.g. 2s, 4beats)';
    return (settings.hasFadeOut()
            ? bad('fade_out', settings.fadeOut.length)
            : null) ??
        (settings.hasFadeIn() ? bad('fade_in', settings.fadeIn.length) : null);
  }

  /// Sets the songs of [name] and, if it is the active playlist, keeps the
  /// current song current when it is still there.
  void _store(
    String name,
    List<RigSong> chosen, {
    List<pp.EntrySettings>? settings,
  }) {
    final current = name == playlist && songs.isNotEmpty ? this.current : null;
    entrySettings[name] = settings ?? _carried(name, chosen);
    revisions[name] = (revisions[name] ?? 0) + 1;
    playlists[name] = chosen;
    if (name == playlist) {
      songs = chosen;
      final at = current == null
          ? -1
          : chosen.indexWhere((s) => s.name == current.name);
      index = at >= 0 ? at : (index >= chosen.length ? 0 : index);
    }
    publish();
  }

  /// What another surface (the web UI) does: sets the songs of the playlist
  /// [name] to those called [names], from the library.
  void editElsewhere(String name, List<String> names) => _store(name, [
    for (final n in names) library.firstWhere((s) => s.name == n),
  ]);

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
    h['GetPlaylist'] = (request) {
      final asked = pp.GetPlaylistRequest.fromBuffer(request.payload).name;
      final name = asked.isEmpty ? playlist : asked;
      final list = name == 'all_songs'
          ? library
          : name == playlist
          ? songs
          : playlists[name];
      if (list == null) {
        return FakeReply.error(5, "Playlist '$name' not found");
      }
      return FakeReply.ok(_playlistResponse(name, list));
    };
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
    FakeReply oldGuard(String method, FakeReply Function() body) => oldPlayer
        ? FakeReply.error(7, 'method $method is not available over Bluetooth')
        : body();
    h['SetLock'] = (request) => oldGuard('SetLock', () {
      locked = pp.SetLockRequest.fromBuffer(request.payload).locked;
      publish();
      return FakeReply.ok(pp.SetLockResponse(locked: locked));
    });
    h['SavePlaylist'] = (request) => oldGuard(
      'SavePlaylist',
      () => guard(() {
        final req = pp.SavePlaylistRequest.fromBuffer(request.payload);
        final name = req.name;
        if (name.isEmpty ||
            name.contains('/') ||
            name.contains(r'\') ||
            name == 'all_songs') {
          return FakeReply.error(3, "Invalid playlist name '$name'");
        }
        final have = playlists[name];
        if (req.create_5 && have != null) {
          return FakeReply.error(6, "Playlist '$name' already exists");
        }
        if (!req.create_5 && have == null) {
          return FakeReply.error(5, "Playlist '$name' not found");
        }
        final chosen = <RigSong>[];
        for (final song in req.songs) {
          final found = library.where((s) => s.name == song);
          if (found.isEmpty) {
            return FakeReply.error(3, "Song '$song' not found");
          }
          chosen.add(found.first);
        }
        final changedSettings =
            !_noSettings &&
            req.expectedRevision.isNotEmpty &&
            req.expectedRevision != 'r${revisions[name] ?? 0}';
        if (req.checkExpected &&
            have != null &&
            (!_sameNames([for (final s in have) s.name], req.expected) ||
                changedSettings)) {
          return FakeReply.error(
            9,
            'the playlist has changed: it no longer holds the songs that '
            'were read',
          );
        }
        final setSettings = req.setSettings && !_noSettings;
        if (setSettings) {
          if (req.settings.length != req.songs.length) {
            return FakeReply.error(
              3,
              'settings has ${req.settings.length} entries, but there are '
              '${req.songs.length} songs: one for each',
            );
          }
          for (var i = 0; i < req.songs.length; i++) {
            final refusal = _refusal(req.songs[i], req.settings[i]);
            if (refusal != null) {
              return FakeReply.error(3, refusal);
            }
          }
        }
        if (req.setAmbient && !_noSettings) {
          if (req.hasAmbient()) {
            playlistAmbient[name] = req.ambient;
          } else {
            playlistAmbient.remove(name);
          }
        }
        _store(
          name,
          chosen,
          settings: setSettings ? List.of(req.settings) : null,
        );
        return FakeReply.ok(
          _playlistResponse(name, name == playlist ? songs : chosen),
        );
      }),
    );
    h['ListLightShows'] = (_) => _noSettings
        ? FakeReply.error(
            7,
            'method ListLightShows is not available over '
            'Bluetooth',
          )
        : FakeReply.ok(pp.ListLightShowsResponse(paths: lightShows));
    FakeReply songsGuard(String method, FakeReply Function() body) => _noSongs
        ? FakeReply.error(7, 'method $method is not available over Bluetooth')
        : body();
    h['ListSongs'] = (request) => songsGuard('ListSongs', () {
      final start = pp.ListSongsRequest.fromBuffer(request.payload).start;
      final rows = [
        for (final song in library)
          pp.SongListing(
            name: song.name,
            duration: seconds(song.lengthSeconds),
            loops: songOf(song.name).looping.loopPlayback,
            hasSamples: songOf(song.name).samples.samples.isNotEmpty,
          ),
      ];
      final end = (start + songsPage).clamp(0, rows.length);
      final page = pp.ListSongsResponse(
        songs: rows.sublist(start.clamp(0, rows.length), end),
        total: rows.length,
      );
      if (end < rows.length) {
        page.next = end;
      }
      return FakeReply.ok(page);
    });
    h['GetSong'] = (request) => songsGuard('GetSong', () {
      final name = pp.GetSongRequest.fromBuffer(request.payload).name;
      if (!library.any((s) => s.name == name)) {
        return FakeReply.error(5, "Song '$name' not found");
      }
      return FakeReply.ok(songOf(name));
    });
    h['UpdateSongSettings'] = (request) => songsGuard(
      'UpdateSongSettings',
      () => guard(() {
        final req = pp.UpdateSongSettingsRequest.fromBuffer(request.payload);
        final name = req.name;
        if (!library.any((s) => s.name == name)) {
          return FakeReply.error(5, "Song '$name' not found");
        }
        if (req.expectedRevision.isNotEmpty &&
            req.expectedRevision != 's${songRevisions[name] ?? 0}') {
          return FakeReply.error(9, 'the song has changed since it was read');
        }
        // Only the crossfade is checked here, as the player's loader would.
        if (req.hasLooping() &&
            req.looping.crossfade.isNotEmpty &&
            !RegExp(r'^\d+(\.\d+)?(ms|s|beats?|measures?)$')
                .hasMatch(req.looping.crossfade)) {
          return FakeReply.error(
            3,
            'song $name: loop_crossfade "${req.looping.crossfade}" is not a '
            'length',
          );
        }
        songUpdates.add(req);
        final song = (songSettings[name] ?? _defaultSong(name)).deepCopy();
        if (req.hasLooping()) {
          song.looping = req.looping;
        }
        if (req.hasTracks()) {
          song.tracks = req.tracks;
        }
        if (req.hasSamples()) {
          song.samples = pp.SongSamples(
            samples: [
              for (final s in req.samples.samples)
                s.deepCopy()..originalName = s.name,
            ],
          );
        }
        songSettings[name] = song;
        songRevisions[name] = (songRevisions[name] ?? 0) + 1;
        return FakeReply.ok(songOf(name));
      }),
    );
    h['DeletePlaylist'] = (request) => oldGuard(
      'DeletePlaylist',
      () => guard(() {
        final name = pp.DeletePlaylistRequest.fromBuffer(request.payload).name;
        if (name == 'all_songs') {
          return FakeReply.error(3, 'all_songs cannot be deleted');
        }
        if (!playlists.containsKey(name)) {
          return FakeReply.error(5, "Playlist '$name' not found");
        }
        playlists.remove(name);
        if (name == playlist) {
          playlist = 'all_songs';
          songs = library;
          index = 0;
        }
        publish();
        return FakeReply.ok(pp.DeletePlaylistResponse());
      }),
    );
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
      final list = name == 'all_songs' ? library : playlists[name];
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
