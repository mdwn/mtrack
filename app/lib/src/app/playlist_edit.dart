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
import 'package:flutter/foundation.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';
import 'failure.dart';

/// The player's own playlist of every song. It cannot be edited or deleted.
const String allSongsName = 'all_songs';

/// Said when a save finds the playlist changed on the player.
const String playlistChangedMessage =
    'This playlist was changed on the player while you were editing.';

/// Said when a save is refused because the player is locked.
const String lockedToSaveMessage = 'The player is locked. Unlock it to save.';

/// Said when a new playlist's name is taken.
const String nameTakenMessage = 'A playlist with that name already exists.';

/// Why [name], already trimmed, cannot be a new playlist's name; null when it
/// can. The player has the final say.
String? playlistNameProblem(String name) {
  if (name.isEmpty) {
    return 'Give the playlist a name.';
  }
  if (name.contains('/') || name.contains(r'\')) {
    return r'A name cannot contain / or \.';
  }
  if (name.toLowerCase() == allSongsName) {
    return '$allSongsName is the player\'s own playlist. Choose another name.';
  }
  return null;
}

/// The songs of [library] whose name has [query] in it, any case. An empty
/// query keeps all of them.
List<player.PlaylistEntry> filterSongs(
  List<player.PlaylistEntry> library,
  String query,
) {
  final needle = query.trim().toLowerCase();
  if (needle.isEmpty) {
    return library;
  }
  return [
    for (final song in library)
      if (song.songName.toLowerCase().contains(needle)) song,
  ];
}

/// One row of the list being edited.
class EditRow {
  /// A row. [id] tells it from another row of the same song, [originalIndex]
  /// is where the player had it when editing began, or null for a song added
  /// since. [settings] are its own (fades, auto play, ambient light); not to
  /// be changed in place.
  EditRow(
    this.id,
    this.name,
    this.duration,
    this.originalIndex, [
    player.EntrySettings? settings,
  ]) : settings = settings ?? player.EntrySettings();

  /// Unique within one edit.
  final int id;

  /// The song's name.
  final String name;

  /// Its length, when known.
  final Duration? duration;

  /// Its place in the list as loaded.
  final int? originalIndex;

  /// The entry's settings.
  final player.EntrySettings settings;

  /// This row with [settings] in place of its own.
  EditRow withSettings(player.EntrySettings settings) =>
      EditRow(id, name, duration, originalIndex, settings);
}

/// What stands in the way of a save, other than a plain failure.
enum EditProblem {
  /// Nothing.
  none,

  /// The playlist changed on the player since it was read.
  changed,

  /// The player is locked.
  locked,

  /// A new playlist's name is taken.
  exists,

  /// The player refused what was sent, saying why: a setting it cannot
  /// read, say. See [PlaylistEdit.refusal].
  refused,
}

/// A playlist being edited on the phone: the working list, what the player
/// had when editing began, and the save that sends one to the other.
///
/// Nothing reaches the player until [save]. The edit lives in the session, so
/// a dropped link does not lose it.
class PlaylistEdit extends ChangeNotifier {
  /// An edit of the playlist [name], which held [entries] and the ambient
  /// light [ambient] when read, at [revision]; or, with [isNew], of a
  /// playlist that does not exist yet. [supportsSettings] says whether the
  /// player reads and sets entries' settings: one from before does not.
  PlaylistEdit({
    required this.name,
    required List<player.PlaylistEntry> entries,
    player.AmbientSetting? ambient,
    String revision = '',
    this.supportsSettings = false,
    required this._client,
    required this._isReady,
    required this._locked,
    required this._notice,
    required this._lostIfUnanswered,
    required this._saved,
    this.isNew = false,
  }) {
    _load(entries, ambient, revision);
  }

  /// The playlist's name. Changeable only while [isNew].
  String name;

  /// Whether the playlist is yet to be made.
  final bool isNew;

  /// Whether the player reads and sets each entry's settings and the
  /// playlist's ambient light. False for a player from before, which edits
  /// songs and their order only.
  final bool supportsSettings;

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final bool? Function() _locked;
  final String Function(String) _notice;
  final bool Function(PlayerClient, Object) _lostIfUnanswered;
  final void Function(PlaylistEdit, player.GetPlaylistResponse) _saved;

  List<EditRow> _rows = [];
  List<String> _expected = [];
  List<player.EntrySettings> _expectedSettings = [];
  player.AmbientSetting? _ambient;
  player.AmbientSetting? _expectedAmbient;
  String _revision = '';
  bool _settingsTouched = false;
  bool _ambientTouched = false;
  String? _refusal;
  List<String>? _lightShows;
  bool _loadingLightShows = false;
  Map<String, player.SongPalette>? _songPalettes;
  bool _loadingSongPalettes = false;
  int _nextId = 0;
  bool _saving = false;
  EditProblem _problem = EditProblem.none;
  bool _disposed = false;

  List<player.PlaylistEntry>? _library;
  bool _loadingLibrary = false;
  bool _libraryFailed = false;

  void _load(
    List<player.PlaylistEntry> entries,
    player.AmbientSetting? ambient,
    String revision,
  ) {
    _expected = [for (final e in entries) e.songName];
    _expectedSettings = [
      for (final e in entries)
        e.hasSettings() ? e.settings : player.EntrySettings(),
    ];
    _rows = [
      for (var i = 0; i < entries.length; i++)
        EditRow(
          _nextId++,
          entries[i].songName,
          entries[i].hasDuration() ? entries[i].duration.toDart() : null,
          i,
          _expectedSettings[i],
        ),
    ];
    _ambient = ambient;
    _expectedAmbient = ambient;
    _revision = revision;
    _settingsTouched = false;
    _ambientTouched = false;
  }

  /// The rows, in the order they will be saved.
  List<EditRow> get rows => _rows;

  /// The song names as the player had them when editing began: what a save
  /// expects to find.
  List<String> get expected => _expected;

  /// What is in the way of a save.
  EditProblem get problem => _locked() == true && _problem == EditProblem.none
      ? EditProblem.locked
      : _problem;

  /// True while a save is out.
  bool get saving => _saving;

  /// The total of the songs' lengths that are known.
  Duration get total => _rows.fold(
    Duration.zero,
    (sum, row) => sum + (row.duration ?? Duration.zero),
  );

  /// The playlist's own ambient light as it will be saved: null for the
  /// profile's.
  player.AmbientSetting? get ambient => _ambient;

  /// The revision of the playlist as read, which a save sends back.
  String get revision => _revision;

  /// What the player said when it refused the last save, with
  /// [EditProblem.refused].
  String? get refusal => _problem == EditProblem.refused ? _refusal : null;

  /// Whether the list or its settings are not the ones loaded.
  bool get dirty {
    if (!listEquals([for (final r in _rows) r.name], _expected)) {
      return true;
    }
    if (!supportsSettings) {
      return false;
    }
    for (var i = 0; i < _rows.length; i++) {
      if (_rows[i].settings != _expectedSettings[i]) {
        return true;
      }
    }
    return _ambient != _expectedAmbient;
  }

  /// Whether leaving would lose something: changes, or a new playlist with
  /// songs in it.
  bool get hasChanges => isNew ? _rows.isNotEmpty : dirty;

  /// Whether Save may be pressed. A new playlist may be saved empty.
  bool get canSave =>
      !_saving &&
      _isReady() &&
      problem != EditProblem.changed &&
      problem != EditProblem.locked &&
      (isNew || dirty);

  /// Whether [song] is somewhere in the list.
  bool contains(String song) => _rows.any((r) => r.name == song);

  // Changing the list.

  /// Removes the row at [index], and only that row.
  void removeAt(int index) {
    if (index < 0 || index >= _rows.length) {
      return;
    }
    _rows = List.of(_rows)..removeAt(index);
    _edited();
  }

  /// Moves the row at [oldIndex] so it sits at [newIndex] in the list that
  /// is left once it is taken out, as a reorderable list reports a drop.
  void move(int oldIndex, int newIndex) {
    if (oldIndex < 0 || oldIndex >= _rows.length) {
      return;
    }
    final to = newIndex.clamp(0, _rows.length - 1);
    if (to == oldIndex) {
      return;
    }
    final next = List.of(_rows);
    next.insert(to, next.removeAt(oldIndex));
    _rows = next;
    _edited();
  }

  /// Appends the songs named [songs], in that order. A song already in the
  /// list is added again: the player allows it.
  void addSongs(List<String> songs) {
    final known = {
      for (final s in _library ?? <player.PlaylistEntry>[]) s.songName: s,
    };
    final next = List.of(_rows);
    for (final name in songs) {
      final entry = known[name];
      next.add(
        EditRow(
          _nextId++,
          name,
          entry != null && entry.hasDuration() ? entry.duration.toDart() : null,
          null,
        ),
      );
    }
    _rows = next;
    _edited();
  }

  /// Sets the settings of the row [id] (see [EditRow.id]). Nothing reaches
  /// the player until [save].
  void setSettings(int id, player.EntrySettings settings) {
    final at = _rows.indexWhere((r) => r.id == id);
    if (at < 0 || !supportsSettings) {
      return;
    }
    _rows = List.of(_rows)..[at] = _rows[at].withSettings(settings);
    _settingsTouched = true;
    _edited();
  }

  /// Sets the playlist's own ambient light; null for the profile's.
  void setAmbient(player.AmbientSetting? ambient) {
    if (!supportsSettings) {
      return;
    }
    _ambient = ambient;
    _ambientTouched = true;
    _edited();
  }

  /// The `.light` files the player has, for an ambient show; null until
  /// fetched, or when they could not be.
  List<String>? get lightShows => _lightShows;

  /// Fetches [lightShows], once. A failure leaves them null: a show can
  /// still be typed.
  Future<void> loadLightShows() async {
    final client = _client();
    if (_lightShows != null ||
        _loadingLightShows ||
        client == null ||
        !_isReady()) {
      return;
    }
    _loadingLightShows = true;
    try {
      final response = await client.listLightShows();
      if (!_disposed) {
        _lightShows = List.unmodifiable(response.paths);
      }
    } on Object catch (e) {
      if (!_disposed) {
        _lostIfUnanswered(client, e);
      }
    } finally {
      _loadingLightShows = false;
      _changed();
    }
  }

  /// Each loaded song's lighting palette by the song's name, for the
  /// entries' colours; null until fetched, or when it could not be. A song
  /// with no palette is not in it.
  Map<String, player.SongPalette>? get songPalettes => _songPalettes;

  /// Fetches [songPalettes], once. A failure leaves them null, and the
  /// entries' sheets without their colours. A player from before palettes
  /// answers none.
  Future<void> loadSongPalettes() async {
    final client = _client();
    if (_songPalettes != null ||
        _loadingSongPalettes ||
        client == null ||
        !_isReady()) {
      return;
    }
    _loadingSongPalettes = true;
    try {
      final response = await client.getPlaylistWithLibrary(allSongsName);
      if (!_disposed) {
        _songPalettes = Map.unmodifiable(response.songPalettes);
      }
    } on Object catch (e) {
      if (!_disposed) {
        _lostIfUnanswered(client, e);
      }
    } finally {
      _loadingSongPalettes = false;
      _changed();
    }
  }

  /// Gives a new playlist another name. Returns what is wrong with it, or
  /// null when it was taken.
  String? rename(String raw) {
    final trimmed = raw.trim();
    final bad = playlistNameProblem(trimmed);
    if (bad != null) {
      return bad;
    }
    name = trimmed;
    if (_problem == EditProblem.exists) {
      _problem = EditProblem.none;
    }
    _changed();
    return null;
  }

  // The library.

  /// Every song on the player, once fetched.
  List<player.PlaylistEntry>? get library => _library;

  /// True while the library is being fetched.
  bool get loadingLibrary => _loadingLibrary;

  /// True when the last fetch of the library failed.
  bool get libraryFailed => _libraryFailed;

  /// Fetches the library, for the song picker. Returns the failure in words,
  /// or null.
  Future<String?> loadLibrary() async {
    final client = _client();
    if (client == null || !_isReady()) {
      return _notice(notConnectedMessage);
    }
    if (_loadingLibrary) {
      return null;
    }
    _loadingLibrary = true;
    _libraryFailed = false;
    _changed();
    try {
      final response = await client.getPlaylist(allSongsName);
      if (_disposed) {
        return null;
      }
      _library = response.entries;
      return null;
    } on Object catch (e) {
      if (_disposed) {
        return null;
      }
      _libraryFailed = true;
      _lostIfUnanswered(client, e);
      return _notice(describeFailure(e));
    } finally {
      if (!_disposed) {
        _loadingLibrary = false;
        _changed();
      }
    }
  }

  // Saving.

  /// Sends the list to the player. True when it was saved.
  Future<bool> save() {
    if (!canSave) {
      return Future<bool>.value(false);
    }
    return _submit(refetch: false);
  }

  /// After a [EditProblem.changed]: fetches what the player has now and saves
  /// over it.
  Future<bool> saveAnyway() {
    if (_saving || _problem != EditProblem.changed) {
      return Future<bool>.value(false);
    }
    return _submit(refetch: true);
  }

  /// After a [EditProblem.changed]: drops the edits and loads what the player
  /// has now. True when it did.
  Future<bool> discardAndReload() async {
    final client = _client();
    if (_saving || client == null || !_isReady()) {
      return false;
    }
    _saving = true;
    _changed();
    try {
      final current = await client.getPlaylist(name);
      if (_disposed) {
        return false;
      }
      _load(
        current.entries,
        current.hasAmbient() ? current.ambient : null,
        current.revision,
      );
      _problem = EditProblem.none;
      return true;
    } on Object catch (e) {
      if (!_disposed) {
        _lostIfUnanswered(client, e);
        _notice(describeFailure(e));
      }
      return false;
    } finally {
      if (!_disposed) {
        _saving = false;
        _changed();
      }
    }
  }

  Future<bool> _submit({required bool refetch}) async {
    final client = _client();
    if (client == null || !_isReady()) {
      _notice(notConnectedMessage);
      return false;
    }
    _saving = true;
    _problem = EditProblem.none;
    _changed();
    try {
      if (refetch) {
        final current = await client.getPlaylist(name);
        if (_disposed) {
          return false;
        }
        // Saved over what is there now: the songs and the settings this
        // edit did not touch are the player's, and the check is against it.
        _expected = [for (final e in current.entries) e.songName];
        _revision = current.revision;
      }
      final response = await client.savePlaylist(
        name,
        [for (final r in _rows) r.name],
        expected: isNew ? null : _expected,
        expectedRevision: _revision.isEmpty ? null : _revision,
        create: isNew,
        // Only what was changed is sent: what was not stays as the player
        // has it, written as it was.
        settings: supportsSettings && _settingsTouched
            ? [for (final r in _rows) r.settings]
            : null,
        setAmbient: supportsSettings && _ambientTouched,
        ambient: _ambient,
      );
      if (_disposed) {
        return false;
      }
      _saved(this, response);
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
          said.contains('playlist has changed')) {
        _problem = EditProblem.changed;
        return;
      }
      if (e.code == GrpcCode.alreadyExists) {
        _problem = EditProblem.exists;
        return;
      }
      // The player's own words say what it would not take: a length it
      // cannot read, a fade longer than its song.
      if (e.code == GrpcCode.invalidArgument && e.message.isNotEmpty) {
        _problem = EditProblem.refused;
        _refusal = e.message;
        return;
      }
      if ((e.code == GrpcCode.failedPrecondition && said.contains('locked')) ||
          (e.isPermissionDenied && _locked() == true)) {
        _problem = EditProblem.locked;
        return;
      }
    }
    _notice(describeFailure(e));
  }

  /// The status moved on: a lock that was found is no longer one if the
  /// player says it is unlocked.
  void noteStatus() {
    if (_problem == EditProblem.locked && _locked() != true) {
      _problem = EditProblem.none;
    }
    _changed();
  }

  /// The user changed something: a refusal was about what was sent before.
  void _edited() {
    if (_problem == EditProblem.refused) {
      _problem = EditProblem.none;
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
