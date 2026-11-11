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

/// The song library and a song's settings: the list the Songs screen
/// shows, and the edit a song's page stages and saves.
///
/// Lengths stay text, as in the playlist settings: the player reads them
/// and says what is wrong with one when the song is saved.
library;

import 'package:flutter/foundation.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';
import 'failure.dart';

/// Said when a save finds the song changed on the player.
const String songChangedMessage =
    'This song was changed on the player while you were editing.';

/// Said on the page of the song that is playing: what a save does to it,
/// as the player does it (the web UI's save does the same).
const String playingSongNote =
    'This song is playing. A save is written at once and takes effect the '
    'next time it plays: the pass playing keeps its tracks, loop and '
    'samples. Looping is the exception: turning it on or off changes at '
    'once whether Play and Next break out of the song.';

/// Said on a song page while the player is locked.
const String lockedSongNote =
    'The player is locked. Unlock it to change this song.';

/// Said on the Songs screen against a player without the song library.
const String olderPlayerSongsNote =
    'This player runs an older mtrack, so its songs are not shown here. '
    'Update mtrack on the player to see and change them from the phone.';

/// The release behaviours a sample may have, as the player names them, with
/// the web UI's words for them.
const List<(String, String)> releaseChoices = [
  ('play_to_completion', 'Play to Completion'),
  ('stop', 'Stop'),
  ('fade', 'Fade'),
];

/// The retrigger behaviours a sample may have, with the web UI's words.
const List<(String, String)> retriggerChoices = [
  ('cut', 'Cut'),
  ('polyphonic', 'Polyphonic'),
];

/// The fade time a sample has when its file sets none.
const int defaultSampleFadeMs = 50;

/// The songs of [library] whose name has [query] in it, any case. An empty
/// query keeps all of them.
List<player.SongListing> filterListings(
  List<player.SongListing> library,
  String query,
) {
  final needle = query.trim().toLowerCase();
  if (needle.isEmpty) {
    return library;
  }
  return [
    for (final song in library)
      if (song.name.toLowerCase().contains(needle)) song,
  ];
}

/// What a song has, in a few words for its row: empty for nothing.
String listingFlags(player.SongListing song) {
  if (song.hasFailure()) {
    return 'failed to load';
  }
  return [
    if (song.loops) 'loops',
    if (song.hasLighting) 'lights',
    if (song.hasMidi) 'MIDI',
    if (song.hasSamples) 'samples',
  ].join(' · ');
}

/// A track as its row says it: its file, and the channel when it names one.
String trackSummary(player.SongTrack track) => track.hasFileChannel()
    ? '${track.file} · channel ${track.fileChannel}'
    : track.file;

/// The web UI's word for [value] among [choices], or [value] itself.
String choiceLabel(List<(String, String)> choices, String value) {
  for (final (key, label) in choices) {
    if (key == value) {
      return label;
    }
  }
  return value;
}

/// The release behaviour [sample] has: its own, or the default.
String releaseOf(player.SongSample sample) => sample.releaseBehavior.isEmpty
    ? releaseChoices.first.$1
    : sample.releaseBehavior;

/// The retrigger behaviour [sample] has: its own, or the default.
String retriggerOf(player.SongSample sample) =>
    sample.retrigger.isEmpty ? retriggerChoices.first.$1 : sample.retrigger;

/// What a sample does, in short, for its row.
String sampleSummary(player.SongSample sample) {
  final output = sample.hasOutputTrack()
      ? 'to ${sample.outputTrack}'
      : sample.outputChannels.isEmpty
      ? null
      : 'to ${sample.outputChannels.length == 1 ? 'channel' : 'channels'} '
            '${sample.outputChannels.join(', ')}';
  return [
    if (sample.hasFile()) sample.file else 'no file',
    ?output,
    if (releaseOf(sample) != releaseChoices.first.$1)
      'release: ${choiceLabel(releaseChoices, releaseOf(sample)).toLowerCase()}',
    if (retriggerOf(sample) != retriggerChoices.first.$1)
      choiceLabel(retriggerChoices, retriggerOf(sample)).toLowerCase(),
  ].join(' · ');
}

/// How a sample answers velocity, as the web UI words its modes; null for
/// the plain case (ignored, at the default velocity).
String? velocitySummary(player.SongSample sample) {
  if (!sample.hasVelocity()) {
    return null;
  }
  final velocity = sample.velocity;
  switch (velocity.mode) {
    case 'layers':
      final layers = velocity.layers == 1
          ? '1 layer'
          : '${velocity.layers} layers';
      return velocity.scale
          ? 'Velocity: $layers, scaled by velocity'
          : 'Velocity: $layers';
    case 'scale':
      return 'Velocity: scale';
    default:
      return velocity.hasDefaultVelocity()
          ? 'Velocity: ignore, at ${velocity.defaultVelocity}'
          : null;
  }
}

/// Output channels as typed (`1, 2`), or null when the text is not a list
/// of channel numbers from 1. Empty is none.
List<int>? parseChannels(String text) {
  final parts = text
      .split(RegExp(r'[,\s]+'))
      .where((part) => part.isNotEmpty)
      .toList();
  final channels = <int>[];
  for (final part in parts) {
    final value = int.tryParse(part);
    if (value == null || value < 1) {
      return null;
    }
    channels.add(value);
  }
  return channels;
}

/// What a song has besides what the phone changes, a line each.
List<String> overviewLines(player.SongOverview overview) => [
  if (overview.hasTempo()) 'Tempo: ${overview.tempo}',
  if (overview.sections.isNotEmpty)
    overview.sections.length == 1
        ? 'Section: ${overview.sections.single}'
        : '${overview.sections.length} sections: '
              '${overview.sections.join(', ')}',
  if (overview.metronome) 'Metronome',
  if (overview.pilotHints > 0)
    overview.pilotHints == 1
        ? '1 pilot hint'
        : '${overview.pilotHints} pilot hints',
  if (overview.hasMidiFile()) 'MIDI file: ${overview.midiFile}',
  for (final show in overview.lighting) 'Light show: $show',
  if (overview.midiLightShows > 0)
    overview.midiLightShows == 1
        ? '1 MIDI DMX light show'
        : '${overview.midiLightShows} MIDI DMX light shows',
  if (overview.notificationAudio) 'Notification audio of its own',
  if (overview.songSelectEvent) 'A song select MIDI event',
];

/// The player's songs, for the Songs screen: fetched whole, page by page,
/// when asked for, and kept until the player changes.
class SongLibrary extends ChangeNotifier {
  /// A library fetched through the session's client.
  SongLibrary({
    required this._client,
    required this._isReady,
    required this._lostIfUnanswered,
  });

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final bool Function(PlayerClient, Object) _lostIfUnanswered;

  List<player.SongListing>? _songs;
  bool _loading = false;
  String? _failure;
  bool _tooOld = false;
  bool _disposed = false;

  /// The songs, once fetched.
  List<player.SongListing>? get songs => _songs;

  /// True while they are being fetched.
  bool get loading => _loading;

  /// Why the last fetch failed, or null.
  String? get failure => _failure;

  /// True once the player has refused the library as a call it does not
  /// have: it is from before songs could be read.
  bool get tooOld => _tooOld;

  /// Fetches the library.
  Future<void> load() async {
    final client = _client();
    if (_loading) {
      return;
    }
    if (client == null || !_isReady()) {
      _failure = notConnectedMessage;
      _changed();
      return;
    }
    _loading = true;
    _failure = null;
    _changed();
    try {
      final songs = await client.listAllSongs();
      if (!_disposed) {
        _songs = List.unmodifiable(songs);
        _tooOld = false;
      }
    } on Object catch (e) {
      if (_disposed) {
        return;
      }
      // A player from before refuses the call as one it does not allow.
      if (e is PlayerException && e.isStreamTooOld) {
        _tooOld = true;
      } else {
        _lostIfUnanswered(client, e);
        _failure = describeFailure(e);
      }
    } finally {
      _loading = false;
      _changed();
    }
  }

  /// A song saved from its page: its row says what it now has.
  void noteSaved(player.GetSongResponse song) {
    final songs = _songs;
    if (songs == null) {
      return;
    }
    _songs = List.unmodifiable([
      for (final row in songs)
        if (row.name == song.name) _listingOf(row, song) else row,
    ]);
    _changed();
  }

  static player.SongListing _listingOf(
    player.SongListing row,
    player.GetSongResponse song,
  ) {
    final next = row.deepCopy();
    if (song.hasLooping()) {
      next.loops = song.looping.loopPlayback;
    }
    if (song.hasSamples()) {
      next.hasSamples = song.samples.samples.isNotEmpty;
    }
    if (song.hasFailure()) {
      next.failure = song.failure;
      next.clearDuration();
    } else {
      next.clearFailure();
      if (song.hasDuration()) {
        next.duration = song.duration;
      }
    }
    return next;
  }

  /// Forgets the songs, for another player.
  void clear() {
    _songs = null;
    _failure = null;
    _tooOld = false;
    _changed();
  }

  void _changed() {
    if (!_disposed) {
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// What stands in the way of saving a song, other than a plain failure.
enum SongProblem {
  /// Nothing.
  none,

  /// The song changed on the player since it was read.
  changed,

  /// The player is locked.
  locked,

  /// The player refused what was sent, saying why: see
  /// [SongEdit.refusal].
  refused,
}

/// A song's settings being changed on its page: the song as read, the
/// looping, tracks and samples as they will be saved, and the save.
///
/// Nothing reaches the player until [save], and then only the groups that
/// were changed: a group left alone stays as the player has it.
class SongEdit extends ChangeNotifier {
  /// An edit of the song [name], fetched with [load].
  SongEdit({
    required this.name,
    required this._client,
    required this._isReady,
    required this._locked,
    required this._notice,
    required this._lostIfUnanswered,
    this._saved,
  });

  /// The song's name.
  final String name;

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final bool? Function() _locked;
  final String Function(String) _notice;
  final bool Function(PlayerClient, Object) _lostIfUnanswered;
  final void Function(player.GetSongResponse)? _saved;

  player.GetSongResponse? _song;
  player.SongLooping _looping = player.SongLooping();
  List<player.SongTrack> _tracks = const [];
  List<player.SongSample> _samples = const [];
  bool _loading = false;
  String? _loadFailure;
  bool _saving = false;
  SongProblem _problem = SongProblem.none;
  bool _lockHeard = false;
  String? _refusal;
  bool _disposed = false;

  /// The song as last read or saved; null until it is.
  player.GetSongResponse? get song => _song;

  /// True while the song is being fetched.
  bool get loading => _loading;

  /// Why the last fetch failed, or null.
  String? get loadFailure => _loadFailure;

  /// True while a save is out.
  bool get saving => _saving;

  /// The looping as it will be saved.
  player.SongLooping get looping => _looping;

  /// The tracks as they will be saved.
  List<player.SongTrack> get tracks => _tracks;

  /// The song's own samples as they will be saved.
  List<player.SongSample> get samples => _samples;

  /// What is in the way of a save. A lock found by a save is gone once the
  /// player says it is unlocked.
  SongProblem get problem {
    if (_problem == SongProblem.locked) {
      if (_locked() == true) {
        _lockHeard = true;
      } else if (_lockHeard) {
        // Unlocked since the status said it was locked.
        return SongProblem.none;
      }
      return SongProblem.locked;
    }
    if (_problem == SongProblem.none && _locked() == true && dirty) {
      return SongProblem.locked;
    }
    return _problem;
  }

  /// What the player said when it refused the last save.
  String? get refusal => _problem == SongProblem.refused ? _refusal : null;

  /// Why the song cannot be changed from the phone at all, in the player's
  /// words: it has no file of its own, or its file does not read. Null
  /// when it can.
  String? get cannotEdit {
    final song = _song;
    return song != null && song.hasReadOnly() ? song.readOnly : null;
  }

  /// Whether the page offers changes: the song is read, can be changed,
  /// and the player is not locked.
  bool get editable =>
      _song != null && cannotEdit == null && _locked() != true && !_saving;

  /// Whether the looping is not as read.
  bool get loopingDirty {
    final song = _song;
    return song != null && song.hasLooping() && _looping != song.looping;
  }

  /// Whether the tracks are not as read.
  bool get tracksDirty {
    final song = _song;
    return song != null && !listEquals(_tracks, song.tracks.tracks);
  }

  /// Whether the samples are not as read.
  bool get samplesDirty {
    final song = _song;
    return song != null && !listEquals(_samples, song.samples.samples);
  }

  /// Whether anything is not as read.
  bool get dirty => loopingDirty || tracksDirty || samplesDirty;

  /// Whether Save may be pressed.
  bool get canSave =>
      !_saving &&
      _isReady() &&
      cannotEdit == null &&
      dirty &&
      problem != SongProblem.changed &&
      // A save the lock refused may be tried again once the player says
      // it is unlocked.
      _locked() != true;

  /// The track names a crossfade may cut: the tracks as they will be saved.
  List<String> get trackNames => [for (final t in _tracks) t.name];

  /// The audio files in the song's folder, for a track or a sample.
  List<player.SongFile> get files => _song?.files ?? const [];

  // Reading.

  /// Fetches the song, dropping anything staged.
  Future<void> load() async {
    final client = _client();
    if (_loading) {
      return;
    }
    if (client == null || !_isReady()) {
      _loadFailure = notConnectedMessage;
      _changed();
      return;
    }
    _loading = true;
    _loadFailure = null;
    _changed();
    try {
      final song = await client.getSong(name);
      if (!_disposed) {
        _take(song);
        _problem = SongProblem.none;
      }
    } on Object catch (e) {
      if (!_disposed) {
        _lostIfUnanswered(client, e);
        _loadFailure = describeFailure(e);
      }
    } finally {
      _loading = false;
      _changed();
    }
  }

  void _take(player.GetSongResponse song) {
    _song = song;
    _looping = song.hasLooping() ? song.looping : player.SongLooping();
    _tracks = List.unmodifiable(song.tracks.tracks);
    _samples = List.unmodifiable(song.samples.samples);
  }

  // Changing.

  /// Sets whether the song loops.
  void setLoopPlayback(bool on) =>
      _setLooping(_looping.deepCopy()..loopPlayback = on);

  /// Sets the loop crossfade, as typed; empty for none.
  void setCrossfade(String text) =>
      _setLooping(_looping.deepCopy()..crossfade = text.trim());

  /// Sets the tracks that switch over at the crossfade's midpoint.
  void setCut(List<String> names) {
    final next = _looping.deepCopy();
    next.cut
      ..clear()
      ..addAll(names);
    _setLooping(next);
  }

  void _setLooping(player.SongLooping looping) {
    _looping = looping;
    _edited();
  }

  /// Puts [track] in place of the track at [index]. A renamed track is
  /// renamed among the tracks the crossfade cuts too.
  void setTrack(int index, player.SongTrack track) {
    if (index < 0 || index >= _tracks.length) {
      return;
    }
    final old = _tracks[index].name;
    _tracks = List.unmodifiable(List.of(_tracks)..[index] = track);
    if (old != track.name && _looping.cut.contains(old)) {
      setCut([
        for (final name in _looping.cut) name == old ? track.name : name,
      ]);
      return;
    }
    _edited();
  }

  /// Adds [track] at the end.
  void addTrack(player.SongTrack track) {
    _tracks = List.unmodifiable([..._tracks, track]);
    _edited();
  }

  /// Removes the track at [index], and its name from the tracks the
  /// crossfade cuts unless another track has it.
  void removeTrack(int index) {
    if (index < 0 || index >= _tracks.length) {
      return;
    }
    final gone = _tracks[index].name;
    _tracks = List.unmodifiable(List.of(_tracks)..removeAt(index));
    if (_looping.cut.contains(gone) && !trackNames.contains(gone)) {
      setCut([
        for (final name in _looping.cut)
          if (name != gone) name,
      ]);
      return;
    }
    _edited();
  }

  /// Puts [sample] in place of the sample at [index]. Its `originalName`
  /// stays the one read, so a rename keeps what the phone does not show.
  void setSample(int index, player.SongSample sample) {
    if (index < 0 || index >= _samples.length) {
      return;
    }
    _samples = List.unmodifiable(List.of(_samples)..[index] = sample);
    _edited();
  }

  /// Adds a new sample, [sample], at the end.
  void addSample(player.SongSample sample) {
    _samples = List.unmodifiable([
      ..._samples,
      sample.deepCopy()..clearOriginalName(),
    ]);
    _edited();
  }

  /// Removes the sample at [index].
  void removeSample(int index) {
    if (index < 0 || index >= _samples.length) {
      return;
    }
    _samples = List.unmodifiable(List.of(_samples)..removeAt(index));
    _edited();
  }

  /// Whether [name] is taken by a sample other than the one at [index].
  bool sampleNameTaken(String name, {int? index}) {
    for (var i = 0; i < _samples.length; i++) {
      if (i != index && _samples[i].name == name) {
        return true;
      }
    }
    return false;
  }

  /// Puts everything back as read.
  void revert() {
    final song = _song;
    if (song != null) {
      _take(song);
      _edited();
    }
  }

  // Saving.

  /// Sends what was changed. True when it was saved.
  Future<bool> save() {
    if (!canSave) {
      return Future<bool>.value(false);
    }
    return _submit(refetch: false);
  }

  /// After a [SongProblem.changed]: saves what was changed here over what
  /// the player has now. The groups not changed here stay the player's.
  Future<bool> saveAnyway() {
    if (_saving || _problem != SongProblem.changed) {
      return Future<bool>.value(false);
    }
    return _submit(refetch: true);
  }

  /// After a [SongProblem.changed]: drops the changes and reads the song
  /// again.
  Future<void> discardAndReload() => load();

  Future<bool> _submit({required bool refetch}) async {
    final client = _client();
    final read = _song;
    if (client == null || !_isReady() || read == null) {
      _notice(notConnectedMessage);
      return false;
    }
    final looping = loopingDirty ? _looping : null;
    final tracks = tracksDirty ? player.SongTracks(tracks: _tracks) : null;
    final samples = samplesDirty ? player.SongSamples(samples: _samples) : null;
    _saving = true;
    _problem = SongProblem.none;
    _changed();
    try {
      var revision = read.revision;
      if (refetch) {
        final current = await client.getSong(name);
        if (_disposed) {
          return false;
        }
        revision = current.revision;
      }
      final saved = await client.updateSongSettings(
        name,
        expectedRevision: revision,
        looping: looping,
        tracks: tracks,
        samples: samples,
      );
      if (_disposed) {
        return false;
      }
      _take(saved);
      _saved?.call(saved);
      _notice(
        saved.playing
            ? 'Saved $name. It takes effect the next time it plays.'
            : 'Saved $name.',
      );
      return true;
    } on Object catch (e) {
      if (!_disposed) {
        _failed(client, e);
      }
      return false;
    } finally {
      if (!_disposed) {
        _saving = false;
        _changed();
      }
    }
  }

  void _failed(PlayerClient client, Object e) {
    if (_lostIfUnanswered(client, e)) {
      _notice(describeFailure(e));
      return;
    }
    if (e is PlayerException) {
      final said = e.message.toLowerCase();
      if (e.code == GrpcCode.failedPrecondition &&
          said.contains('song has changed')) {
        _problem = SongProblem.changed;
        return;
      }
      if ((e.code == GrpcCode.failedPrecondition && said.contains('locked')) ||
          (e.isPermissionDenied && _locked() == true)) {
        _problem = SongProblem.locked;
        _lockHeard = _locked() == true;
        return;
      }
      // The player's own words: a crossfade it cannot read, a channel the
      // file does not have, a file not in the song's folder.
      if ((e.code == GrpcCode.invalidArgument ||
              e.code == GrpcCode.failedPrecondition) &&
          e.message.isNotEmpty) {
        _problem = SongProblem.refused;
        _refusal = e.message;
        return;
      }
    }
    _notice(describeFailure(e));
  }

  /// The user changed something: a refusal was about what was sent before.
  void _edited() {
    if (_problem == SongProblem.refused) {
      _problem = SongProblem.none;
      _refusal = null;
    }
    _changed();
  }

  void _changed() {
    if (!_disposed) {
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
