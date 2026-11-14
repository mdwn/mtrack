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
import 'asset_cache.dart';

/// How long a download may go without a chunk before it is given up. A
/// still stage sends nothing on the stage stream, but a download that has
/// begun always has more to send.
const Duration assetIdleTimeout = Duration(seconds: 30);

/// Why a lighting asset could not be had.
enum AssetFailure {
  /// The player does not have it.
  notFound,

  /// The player is too old to send assets.
  tooOld,

  /// The link was down, or went down while it came.
  unavailable,

  /// It came short or damaged, too large, or too slowly.
  failed,
}

/// A lighting asset that could not be had now: asking again later may do.
/// (One the player does not have is not this: see [LightingAssets.get].)
class AssetUnavailable implements Exception {
  /// An asset that could not be had, for [reason].
  const AssetUnavailable(this.reason, this.message);

  /// Why.
  final AssetFailure reason;

  /// What happened, for a log.
  final String message;

  @override
  String toString() => 'AssetUnavailable(${reason.name}): $message';
}

/// The player's lighting assets (rig models, meshes, scenery) for the 3D
/// view: from the phone's cache when it has them, from the player over the
/// link when it does not.
///
/// The link is slow and shared with the stage stream, and the player allows
/// a phone two streams, of which the stage holds one. So one asset is
/// fetched at a time, in the order asked for, and the next waits until the
/// player has let go of the one before; two asks for the same asset share
/// one fetch. What is fetched is kept in the cache.
///
/// Listeners are told how the fetching goes ([loading], [done], [count],
/// [fraction]) and what failed since [clearFailures] ([failures],
/// [onlyMissing], [tooOld]).
class LightingAssets extends ChangeNotifier {
  /// Assets fetched through [client] while [isReady], kept in [cache] (none
  /// kept when null).
  LightingAssets({required this._client, required this._isReady, this._cache});

  final PlayerClient? Function() _client;
  final bool Function() _isReady;
  final LightingAssetCache? _cache;

  final Map<String, Future<Uint8List?>> _asked = {};
  Future<void> _queue = Future<void>.value();
  AssetDownload? _current;

  /// Moves when the link goes, so fetches queued before it fail rather than
  /// go to a player that may not be the same one.
  int _link = 0;

  bool _up = false;
  bool _tooOld = false;
  bool _disposed = false;

  int _count = 0;
  int _done = 0;
  int _received = 0;
  int _total = 0;
  int _failures = 0;
  bool _onlyMissing = true;

  /// True while an asset is being fetched or waits to be.
  bool get loading => _done < _count;

  /// Of the assets fetched since nothing was last loading, how many are
  /// over (fetched or failed), and how many were asked for.
  int get done => _done;
  int get count => _count;

  /// How far the fetching since nothing was last loading has come, from 0
  /// to 1, counting the file under way by its bytes; null while that file's
  /// size is not known.
  double? get fraction {
    if (_count == 0) {
      return null;
    }
    if (_current != null && _total <= 0) {
      return null;
    }
    final part = _current != null ? _received / _total : 0.0;
    return ((_done + part) / _count).clamp(0.0, 1.0);
  }

  /// How many assets could not be had since [clearFailures].
  int get failures => _failures;

  /// True when every one that failed did so because the player does not
  /// have it: asking again will not help.
  bool get onlyMissing => _failures > 0 && _onlyMissing;

  /// True when the connected player refused to send an asset as a method it
  /// does not have: it is from before the 3D view on the phone. Assets in
  /// the cache are still served.
  bool get tooOld => _tooOld;

  /// Forgets what failed, before the 3D view asks again.
  void clearFailures() {
    if (_failures == 0) {
      return;
    }
    _failures = 0;
    _onlyMissing = true;
    _notify();
  }

  /// The asset at [path] (as the page asks for it, under
  /// `lighting-assets/`), or null when the player does not have it.
  /// Throws [AssetUnavailable] when it could not be had now: the link is
  /// down or went, the player is too old, or the download failed. A [path]
  /// that does not pass [checkLightingAssetPath] is not asked for, and is
  /// null.
  Future<Uint8List?> get(String path) {
    if (checkLightingAssetPath(path) == null) {
      return Future<Uint8List?>.value();
    }
    final asked = _asked[path];
    if (asked != null) {
      return asked;
    }
    final future = _get(path);
    _asked[path] = future;
    // The future itself goes to the caller; this only forgets it.
    future.then<void>((_) {}, onError: (Object _) {}).whenComplete(() {
      if (identical(_asked[path], future)) {
        _asked.remove(path);
      }
    }).ignore();
    return future;
  }

  Future<Uint8List?> _get(String path) async {
    final cached = await _cache?.read(path);
    if (cached != null) {
      return cached;
    }
    final link = _link;
    _count++;
    _notify();
    try {
      return await _inTurn(() => _fetch(path, link));
    } on AssetUnavailable catch (e) {
      _failed(e.reason);
      rethrow;
    } finally {
      _done++;
      if (_done >= _count) {
        _done = 0;
        _count = 0;
      }
      _notify();
    }
  }

  /// Runs [body] once every fetch asked for before it is over.
  Future<T> _inTurn<T>(Future<T> Function() body) {
    final before = _queue;
    final turn = Completer<void>();
    _queue = turn.future;
    return before.then((_) => body()).whenComplete(turn.complete);
  }

  Future<Uint8List?> _fetch(String path, int link) async {
    final client = _client();
    if (_disposed || link != _link || client == null || !_isReady()) {
      throw const AssetUnavailable(AssetFailure.unavailable, 'no link');
    }
    if (_tooOld) {
      throw const AssetUnavailable(
        AssetFailure.tooOld,
        'the player is too old to send assets',
      );
    }
    _received = 0;
    _total = 0;
    Timer? idle;
    late final AssetDownload download;
    void stall() {
      idle?.cancel();
      idle = Timer(assetIdleTimeout, () => unawaited(download.cancel()));
    }

    download = client.getLightingAsset(
      path,
      onProgress: (received, total) {
        _received = received;
        _total = total;
        stall();
        _notify();
      },
    );
    _current = download;
    stall();
    _notify();
    try {
      final bytes = await download.bytes;
      await _cache?.write(path, bytes);
      return bytes;
    } on PlayerException catch (e) {
      if (e.code == GrpcCode.notFound) {
        _failed(AssetFailure.notFound);
        return null;
      }
      if (e.isStreamTooOld) {
        if (!_tooOld) {
          _tooOld = true;
          _notify();
        }
        throw AssetUnavailable(AssetFailure.tooOld, e.message);
      }
      if (e.isUnavailable) {
        throw AssetUnavailable(AssetFailure.unavailable, e.message);
      }
      if (e.code == GrpcCode.cancelled) {
        throw AssetUnavailable(
          _disposed || link != _link
              ? AssetFailure.unavailable
              : AssetFailure.failed,
          _disposed || link != _link
              ? 'the link went'
              : 'nothing came for ${assetIdleTimeout.inSeconds} seconds',
        );
      }
      throw AssetUnavailable(AssetFailure.failed, e.message);
    } finally {
      idle?.cancel();
      _current = null;
      _received = 0;
      _total = 0;
      // The next fetch waits until the player has let go of this one.
      await download.freed;
    }
  }

  void _failed(AssetFailure reason) {
    _failures++;
    if (reason != AssetFailure.notFound) {
      _onlyMissing = false;
    }
    _notify();
  }

  /// The link came up ([ready]) or went. Going stops the fetch under way and
  /// fails the ones waiting; coming up asks the player afresh whether it can
  /// send assets, since it may have been updated meanwhile.
  void linkChanged({required bool ready}) {
    if (ready == _up) {
      return;
    }
    _up = ready;
    if (ready) {
      if (_tooOld) {
        _tooOld = false;
        _notify();
      }
      return;
    }
    _link++;
    final current = _current;
    if (current != null) {
      unawaited(current.cancel());
    }
  }

  /// The player was forgotten: what was learned of it goes too.
  void clear() {
    _up = true;
    linkChanged(ready: false);
    _tooOld = false;
    _failures = 0;
    _onlyMissing = true;
    _notify();
  }

  void _notify() {
    if (!_disposed) {
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _link++;
    final current = _current;
    if (current != null) {
      unawaited(current.cancel());
    }
    super.dispose();
  }
}
