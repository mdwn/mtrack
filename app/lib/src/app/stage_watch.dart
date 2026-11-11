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
import 'dart:typed_data';

import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';

/// How long to wait before asking again for a stage stream the player ended
/// or refused for a reason that may pass.
const Duration stageRetryDelay = Duration(seconds: 3);

/// How often, while the stage stream is open, the link is checked when
/// nothing has been heard from the player since the last check.
const Duration stageProbeInterval = Duration(seconds: 5);

/// What a stage watch tells its listener.
sealed class StageEvent {
  const StageEvent();
}

/// A new stream is about to begin: forget the layout and the frames of the
/// one before. It comes before the first [StageBytes] of every stream the
/// watch opens, the first one included, so a listener that resets on it and
/// feeds everything else to the page is always right.
final class StageReset extends StageEvent {
  /// The start of a new stream.
  const StageReset();
}

/// One message of the stream: a serialized `player.v1.StageUpdate`, a layout
/// or a frame, exactly as the player sent it.
final class StageBytes extends StageEvent {
  /// A message holding [bytes].
  const StageBytes(this.bytes);

  /// The serialized `StageUpdate`.
  final Uint8List bytes;
}

class _Watcher {
  _Watcher(this.maxRate);

  final int maxRate;
  late final StreamController<StageEvent> controller;

  /// True once something was withheld from this listener because it had
  /// paused: what it holds is no longer the stream.
  bool behind = false;
}

/// Keeps the player's stage stream (`WatchStage`) open for as long as anything
/// listens to it and the link is up.
///
/// However many listeners there are, the player has one stream open. It is
/// opened when the first listener arrives and cancelled when the last one
/// leaves, opened again after a reconnect, and started over whenever a
/// listener joins, so that the newcomer is sent the layout; every listener
/// is told with a [StageReset] each time. Nothing is decoded here.
///
/// Nothing is queued for a listener either: each event is handed over as it
/// arrives. A listener that pauses is passed over, and when it resumes the
/// stream is started over, since a layout may have gone by. So a consumer
/// that cannot keep up coalesces for itself (keep the newest frame; never
/// drop a layout), and the watch holds no memory for it whatever it does.
class StageWatch {
  /// A watch that calls through [client] while [isReady], reports calls that
  /// got no answer to [lostIfUnanswered], and calls [changed] when
  /// [missing] changes.
  StageWatch({
    required this._client,
    required this._isReady,
    required this._lostIfUnanswered,
    required this._changed,
  });

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final bool Function(PlayerClient client, Object error) _lostIfUnanswered;
  final void Function() _changed;

  final List<_Watcher> _watchers = [];
  StreamSubscription<Uint8List>? _stream;

  /// The cancel of the stream before, until the player has let go of it.
  Future<void>? _closing;

  /// Moves whenever the stream is closed or opened, so what a stream that is
  /// no longer the one says is ignored.
  int _generation = 0;

  Timer? _retry;
  Timer? _probe;
  bool _heard = false;
  bool _probing = false;
  bool _missing = false;
  bool _disposed = false;

  /// True when the connected player is too old for the stage: it has no
  /// `stream` characteristic, or it refused `WatchStage` as a method it does
  /// not allow. Known from the moment of connecting in the first case, and
  /// from the first time the stage is listened to in the second.
  bool get missing => _missing;

  /// True while a stream is open at the player, or being asked for.
  bool get isOpen => _stream != null;

  /// The stage as the player streams it. See [StageWatch] for what several
  /// listeners share. [maxRate] is the most frames a second this listener
  /// wants (0 for the player's own rate); the stream runs at the highest any
  /// listener asked for when it was opened.
  ///
  /// The stream begins when it is listened to and does not end until the
  /// watch is disposed: while the link is down, or the player is too old, it
  /// is silent. Cancel it when the stage is not showing.
  Stream<StageEvent> watch({int maxRate = 10}) {
    final watcher = _Watcher(maxRate);
    watcher.controller = StreamController<StageEvent>(
      sync: true,
      onListen: () {
        if (_disposed) {
          unawaited(watcher.controller.close());
          return;
        }
        _watchers.add(watcher);
        _restart();
      },
      onResume: () {
        if (watcher.behind) {
          watcher.behind = false;
          _restart();
        }
      },
      onCancel: () {
        _watchers.remove(watcher);
        if (_watchers.isEmpty) {
          _close();
        }
      },
    );
    return watcher.controller.stream;
  }

  /// The link came up ([ready]) or is no longer up. Coming up opens the
  /// stream again for whoever is still listening.
  void linkChanged({required bool ready}) {
    if (_disposed) {
      return;
    }
    if (!ready) {
      _close();
      return;
    }
    // A player with nothing to carry a stream on is known for what it is at
    // once; one that has it gets asked again, whatever the last one said.
    _missing = !(_client()?.hasStreams ?? true);
    _restart();
  }

  /// Something arrived from the player, so the link is alive.
  void heard() => _heard = true;

  /// The player was forgotten: what was learned of it goes too.
  void clear() {
    _close();
    _missing = false;
  }

  void _restart() {
    _close();
    unawaited(_open());
  }

  /// Cancels the stream at the player, if one is open. Its id, and its place
  /// among the two streams a phone may have, are held until the player's
  /// last response: [_open] waits for that.
  void _close() {
    _generation++;
    _retry?.cancel();
    _retry = null;
    _probe?.cancel();
    _probe = null;
    final stream = _stream;
    _stream = null;
    if (stream != null) {
      final closing = stream.cancel();
      _closing = closing;
      unawaited(
        closing.whenComplete(() {
          if (identical(_closing, closing)) {
            _closing = null;
          }
        }),
      );
    }
  }

  Future<void> _open() async {
    final generation = ++_generation;
    // Always a turn later: this is reached from inside a listener's listen
    // and resume, where nothing may be handed to it yet.
    await (_closing ?? Future<void>.value());
    final client = _client();
    if (_disposed ||
        generation != _generation ||
        _watchers.isEmpty ||
        client == null ||
        !_isReady() ||
        _missing) {
      return;
    }
    if (!client.hasStreams) {
      _setMissing();
      return;
    }
    _emit(const StageReset());
    if (generation != _generation) {
      // A listener answered the reset by leaving, or joining.
      return;
    }
    _heard = true;
    _probe = Timer.periodic(stageProbeInterval, (_) => _checkLink(client));
    _stream = client
        .watchStage(maxRate: _rate())
        .listen(
          (bytes) {
            _heard = true;
            _emit(StageBytes(bytes));
          },
          onError: (Object error) => _failed(client, generation, error),
          onDone: () => _ended(generation),
          cancelOnError: true,
        );
  }

  /// The highest rate any listener asked for; 0, the player's own, is the
  /// highest there is.
  int _rate() {
    var rate = 1;
    for (final watcher in _watchers) {
      if (watcher.maxRate <= 0) {
        return 0;
      }
      if (watcher.maxRate > rate) {
        rate = watcher.maxRate;
      }
    }
    return rate;
  }

  void _emit(StageEvent event) {
    for (final watcher in List.of(_watchers)) {
      if (watcher.controller.isPaused) {
        watcher.behind = true;
      } else if (!watcher.behind) {
        watcher.controller.add(event);
      }
    }
  }

  void _failed(PlayerClient client, int generation, Object error) {
    if (generation != _generation) {
      return;
    }
    _stream = null;
    _probe?.cancel();
    _probe = null;
    if (error is PlayerException && error.isStreamTooOld) {
      _setMissing();
      return;
    }
    if (_lostIfUnanswered(client, error)) {
      // Connecting again opens it again.
      return;
    }
    if (error is PlayerException && error.isUnavailable) {
      // The link went; so will the phase, and its return opens it again.
      return;
    }
    _retryLater();
  }

  /// The player ended the stream, which it does not do short of a failure.
  void _ended(int generation) {
    if (generation != _generation) {
      return;
    }
    _stream = null;
    _probe?.cancel();
    _probe = null;
    _retryLater();
  }

  void _retryLater() {
    _retry?.cancel();
    _retry = Timer(stageRetryDelay, () {
      _retry = null;
      unawaited(_open());
    });
  }

  void _setMissing() {
    if (!_missing) {
      _missing = true;
      _changed();
    }
  }

  /// A stream has no timeout, since a still stage sends nothing, and a link
  /// can die without the phone being told (the player restarted and the radio
  /// kept the link): the stream would then be silent for good. So while one
  /// is open and nothing at all has been heard for a whole interval, a small
  /// call is made, and a call that gets no answer is what the session
  /// already takes for a lost link.
  Future<void> _checkLink(PlayerClient client) async {
    if (_heard) {
      _heard = false;
      return;
    }
    if (_probing) {
      return;
    }
    _probing = true;
    try {
      await client.listPlaylists();
    } on Object catch (e) {
      _lostIfUnanswered(client, e);
    } finally {
      _probing = false;
    }
  }

  /// Ends every listener's stream and lets go of the player's.
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    _close();
    for (final watcher in List.of(_watchers)) {
      unawaited(watcher.controller.close());
    }
    _watchers.clear();
  }
}
