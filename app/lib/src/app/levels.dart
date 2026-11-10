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
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';
import 'failure.dart';

/// The least time between two gain writes for one track while a fader moves.
const Duration levelSendInterval = Duration(milliseconds: 100);

/// The quietest gain: the player treats it as silence.
const double minGainDb = -60;

/// The loudest gain the player accepts.
const double maxGainDb = 12;

/// How near to 0 dB a fader may be let go for it to land on 0 dB exactly.
const double snapDb = 1.0;

/// What to say when the player refuses `GetTrackGains` as not allowed: a
/// player from before levels were open to a phone.
const String levelsTooOldMessage =
    'This player is too old for levels. Update mtrack to use them.';

/// Where the list of tracks stands.
enum LevelsPhase {
  /// Not fetched yet.
  idle,

  /// The first fetch is out.
  loading,

  /// The tracks are known.
  ready,

  /// The first fetch failed.
  failed,

  /// The player does not allow levels from a phone.
  tooOld,
}

/// An output track as the screen shows it.
class TrackLevel {
  /// A track [name] at [gainDb], [muted] or not. [busy] while its mute is
  /// being set.
  const TrackLevel(this.name, this.gainDb, this.muted, {this.busy = false});

  /// The track's name.
  final String name;

  /// Its gain.
  final double gainDb;

  /// Whether it is muted.
  final bool muted;

  /// True while a mute request for it is out.
  final bool busy;
}

/// Rounds a gain to a tenth of a dB and keeps it within the player's range.
double clampGain(double db) =>
    ((db.clamp(minGainDb, maxGainDb)) * 10).round() / 10;

class _Track {
  _Track(this.name, this.gain, this.muted) : lastSent = gain;

  final String name;
  double gain;
  bool muted;

  /// The gain the player last had from us or from a fetch; a value equal to
  /// it is not sent again.
  double lastSent;

  /// The newest value not yet sent: older ones are never queued behind it.
  double? pending;
  bool dragging = false;
  bool inFlight = false;
  bool cooling = false;
  bool muteInFlight = false;

  /// A send failed during this drag; the rest of it is not sent.
  bool failed = false;

  /// Counts every change made here, so a fetch that began earlier can tell
  /// its answer is out of date for this track.
  int version = 0;
  Timer? timer;

  /// Whether the person's value is not yet the player's.
  bool get held => dragging || pending != null || inFlight || muteInFlight;
}

/// The Levels tab's state: a gain and a mute for each output track, kept in
/// step with the player.
///
/// A fader sends at most one write per track every [levelSendInterval], and
/// of the values that come between, only the newest is kept. A fetch never
/// overwrites a track the person is moving or has a value out for.
class Levels extends ChangeNotifier {
  /// Levels over the session's connection.
  Levels({
    required this._client,
    required this._isReady,
    required this._notice,
    required this._lostIfUnanswered,
    this.interval = levelSendInterval,
  });

  /// The least time between two gain writes for one track.
  final Duration interval;

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final String Function(String) _notice;
  final bool Function(PlayerClient, Object) _lostIfUnanswered;

  List<_Track> _tracks = [];
  LevelsPhase _phase = LevelsPhase.idle;
  bool _visible = false;
  bool _stale = true;
  bool _fetching = false;

  /// Set by a failed fetch, so the status arriving four times a second does
  /// not start another; cleared by a retry or by the tab being shown again.
  bool _blocked = false;
  int? _seenRevision;
  int? _fetchedRevision;
  int _generation = 0;
  bool _disposed = false;

  /// Where the list stands.
  LevelsPhase get phase => _phase;

  /// The tracks, in the order the player gave them.
  List<TrackLevel> get tracks => [
    for (final t in _tracks)
      TrackLevel(t.name, t.gain, t.muted, busy: t.muteInFlight),
  ];

  /// Tells it whether the tab is on show. Gains are fetched only then.
  void setVisible(bool visible) {
    if (visible == _visible) {
      return;
    }
    _visible = visible;
    if (visible) {
      _blocked = false;
    }
    check();
  }

  /// Notes the revision the status carries; null from a player that has none.
  void onStatus(int? revision) {
    _seenRevision = revision;
    check();
  }

  /// Fetches the gains if the tab is on show and they are out of date.
  void check() {
    if (_disposed || !_visible || _fetching || _blocked || !_isReady()) {
      return;
    }
    if (_tracks.any((t) => t.held)) {
      // Asked again when the last of them settles.
      return;
    }
    final due =
        _phase == LevelsPhase.idle ||
        _stale ||
        (_seenRevision != null && _seenRevision != _fetchedRevision);
    if (due) {
      unawaited(_fetch());
    }
  }

  /// Fetches the gains again after a failure.
  void retry() {
    _blocked = false;
    _stale = true;
    check();
  }

  Future<void> _fetch() async {
    final client = _client();
    if (client == null) {
      return;
    }
    final generation = _generation;
    final revision = _seenRevision;
    final versions = {for (final t in _tracks) t.name: t.version};
    _fetching = true;
    if (_phase != LevelsPhase.ready) {
      _phase = LevelsPhase.loading;
      _changed();
    }
    try {
      final response = await client.getTrackGains();
      if (generation != _generation || _disposed) {
        return;
      }
      _merge(response.gains.map((g) => (g.track, g.gainDb, g.muted)), versions);
      _fetchedRevision = revision;
      _stale = false;
      _phase = LevelsPhase.ready;
    } on Object catch (e) {
      if (generation != _generation || _disposed) {
        return;
      }
      if (_lostIfUnanswered(client, e)) {
        _notice(describeFailure(e));
        return;
      }
      _blocked = true;
      if (e is PlayerException && e.isPermissionDenied) {
        _phase = LevelsPhase.tooOld;
      } else {
        _notice(describeFailure(e));
        if (_phase != LevelsPhase.ready) {
          _phase = LevelsPhase.failed;
        }
      }
    } finally {
      if (generation == _generation && !_disposed) {
        _fetching = false;
        _changed();
        check();
      }
    }
  }

  void _merge(Iterable<(String, double, bool)> fetched, Map<String, int> at) {
    final byName = {for (final t in _tracks) t.name: t};
    final next = <_Track>[];
    for (final (name, gain, muted) in fetched) {
      final have = byName.remove(name);
      if (have == null) {
        next.add(_Track(name, gain, muted));
      } else {
        if (!have.held && have.version == at[name]) {
          have.gain = gain;
          have.lastSent = gain;
          have.muted = muted;
        }
        next.add(have);
      }
    }
    for (final gone in byName.values) {
      gone.timer?.cancel();
    }
    _tracks = next;
  }

  // The faders.

  _Track? _find(String name) {
    for (final t in _tracks) {
      if (t.name == name) {
        return t;
      }
    }
    return null;
  }

  /// A finger is down on [name]'s fader.
  void beginDrag(String name) {
    final t = _find(name);
    if (t == null) {
      return;
    }
    t.dragging = true;
    t.failed = false;
    _changed();
  }

  /// The fader of [name] is at [db] and still moving.
  void dragTo(String name, double db) {
    final t = _find(name);
    if (t == null) {
      return;
    }
    t.dragging = true;
    t.gain = clampGain(db);
    t.version++;
    if (!t.failed) {
      _want(t, t.gain);
    }
    _changed();
    _pump(t);
  }

  /// The fader of [name] was let go at [db]. Near 0 dB it lands on 0 dB,
  /// unless [snap] is false (a step from a screen reader).
  void endDrag(String name, double db, {bool snap = true}) {
    final t = _find(name);
    if (t == null) {
      return;
    }
    final value = snap && db.abs() <= snapDb ? 0.0 : clampGain(db);
    t.dragging = false;
    t.failed = false;
    t.gain = value;
    t.version++;
    _want(t, value);
    _changed();
    _pump(t);
    check();
  }

  void _want(_Track t, double value) {
    t.pending = value == t.lastSent ? null : value;
  }

  /// Sends the pending value if nothing is out for this track and its last
  /// write is at least [interval] old.
  void _pump(_Track t) {
    if (t.inFlight || t.cooling || t.pending == null) {
      return;
    }
    final client = _client();
    if (client == null || !_isReady()) {
      t.pending = null;
      return;
    }
    final value = t.pending!;
    t.pending = null;
    t.lastSent = value;
    t.inFlight = true;
    t.cooling = true;
    final generation = _generation;
    t.timer = Timer(interval, () {
      t.timer = null;
      t.cooling = false;
      if (generation == _generation && !_disposed) {
        _pump(t);
      }
    });
    unawaited(_send(client, t, value, generation));
  }

  Future<void> _send(
    PlayerClient client,
    _Track t,
    double value,
    int generation,
  ) async {
    try {
      final response = await client.setTrackGain(t.name, value);
      if (generation != _generation || _disposed) {
        return;
      }
      t.inFlight = false;
      t.lastSent = response.appliedGainDb;
      if (t.pending == null && !t.dragging && t.gain != t.lastSent) {
        // Shows what the player applied.
        t.gain = t.lastSent;
        t.version++;
      }
    } on Object catch (e) {
      if (generation != _generation || _disposed) {
        return;
      }
      t.inFlight = false;
      t.pending = null;
      t.failed = true;
      _notice(describeFailure(e));
      if (!_lostIfUnanswered(client, e)) {
        // What the player has is no longer known; ask when nothing is held.
        _stale = true;
      }
    }
    _changed();
    _pump(t);
    check();
  }

  // The mutes.

  /// Flips the mute of [name]. Returns the failure in words, or null.
  Future<String?> toggleMute(String name) async {
    final t = _find(name);
    if (t == null || t.muteInFlight) {
      return null;
    }
    final client = _client();
    if (client == null || !_isReady()) {
      return _notice(notConnectedMessage);
    }
    final generation = _generation;
    t.muteInFlight = true;
    _changed();
    try {
      final response = await client.setTrackMute(name, !t.muted);
      if (generation == _generation && !_disposed) {
        t.muted = response.muted;
        t.version++;
      }
      return null;
    } on Object catch (e) {
      if (generation != _generation || _disposed) {
        return null;
      }
      if (!_lostIfUnanswered(client, e)) {
        _stale = true;
      }
      return _notice(describeFailure(e));
    } finally {
      if (generation == _generation && !_disposed) {
        t.muteInFlight = false;
        _changed();
        check();
      }
    }
  }

  // The connection.

  /// The connection ended: nothing out will be answered, and what the player
  /// has may be different when it comes back (it may have restarted).
  void reset() {
    _generation++;
    for (final t in _tracks) {
      t.timer?.cancel();
      t.timer = null;
      t.cooling = false;
      t.inFlight = false;
      t.muteInFlight = false;
      t.dragging = false;
      t.pending = null;
    }
    _fetching = false;
    _blocked = false;
    _stale = true;
    _fetchedRevision = null;
    if (_phase != LevelsPhase.ready) {
      _phase = LevelsPhase.idle;
    }
    _changed();
  }

  /// Forgets the tracks, for a player that is let go.
  void clear() {
    reset();
    _tracks = [];
    _phase = LevelsPhase.idle;
    _seenRevision = null;
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
    for (final t in _tracks) {
      t.timer?.cancel();
    }
    super.dispose();
  }
}
