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
/// A player that lives inside the app, for the demo: the state and the
/// behaviour of a real player's playback, playlists, songs, levels, lock and
/// ambient light, simulated in memory on a clock. It answers as the real
/// player does (the same messages, the same refusals with the same gRPC
/// codes), minus audio, MIDI and DMX, and writes nothing to disk.
///
/// Time comes from an injected monotonic clock, and the only timer it ever
/// holds is cancelled when it is disposed of.
library;

import 'dart:async';

import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import '../../gen/player/v1/player.pb.dart' as player;
import '../app/playlist_settings.dart';
import '../app/sections.dart';
import '../protocol/player_exception.dart';
import 'demo_content.dart';
import 'demo_stage.dart';

/// How often, at the least, a status goes out while a song plays: the
/// interval the real player's status stream keeps for a client's clock.
const Duration demoStatusInterval = Duration(seconds: 1);

/// Said when a change is refused because the player is locked, as the real
/// player says it.
const String lockedMessage = 'Player is locked. Unlock to make changes.';

PlayerException _precondition(String message) =>
    PlayerException(GrpcCode.failedPrecondition, message);

PlayerException _invalid(String message) =>
    PlayerException(GrpcCode.invalidArgument, message);

PlayerException _notFound(String message) =>
    PlayerException(GrpcCode.notFound, message);

/// A duration the way Rust prints one, as the real player's messages do.
String _rust(Duration d) {
  String trim(double v) {
    final text = v.toStringAsFixed(6);
    return text.replaceFirst(RegExp(r'\.?0+$'), '');
  }

  if (d < const Duration(seconds: 1) && d > Duration.zero) {
    return '${trim(d.inMicroseconds / 1000)}ms';
  }
  return '${trim(d.inMicroseconds / Duration.microsecondsPerSecond)}s';
}

class _Track {
  _Track(this.name, this.gain);

  final String name;
  double gain;
  bool muted = false;
}

class _AutoPlay {
  _AutoPlay(this.song, this.startsAt);

  final String song;
  final Duration startsAt;
}

/// The demo player. See the library comment.
class DemoPlayer {
  /// A fresh demo player: nothing playing, the short set active, the player
  /// unlocked. [clock] is a monotonic clock; the default is the stopwatch.
  DemoPlayer({Duration Function()? clock}) : _now = clock ?? _stopwatch() {
    for (final song in demoSongs()) {
      _songs[song.name] = song;
    }
    _playlists[demoAllSongs] = DemoPlaylist([
      for (final name in _songs.keys) DemoEntry(name),
    ]);
    _playlists.addAll(demoPlaylists());
    for (final (name, gain) in demoOutputTracks) {
      _tracks.add(_Track(name, gain));
    }
    _born = _now();
  }

  static Duration Function() _stopwatch() {
    final watch = Stopwatch()..start();
    return () => watch.elapsed;
  }

  final Duration Function() _now;
  late final Duration _born;

  final Map<String, DemoSong> _songs = {};
  final Map<String, DemoPlaylist> _playlists = {};
  final List<_Track> _tracks = [];
  final StreamController<player.RemoteStatus> _statuses =
      StreamController<player.RemoteStatus>.broadcast();
  final Map<String, List<SectionSpan>> _spanCache = {};

  String _active = 'Short set';
  bool _playing = false;
  Duration _base = Duration.zero;
  Duration _since = Duration.zero;
  Duration? _pending;
  DemoSong? _offListSong;
  _AutoPlay? _auto;
  SectionSpan? _loop;
  SectionSpan? _offered;
  Duration? _suppressed;
  bool _locked = false;
  String _ambientMode = 'auto';
  int _gainsRevision = 1;
  Timer? _timer;
  bool _disposed = false;

  /// Every status the player pushes: on change, and at least once a second
  /// while a song plays. A broadcast stream.
  Stream<player.RemoteStatus> get statuses => _statuses.stream;

  /// The player's time, in seconds since it began, for the stage.
  double get seconds =>
      (_now() - _born).inMicroseconds / Duration.microsecondsPerSecond;

  /// What the stage shows now.
  DemoLook get look {
    final song = _currentSong;
    if (_playing && (song?.hasLighting ?? false)) {
      return DemoLook.chase;
    }
    return _ambientState == 'playing' ? DemoLook.ambient : DemoLook.dark;
  }

  /// Lets go of the timer and ends the status stream. Calls after it are
  /// not expected.
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    _timer?.cancel();
    _timer = null;
    unawaited(_statuses.close());
  }

  // What is current.

  DemoPlaylist get _playlist => _playlists[_active]!;

  DemoSong? _songAt(DemoPlaylist playlist, int index) {
    if (index < 0 || index >= playlist.entries.length) {
      return null;
    }
    return _songs[playlist.entries[index].song];
  }

  DemoSong? get _currentSong =>
      _offListSong ?? _songAt(_playlist, _playlist.cursor);

  List<SectionSpan> _spans(DemoSong song) => _spanCache.putIfAbsent(
    song.name,
    () => resolveSections(song.protoSections, song.beatGrid, song.duration),
  );

  Duration get _elapsed =>
      _playing ? _base + (_now() - _since) : (_pending ?? Duration.zero);

  String get _ambientState {
    switch (_ambientMode) {
      case 'off':
        return 'off';
      case 'on':
        return 'playing';
      default:
        // Auto: between songs, and under a song without a show of its own.
        final song = _currentSong;
        return _playing && (song?.hasLighting ?? false) ? 'off' : 'playing';
    }
  }

  // The clock.

  /// Brings the player up to the clock: a loop that has come round wraps, a
  /// song that has come to its end ends, a song waiting to start starts.
  void _catchUp() {
    final auto = _auto;
    if (!_playing && auto != null && _now() >= auto.startsAt) {
      _auto = null;
      final index = _playlist.entries.indexWhere((e) => e.song == auto.song);
      if (index >= 0 && _playlist.cursor == index) {
        _begin(Duration.zero);
      }
      return;
    }
    if (!_playing) {
      return;
    }
    final song = _currentSong;
    if (song == null) {
      return;
    }
    var elapsed = _base + (_now() - _since);
    final loop = _loop;
    if (loop != null && elapsed >= loop.end!) {
      final length = (loop.end! - loop.start!).inMicroseconds;
      final over = (elapsed - loop.start!).inMicroseconds;
      elapsed = loop.start! + Duration(microseconds: over % length);
      _rebase(elapsed);
    }
    final duration = song.duration;
    if (elapsed >= duration) {
      if (song.looping.loopPlayback) {
        elapsed = Duration(
          microseconds: elapsed.inMicroseconds % duration.inMicroseconds,
        );
        _rebase(elapsed);
        _loop = null;
      } else {
        _finish();
        return;
      }
    }
    _updateOffer(elapsed);
  }

  void _rebase(Duration elapsed) {
    _base = elapsed;
    _since = _now();
  }

  /// The section the playhead is in is offered for a section ack, unless a
  /// loop is on, or the loop was just broken out of.
  void _updateOffer(Duration elapsed) {
    final song = _currentSong;
    if (_loop != null || song == null) {
      _offered = null;
      return;
    }
    final at = sectionAt(_spans(song), elapsed);
    if (at == null) {
      _offered = null;
      _suppressed = null;
      return;
    }
    _offered = at.start == _suppressed ? null : at;
  }

  /// A song played to its end: the playlist moves on, and the entry it
  /// reaches may start by itself. The last song of the list stays current.
  void _finish() {
    _playing = false;
    _base = Duration.zero;
    _pending = null;
    _loop = null;
    _offered = null;
    _suppressed = null;
    var moved = false;
    if (_offListSong != null) {
      // Taken off the list while it played: the entry that took its place.
      _offListSong = null;
      final last = _playlist.entries.length - 1;
      if (_playlist.cursor > last) {
        _playlist.cursor = last;
      } else {
        moved = true;
      }
    } else if (_playlist.cursor < _playlist.entries.length - 1) {
      _playlist.cursor++;
      moved = true;
    }
    if (moved) {
      _scheduleAutoPlay();
    }
  }

  void _scheduleAutoPlay() {
    final entry = _playlist.entries[_playlist.cursor];
    if (!entry.settings.hasAutoPlay()) {
      return;
    }
    final gap = _parseTime(entry.settings.autoPlay.gap);
    if (gap == Duration.zero) {
      _begin(Duration.zero);
      return;
    }
    _auto = _AutoPlay(entry.song, _now() + gap);
  }

  /// A time as the player's settings write one (`2s`, `500ms`, `1m30s`); zero
  /// for none or one it cannot read.
  static Duration _parseTime(String text) {
    final cleaned = text.replaceAll(RegExp(r'\s'), '');
    var total = 0.0;
    for (final part in RegExp(
      r'([0-9]*\.?[0-9]+)(ms|s|m|h)',
    ).allMatches(cleaned)) {
      final value = double.parse(part.group(1)!);
      total +=
          value *
          switch (part.group(2)) {
            'ms' => 0.001,
            's' => 1,
            'm' => 60,
            _ => 3600,
          };
    }
    return Duration(
      microseconds: (total * Duration.microsecondsPerSecond).round(),
    );
  }

  /// Starts the current song at [start].
  void _begin(Duration start) {
    _playing = true;
    _base = start;
    _since = _now();
    _pending = null;
    _auto = null;
    _loop = null;
    _offered = null;
    _suppressed = null;
    _updateOffer(start);
  }

  /// Stops the song playing, as a stop does: where it was is forgotten.
  void _stopNow() {
    _playing = false;
    _base = Duration.zero;
    _pending = null;
    _loop = null;
    _offered = null;
    _suppressed = null;
    if (_offListSong != null) {
      _offListSong = null;
      _playlist.cursor = _playlist.cursor.clamp(
        0,
        _playlist.entries.length - 1,
      );
    }
  }

  /// Wakes the player when something is next due: the song's end, the loop's
  /// end, a song waiting to start, and a second on for the status.
  void _reschedule() {
    _timer?.cancel();
    _timer = null;
    if (_disposed) {
      return;
    }
    Duration? wait;
    final song = _currentSong;
    if (_playing && song != null) {
      final elapsed = _elapsed;
      var next = demoStatusInterval;
      final end = _loop?.end ?? song.duration;
      final left = end - elapsed;
      if (left > Duration.zero && left < next) {
        next = left;
      }
      wait = next;
    } else if (_auto != null) {
      final left = _auto!.startsAt - _now();
      wait = left > Duration.zero ? left : Duration.zero;
    }
    if (wait == null) {
      return;
    }
    _timer = Timer(wait, () {
      _timer = null;
      _catchUp();
      _push();
    });
  }

  /// Tells whoever is listening where things stand, and sets the next wake.
  void _push() {
    if (_disposed) {
      return;
    }
    _reschedule();
    if (_statuses.hasListener) {
      _statuses.add(status());
    }
  }

  // Status.

  /// The status as a phone watches it.
  player.RemoteStatus status() {
    _catchUp();
    final song = _currentSong;
    final status = player.RemoteStatus(
      playlistName: _active,
      songName: song?.name ?? '',
      songDuration: song == null ? null : pb.Duration.fromDart(song.duration),
      playing: _playing,
      elapsed: pb.Duration.fromDart(_elapsed),
      position: _offListSong == null && song != null ? _playlist.cursor : null,
      gainsRevision: _gainsRevision,
      ambientMode: _ambientMode,
      ambientState: _ambientState,
      locked: _locked,
    );
    final auto = _auto;
    if (auto != null && !_playing) {
      final left = auto.startsAt - _now();
      status.autoPlay = player.AutoPlayStatus(
        song: auto.song,
        startsIn: pb.Duration.fromDart(left.isNegative ? Duration.zero : left),
      );
    }
    final loop = _loop;
    if (loop != null) {
      status.loopSection = loop.name;
    }
    return status;
  }

  /// The player's full status, with the song's beat grid and sections.
  player.StatusResponse fullStatus() {
    _catchUp();
    final song = _currentSong;
    final response = player.StatusResponse(
      playlistName: _active,
      playing: _playing,
      elapsed: pb.Duration.fromDart(_elapsed),
      ambient: _ambientStatus(),
    );
    if (song != null) {
      response.currentSong = song.toProto();
    }
    final auto = _auto;
    if (auto != null && !_playing) {
      final left = auto.startsAt - _now();
      response.autoPlay = player.AutoPlayStatus(
        song: auto.song,
        startsIn: pb.Duration.fromDart(left.isNegative ? Duration.zero : left),
      );
    }
    return response;
  }

  player.AmbientStatus _ambientStatus() {
    final state = _ambientState;
    return player.AmbientStatus(
      mode: _ambientMode,
      state: state,
      show: state == 'playing' ? demoAmbientShow : null,
      source: state == 'playing' ? 'profile' : null,
    );
  }

  // Playback.

  /// Plays the current song.
  player.PlayResponse play() => _play(null);

  /// Plays the current song from [start].
  player.PlayResponse playFrom(Duration start) => _play(start);

  player.PlayResponse _play(Duration? from) {
    _catchUp();
    if (_playing) {
      _declinedWhilePlaying();
    }
    final song = _currentSong;
    if (song == null) {
      throw _precondition(
        'the playlist is empty — no songs loaded, which usually means none '
        'passed validation at startup (check the log)',
      );
    }
    final start = from ?? _pending ?? Duration.zero;
    try {
      _checkStart(song, start, 'play from');
    } on PlayerException {
      if (_pending == start) {
        _pending = null;
        _push();
      }
      rethrow;
    }
    _begin(start);
    _push();
    return player.PlayResponse(song: song.toProto());
  }

  /// A play while a song plays does not start one. A looping song is broken
  /// out of first, as the real player breaks out of it: the playlist moves on
  /// and what it reaches plays.
  Never _declinedWhilePlaying() {
    if (_currentSong?.looping.loopPlayback ?? false) {
      _breakLoop(back: false);
      _push();
    }
    throw _precondition('song already playing');
  }

  /// Plays the song [name] from [start], moving to it in the active playlist
  /// or, when it is not there, switching to the playlist of every song.
  player.PlayResponse playSongFrom(String name, Duration start) {
    _catchUp();
    if (_playing) {
      _declinedWhilePlaying();
    }
    final song = _songs[name];
    if (song == null) {
      throw _precondition("song '$name' not found");
    }
    _checkStart(song, start, 'play from');
    var index = _playlist.entries.indexWhere((e) => e.song == name);
    if (index < 0) {
      _active = demoAllSongs;
      index = _playlist.entries.indexWhere((e) => e.song == name);
    }
    _playlist.cursor = index;
    _auto = null;
    _begin(start);
    _push();
    return player.PlayResponse(song: song.toProto());
  }

  /// Refuses a start at or past the song's end.
  static void _checkStart(DemoSong song, Duration position, String action) {
    final length = song.duration;
    if (length > Duration.zero && position >= length) {
      throw _precondition(
        'Cannot $action ${_rust(position)}: at or beyond song length '
        '${_rust(length)}',
      );
    }
  }

  /// Stops the song playing.
  player.StopResponse stop() {
    _catchUp();
    if (!_playing) {
      throw _precondition('song not playing');
    }
    final song = _currentSong!;
    _stopNow();
    _push();
    return player.StopResponse(song: song.toProto());
  }

  /// Jumps to [position] in the current song: at once while playing, and
  /// kept for the next play while stopped. Ends a section loop.
  player.SeekResponse seek(Duration position) {
    _catchUp();
    final song = _currentSong;
    if (song == null) {
      throw _precondition('Cannot seek: playlist is empty');
    }
    _checkStart(song, position, 'seek to');
    if (_playing) {
      _rebase(position);
      _loop = null;
      _suppressed = null;
      _updateOffer(position);
    } else {
      _pending = position;
      _loop = null;
    }
    _push();
    return player.SeekResponse();
  }

  /// Jumps to the start of the first section called [name].
  player.SeekResponse seekToSection(String name) {
    _catchUp();
    final song = _currentSong;
    if (song == null) {
      throw _precondition('Cannot seek: playlist is empty');
    }
    final span = _span(song, name);
    if (span == null) {
      throw _precondition(
        "Section '$name' not found or cannot be resolved (missing beat grid?)",
      );
    }
    return seek(span.start!);
  }

  SectionSpan? _span(DemoSong song, String name) {
    for (final span in _spans(song)) {
      if (span.name == name && span.hasTimes) {
        return span;
      }
    }
    return null;
  }

  /// Loops the section called [name] of the song playing.
  player.LoopSectionResponse loopSection(String name) {
    _catchUp();
    final song = _currentSong;
    if (!_playing || song == null) {
      throw _precondition('Cannot loop section: no song is playing');
    }
    final span = _span(song, name);
    if (span == null) {
      throw _precondition(
        "Section '$name' not found or cannot be resolved (missing beat grid?)",
      );
    }
    if (_elapsed >= span.end!) {
      throw _precondition(
        "Cannot loop section '$name': playback has already passed the section end",
      );
    }
    _loop = span;
    _offered = null;
    _suppressed = null;
    _push();
    return player.LoopSectionResponse();
  }

  /// Ends the section loop: the pass in progress plays out and the song goes
  /// on from the section's end.
  player.StopSectionLoopResponse stopSectionLoop() {
    _catchUp();
    final loop = _loop;
    if (loop != null) {
      _suppressed = loop.start;
      _loop = null;
      _offered = null;
    }
    _push();
    return player.StopSectionLoopResponse();
  }

  /// Acknowledges the section on offer: its loop engages at its end.
  player.SectionAckResponse sectionAck() {
    _catchUp();
    final offered = _offered;
    if (offered == null) {
      throw _precondition('No section currently offered to acknowledge');
    }
    _loop = offered;
    _offered = null;
    _push();
    return player.SectionAckResponse();
  }

  /// Stops the triggered samples that sound. The demo has none that do.
  player.StopSamplesResponse stopSamples() => player.StopSamplesResponse();

  /// Calls off the song waiting to start by itself.
  player.CancelAutoPlayResponse cancelAutoPlay() {
    _catchUp();
    final cancelled = _auto != null;
    _auto = null;
    if (cancelled) {
      _push();
    }
    return player.CancelAutoPlayResponse(cancelled: cancelled);
  }

  // Navigation.

  /// Moves to the next entry, or does as the player does while a song plays.
  player.NextResponse next() =>
      player.NextResponse(song: _navigate(back: false));

  /// Moves to the previous entry.
  player.PreviousResponse previous() =>
      player.PreviousResponse(song: _navigate(back: true));

  player.Song _navigate({required bool back}) {
    _catchUp();
    final playlist = _playlist;
    if (playlist.entries.isEmpty) {
      throw _precondition('playlist is empty');
    }
    if (_playing) {
      // A song that plays does not give way, unless it loops: that one is
      // broken out of, and what it reaches plays at once.
      if (_currentSong!.looping.loopPlayback) {
        _breakLoop(back: back);
        _push();
      }
      return _currentSong!.toProto();
    }
    final from = playlist.cursor;
    final target = back ? from - 1 : from + 1;
    if (target >= 0 && target < playlist.entries.length) {
      playlist.cursor = target;
    }
    _movedWhileStopped(moved: playlist.cursor != from);
    return _currentSong!.toProto();
  }

  /// What a navigation does once the playlist is on its entry: a kept
  /// position, a section loop and a song waiting are called off, and the
  /// entry may start by itself.
  void _movedWhileStopped({required bool moved}) {
    _pending = null;
    _loop = null;
    _offered = null;
    _auto = null;
    if (moved) {
      _scheduleAutoPlay();
    }
    _push();
  }

  /// Breaks out of the looping song playing: on to the next (or previous)
  /// entry, which plays at once; where there is none, the song stops.
  void _breakLoop({required bool back}) {
    final playlist = _playlist;
    final target = playlist.cursor + (back ? -1 : 1);
    if (target < 0 || target >= playlist.entries.length) {
      _stopNow();
      return;
    }
    playlist.cursor = target;
    _begin(Duration.zero);
  }

  /// Makes the entry at [position] current without playing it. [songName] is
  /// the song the caller believes is there.
  player.NavigateToSongResponse navigateToSong(int position, String songName) {
    _catchUp();
    final playlist = _playlist;
    if (_playing) {
      final song = _currentSong;
      if (song == null) {
        throw PlayerException(
          _outOfRange,
          'the playlist has no entry $position (it has 0)',
        );
      }
      return player.NavigateToSongResponse(song: song.toProto());
    }
    if (position < 0 || position >= playlist.entries.length) {
      throw PlayerException(
        _outOfRange,
        'the playlist has no entry $position (it has ${playlist.entries.length})',
      );
    }
    final found = playlist.entries[position].song;
    if (songName.isNotEmpty && songName != found) {
      throw _precondition(
        'the playlist has changed: entry $position is $found, not $songName',
      );
    }
    if (playlist.cursor != position) {
      playlist.cursor = position;
      _movedWhileStopped(moved: true);
    }
    return player.NavigateToSongResponse(song: _songs[found]!.toProto());
  }

  /// gRPC's OUT_OF_RANGE.
  static const int _outOfRange = 11;

  /// Switches to the playlist [name]; refused while a song plays.
  player.SwitchToPlaylistResponse switchToPlaylist(String name) {
    _catchUp();
    if (_playing) {
      throw _precondition('Cannot switch playlist while playing');
    }
    if (!_playlists.containsKey(name)) {
      throw _notFound("Playlist '$name' not found");
    }
    _active = name;
    _pending = null;
    _loop = null;
    _offered = null;
    _auto = null;
    _push();
    return player.SwitchToPlaylistResponse();
  }

  // Playlists.

  /// The names of the playlists, sorted as the real player sorts them, and
  /// which is active.
  player.ListPlaylistsResponse listPlaylists(bool detailed) {
    final names = _playlists.keys.toList()..sort();
    return player.ListPlaylistsResponse(
      names: names,
      active: _active,
      playlists: detailed
          ? [
              for (final name in names)
                player.PlaylistSummary(
                  name: name,
                  songCount: _playlists[name]!.entries.length,
                  reserved: name == demoAllSongs,
                ),
            ]
          : null,
    );
  }

  /// The playlist [name], or the active one when it is empty.
  player.GetPlaylistResponse getPlaylist(
    String name, {
    bool withLibrary = false,
    bool withPalettes = false,
  }) {
    _catchUp();
    final key = name.isEmpty ? _active : name;
    final playlist = _playlists[key];
    if (playlist == null) {
      throw _notFound("Playlist '$name' not found");
    }
    final response = _playlistResponse(key, playlist);
    if (withLibrary) {
      for (final song in _songs.values) {
        response.availableSongs.add(song.name);
        if (song.palette != null) {
          response.songPalettes[song.name] = song.palette!;
        }
        response.songTracks[song.name] = player.TrackNames(
          names: song.trackNames,
        );
      }
    }
    if (withPalettes) {
      for (final entry in playlist.entries) {
        final palette = _songs[entry.song]?.palette;
        if (palette != null) {
          response.songPalettes[entry.song] = palette;
        }
      }
    }
    return response;
  }

  player.GetPlaylistResponse _playlistResponse(
    String name,
    DemoPlaylist playlist,
  ) {
    final offList = name == _active && _offListSong != null;
    return player.GetPlaylistResponse(
      name: name,
      entries: [
        for (final entry in playlist.entries)
          player.PlaylistEntry(
            songName: entry.song,
            duration: pb.Duration.fromDart(_songs[entry.song]!.duration),
            settings: entry.settings,
          ),
      ],
      position: offList || playlist.entries.isEmpty ? null : playlist.cursor,
      ambient: playlist.ambient,
      revision: playlist.revision.toRadixString(16).padLeft(16, '0'),
    );
  }

  void _requireUnlocked() {
    if (_locked) {
      throw _precondition(lockedMessage);
    }
  }

  /// Sets the songs, and with them the settings, of a playlist.
  player.GetPlaylistResponse savePlaylist(player.SavePlaylistRequest request) {
    _requireUnlocked();
    final name = request.name;
    _checkPlaylistName(name);
    final existing = _playlists[name];
    if (request.create_5 && existing != null) {
      throw PlayerException(
        GrpcCode.alreadyExists,
        "Playlist '$name' already exists",
      );
    }
    if (!request.create_5 && existing == null) {
      throw _notFound("Playlist '$name' not found");
    }
    if (request.songs.isEmpty) {
      throw _invalid('a playlist needs at least one song');
    }
    if (existing != null && request.checkExpected) {
      final songs = [for (final e in existing.entries) e.song];
      final revision = existing.revision.toRadixString(16).padLeft(16, '0');
      if (!_sameList(songs, request.expected) ||
          (request.expectedRevision.isNotEmpty &&
              request.expectedRevision != revision)) {
        throw _precondition('the playlist has changed since it was read');
      }
    } else if (existing == null &&
        request.checkExpected &&
        request.expected.isNotEmpty) {
      throw _precondition('the playlist has changed since it was read');
    }
    for (final song in request.songs) {
      if (!_songs.containsKey(song)) {
        throw _invalid("song '$song' not found");
      }
    }
    // Each entry keeps its settings with its song, unless new ones are sent.
    final kept = <String, List<player.EntrySettings>>{};
    for (final entry in existing?.entries ?? const <DemoEntry>[]) {
      (kept[entry.song] ??= []).add(entry.settings);
    }
    List<DemoEntry> entries;
    if (request.setSettings) {
      if (request.settings.length != request.songs.length) {
        throw _invalid(
          'settings for ${request.settings.length} entries, '
          'but the playlist has ${request.songs.length}',
        );
      }
      for (var i = 0; i < request.songs.length; i++) {
        _checkEntry(request.songs[i], request.settings[i]);
      }
      entries = [
        for (var i = 0; i < request.songs.length; i++)
          DemoEntry(request.songs[i], request.settings[i].deepCopy()),
      ];
    } else {
      entries = [
        for (final song in request.songs)
          DemoEntry(
            song,
            (kept[song]?.isNotEmpty ?? false)
                ? kept[song]!.removeAt(0)
                : player.EntrySettings(),
          ),
      ];
    }
    player.AmbientSetting? ambient = existing?.ambient;
    if (request.setAmbient) {
      ambient = request.hasAmbient() ? request.ambient.deepCopy() : null;
      _checkAmbient(ambient);
    }
    _catchUp();
    final playlist = existing ?? DemoPlaylist([]);
    final before = existing == null ? null : _cursorSong(name);
    playlist
      ..entries = entries
      ..ambient = ambient
      ..revision += 1;
    if (existing == null) {
      _playlists[name] = playlist;
    }
    _reloaded(name, playlist, before);
    _push();
    return _playlistResponse(name, playlist);
  }

  String? _cursorSong(String name) {
    final playlist = _playlists[name]!;
    if (name == _active && _offListSong != null) {
      return _offListSong!.name;
    }
    return playlist.cursor < playlist.entries.length
        ? playlist.entries[playlist.cursor].song
        : null;
  }

  /// The playlist was rewritten: it stays where it was. Where its current
  /// song is still on it, that entry (the nearest) is current; a song that
  /// plays and is gone plays on as off the list.
  void _reloaded(String name, DemoPlaylist playlist, String? before) {
    final last = playlist.entries.length - 1;
    var cursor = playlist.cursor;
    if (before != null) {
      final same = [
        for (var i = 0; i < playlist.entries.length; i++)
          if (playlist.entries[i].song == before) i,
      ];
      if (same.isNotEmpty) {
        same.sort((a, b) => (a - cursor).abs().compareTo((b - cursor).abs()));
        cursor = same.first;
        if (name == _active) {
          _offListSong = null;
        }
      } else if (name == _active && _playing) {
        _offListSong = _songs[before];
        cursor = cursor.clamp(0, playlist.entries.length);
        playlist.cursor = cursor > last ? last : cursor;
        return;
      } else if (name == _active) {
        _pending = null;
        _loop = null;
      }
    }
    playlist.cursor = cursor.clamp(0, last);
  }

  static bool _sameList(List<String> a, List<String> b) {
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

  static void _checkPlaylistName(String name) {
    if (name.toLowerCase() == demoAllSongs) {
      throw _invalid(
        "the name '$name' is reserved for the player's own playlist",
      );
    }
    if (name.trim().isEmpty ||
        name.contains('/') ||
        name.contains(r'\') ||
        name.contains('..')) {
      throw _invalid("invalid playlist name '$name'");
    }
  }

  void _checkEntry(String song, player.EntrySettings settings) {
    for (final (field, fade) in [
      ('fade_out', settings.hasFadeOut() ? settings.fadeOut : null),
      ('fade_in', settings.hasFadeIn() ? settings.fadeIn : null),
    ]) {
      if (fade != null && !looksLikeLength(fade.length)) {
        throw _invalid(
          'song $song: $field: ${fade.length} is not a length (try 2s, 500ms, '
          '4beats or 1measure)',
        );
      }
    }
    if (settings.hasAutoPlay() && !looksLikeTime(settings.autoPlay.gap)) {
      throw _invalid(
        'song $song: auto_play: ${settings.autoPlay.gap} is not a time',
      );
    }
    if (settings.hasAmbient()) {
      _checkAmbient(settings.ambient);
    }
  }

  void _checkAmbient(player.AmbientSetting? ambient) {
    if (ambient == null) {
      return;
    }
    for (final (field, value) in [
      ('delay', ambient.hasDelay() ? ambient.delay : ''),
      ('fade_in', ambient.hasFadeIn() ? ambient.fadeIn : ''),
      ('fade_out', ambient.hasFadeOut() ? ambient.fadeOut : ''),
    ]) {
      if (!looksLikeTime(value)) {
        throw _invalid('ambient $field: $value is not a time');
      }
    }
    if (ambient.hasShow() &&
        (ambient.show.startsWith('/') || ambient.show.contains('..'))) {
      throw _invalid(
        'ambient show "${ambient.show}": must be a path inside the project, '
        'relative to it and without ".."',
      );
    }
  }

  /// Deletes the playlist [name]. One that is active goes back to every song.
  player.DeletePlaylistResponse deletePlaylist(String name) {
    _requireUnlocked();
    _checkPlaylistName(name);
    if (!_playlists.containsKey(name)) {
      throw _notFound("Playlist '$name' not found");
    }
    _catchUp();
    _playlists.remove(name);
    if (_active == name) {
      _active = demoAllSongs;
      _offListSong = null;
      _pending = null;
      _loop = null;
      _auto = null;
      if (_playing) {
        _stopNow();
      }
    }
    _push();
    return player.DeletePlaylistResponse();
  }

  /// The `.light` files an ambient light can play.
  player.ListLightShowsResponse listLightShows() =>
      player.ListLightShowsResponse(paths: demoLightShows);

  // Songs.

  /// A page of the library from [start]: every song, sorted by name. The
  /// demo's library fits one page.
  player.ListSongsResponse listSongs(int start, bool detailed) {
    final songs = _songs.values.toList();
    return player.ListSongsResponse(
      songs: [
        for (var i = start; i < songs.length; i++) _listing(songs[i], detailed),
      ],
      total: songs.length,
    );
  }

  static String _baseDir(DemoSong song) =>
      song.folder.isEmpty ? song.name : '${song.folder}/${song.name}';

  player.SongListing _listing(DemoSong song, bool detailed) {
    final row = player.SongListing(
      name: song.name,
      duration: pb.Duration.fromDart(song.duration),
      loops: song.looping.loopPlayback,
      hasLighting: song.hasLighting,
      hasMidi: false,
      hasSamples: song.sampleList.isNotEmpty,
      baseDir: _baseDir(song),
    );
    if (detailed) {
      row
        ..channels = 2
        ..sampleFormat = 'int'
        ..tracks.addAll(song.trackNames)
        ..beatGrid = song.beatGrid
        ..sections.addAll(song.protoSections);
      if (song.hasLighting) {
        row.lightingFiles.add('songs/${song.name}/${song.name}.light');
      }
      if (song.palette != null) {
        row.palette = song.palette!;
      }
      row.loopCrossfadeCut.addAll(song.looping.cut);
    }
    return row;
  }

  DemoSong _song(String name) {
    final song = _songs[name];
    if (song == null) {
      throw _notFound("Song '$name' not found");
    }
    return song;
  }

  /// The song [name]'s settings.
  player.GetSongResponse getSong(String name) {
    _catchUp();
    final song = _song(name);
    final samples = song.sampleList.toList()
      ..sort((a, b) => a.name.compareTo(b.name));
    return player.GetSongResponse(
      name: song.name,
      duration: pb.Duration.fromDart(song.duration),
      looping: song.looping,
      tracks: player.SongTracks(tracks: song.trackList),
      samples: player.SongSamples(samples: samples),
      triggers: song.triggers,
      overview: player.SongOverview(
        tempo: '${_bpmText(song.bpm)} BPM, ${song.beatsPerBar}/4',
        sections: [for (final s in song.sections) s.name],
        lighting: song.hasLighting ? ['${song.name}.light'] : null,
      ),
      files: song.files,
      revision: song.revision.toRadixString(16).padLeft(16, '0'),
      playing: _playing && _currentSong?.name == name,
      baseDir: _baseDir(song),
    );
  }

  static String _bpmText(double bpm) =>
      bpm == bpm.roundToDouble() ? '${bpm.round()}' : '$bpm';

  /// Changes the song's looping, tracks or samples, each only if sent.
  player.GetSongResponse updateSongSettings(
    player.UpdateSongSettingsRequest request,
  ) {
    _requireUnlocked();
    final song = _song(request.name);
    final revision = song.revision.toRadixString(16).padLeft(16, '0');
    if (request.expectedRevision.isNotEmpty &&
        request.expectedRevision != revision) {
      throw _precondition('the song has changed since it was read');
    }
    final files = {for (final f in song.files) f.name};
    var tracks = song.trackList;
    if (request.hasTracks()) {
      final seen = <String>{};
      for (final track in request.tracks.tracks) {
        if (track.name.trim().isEmpty) {
          throw _invalid('a track needs a name');
        }
        if (!seen.add(track.name)) {
          throw _invalid('two tracks are named "${track.name}"');
        }
        if (!files.contains(track.file)) {
          throw _invalid(
            '"${track.file}" is not an audio file in the song\'s folder',
          );
        }
      }
      tracks = List.unmodifiable([
        for (final t in request.tracks.tracks) t.deepCopy(),
      ]);
    }
    var looping = song.looping;
    if (request.hasLooping()) {
      final next = request.looping;
      if (!looksLikeTime(next.crossfade)) {
        throw _invalid('crossfade "${next.crossfade}" is not a time');
      }
      looping = next.deepCopy();
    }
    final names = {for (final t in tracks) t.name};
    for (final cut in looping.cut) {
      if (!names.contains(cut)) {
        throw _invalid('crossfade cut: "$cut" is not a track of the song');
      }
    }
    var samples = song.sampleList;
    if (request.hasSamples()) {
      final sampleNames = <String>{};
      final originals = <String>{};
      for (final sample in request.samples.samples) {
        if (sample.name.trim().isEmpty) {
          throw _invalid('a sample needs a name');
        }
        if (sample.name != sample.name.trim()) {
          throw _invalid(
            'sample name "${sample.name}" starts or ends with a space',
          );
        }
        if (!sampleNames.add(sample.name)) {
          throw _invalid('two samples are named "${sample.name}"');
        }
        if (sample.originalName.isNotEmpty) {
          if (!song.sampleList.any((s) => s.name == sample.originalName)) {
            throw _invalid(
              'sample "${sample.originalName}" is not in the song',
            );
          }
          if (!originals.add(sample.originalName)) {
            throw _invalid('sample "${sample.originalName}" is listed twice');
          }
        }
        if (sample.hasFile() && !files.contains(sample.file)) {
          throw _invalid(
            '"${sample.file}" is not an audio file in the song\'s folder',
          );
        }
      }
      samples = List.unmodifiable([
        for (final s in request.samples.samples)
          s.deepCopy()..originalName = s.name,
      ]);
    }
    final changed =
        !_sameMessages(tracks, song.trackList) ||
        looping != song.looping ||
        !_sameMessages(samples, song.sampleList);
    if (changed) {
      song
        ..trackList = tracks
        ..looping = looping
        ..sampleList = samples
        ..revision += 1;
      // The loop switch takes effect at once on a song that plays.
      _push();
    }
    return getSong(song.name);
  }

  static bool _sameMessages<T>(List<T> a, List<T> b) {
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

  // The lock, the levels and the ambient light.

  /// Locks the player, or unlocks it. Allowed while locked.
  player.SetLockResponse setLock(bool locked) {
    _locked = locked;
    _push();
    return player.SetLockResponse(locked: _locked);
  }

  /// The gain and mute of every output track, in the player's order.
  player.GetTrackGainsResponse getTrackGains() => player.GetTrackGainsResponse(
    gains: [
      for (final t in _tracks)
        player.TrackGain(track: t.name, gainDb: t.gain, muted: t.muted),
    ],
  );

  _Track _track(String name) {
    for (final track in _tracks) {
      if (track.name == name) {
        return track;
      }
    }
    throw _invalid("unknown track '$name'");
  }

  /// Sets a track's gain, clamped to -60 to +12 dB.
  player.SetTrackGainResponse setTrackGain(String name, double gainDb) {
    if (!gainDb.isFinite) {
      throw _invalid('gain must be a finite number, got $gainDb');
    }
    final track = _track(name);
    final applied = gainDb.clamp(-60.0, 12.0).toDouble();
    if (track.gain != applied) {
      track.gain = applied;
      _gainsRevision++;
      _push();
    }
    return player.SetTrackGainResponse(appliedGainDb: applied);
  }

  /// Mutes or unmutes a track.
  player.SetTrackMuteResponse setTrackMute(String name, bool muted) {
    final track = _track(name);
    if (track.muted != muted) {
      track.muted = muted;
      _gainsRevision++;
      _push();
    }
    return player.SetTrackMuteResponse(muted: muted);
  }

  /// Sets the ambient light's mode: `auto`, `on` or `off`.
  player.SetAmbientModeResponse setAmbientMode(String mode) {
    final name = mode.trim().toLowerCase();
    if (!const ['auto', 'on', 'off'].contains(name)) {
      throw _invalid("unknown ambient mode '$mode': use auto, on or off");
    }
    _catchUp();
    _ambientMode = name;
    _push();
    return player.SetAmbientModeResponse(ambient: _ambientStatus());
  }

  /// Where the player is on the network: nowhere. It has a name and no
  /// addresses and no port, which the app takes for a player it has nothing
  /// to try over Wi-Fi for, so it never tries.
  player.GetNetworkInfoResponse networkInfo() =>
      player.GetNetworkInfoResponse(hostname: 'demo-player');
}
