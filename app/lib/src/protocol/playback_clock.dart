import '../../gen/ble/v1/ble.pb.dart' as ble;

/// A reading of a monotonic clock, as time since some fixed start. Only
/// differences between readings are used.
typedef MonotonicClock = Duration Function();

/// The song position to show right now, from the player's status messages.
///
/// A status carries the elapsed time as of the moment it was sent and arrives
/// a few times a second at most. Between messages the position advances on the
/// local monotonic clock while the player is playing, snaps to each new
/// message, holds while it is stopped, and never passes the song's length.
class PlaybackClock {
  /// A clock reading time from [now], which must be monotonic: a wall clock
  /// that the phone adjusts mid-song would make the position jump.
  PlaybackClock(this._now);

  final MonotonicClock _now;

  bool _playing = false;
  Duration _elapsed = Duration.zero;
  Duration _duration = Duration.zero;
  Duration _receivedAt = Duration.zero;

  /// Takes the latest status. The position is the status's elapsed time as of
  /// this call.
  void update(ble.Status status) {
    _playing = status.playing;
    _elapsed = status.hasElapsed() ? status.elapsed.toDart() : Duration.zero;
    _duration = status.hasSongDuration()
        ? status.songDuration.toDart()
        : Duration.zero;
    _receivedAt = _now();
  }

  /// Forgets the last status, for a lost connection: the position is zero and
  /// holds.
  void reset() {
    _playing = false;
    _elapsed = Duration.zero;
    _duration = Duration.zero;
  }

  /// Freezes the position where it is now, for a lost connection: the last
  /// state stays on show without running on.
  void hold() {
    _elapsed = position;
    _playing = false;
  }

  /// Whether the last status said a song is playing.
  bool get playing => _playing;

  /// The song's length, or zero when the player has no song or has not said.
  Duration get duration => _duration;

  /// How far into the song the player is, as of now.
  Duration get position {
    var position = _elapsed;
    if (_playing) {
      final since = _now() - _receivedAt;
      if (!since.isNegative) {
        position += since;
      }
    }
    if (_duration > Duration.zero && position > _duration) {
      position = _duration;
    }
    return position.isNegative ? Duration.zero : position;
  }
}
