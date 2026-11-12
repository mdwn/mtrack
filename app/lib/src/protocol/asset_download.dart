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
import 'dart:io';
import 'dart:typed_data';

import 'package:protobuf/protobuf.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import 'player_exception.dart';

/// The largest lighting asset the app takes, as sent and once unpacked: the
/// player's own cap on what it serves.
const int maxLightingAssetBytes = 64 * 1024 * 1024;

/// Told as an asset arrives: [received] bytes so far of [total], counted as
/// the player sends them (gzipped). [total] is 0 when the player did not say.
typedef AssetProgress = void Function(int received, int total);

/// One lighting asset coming from the player (`GetLightingAsset`): its
/// chunks taken as they arrive, joined, and unpacked when they were sent
/// gzipped.
///
/// The chunks are consumed the moment they arrive, never paused for, so the
/// stream holds nothing back for this listener and its limit on what it
/// holds (`EnvelopeChannel.maxHeldStreamBytes`) does not apply to an asset of
/// any size; what is kept is the asset itself, up to [maxBytes].
class AssetDownload {
  /// A download of the messages of a `GetLightingAsset` stream, each a
  /// serialized `AssetChunk`. Listens at once.
  AssetDownload(
    Stream<Uint8List> messages, {
    this.maxBytes = maxLightingAssetBytes,
    this._onProgress,
  }) {
    // A caller that cancels and never looks at [bytes] is not an error.
    _result.future.ignore();
    _subscription = messages.listen(
      _onMessage,
      onError: (Object error) => _finish(
        error: error is PlayerException
            ? error
            : PlayerException(GrpcCode.unavailable, '$error'),
      ),
      onDone: _onDone,
    );
  }

  /// The most bytes taken, as sent and once unpacked.
  final int maxBytes;

  final AssetProgress? _onProgress;
  final Completer<Uint8List> _result = Completer<Uint8List>();
  final Completer<void> _freed = Completer<void>();
  final BytesBuilder _data = BytesBuilder(copy: false);
  late final StreamSubscription<Uint8List> _subscription;

  bool _first = true;
  bool _finished = false;
  int _received = 0;
  int _total = 0;

  /// The asset, unpacked. Fails with a [PlayerException]: the player's own
  /// code (`NOT_FOUND` for a file it does not have), `UNAVAILABLE` when the
  /// link went, `RESOURCE_EXHAUSTED` for an asset over [maxBytes],
  /// `DATA_LOSS` for one that arrived short or damaged, and `CANCELLED`
  /// after [cancel].
  Future<Uint8List> get bytes => _result.future;

  /// Completes once the player has let go of the stream, so its place among
  /// the streams a phone may have open is free again. The next download
  /// waits for it.
  Future<void> get freed => _freed.future;

  /// Bytes received so far, as sent.
  int get received => _received;

  /// The bytes the player said it would send, or 0 until it has.
  int get total => _total;

  /// Stops the download: [bytes] fails with `CANCELLED`, and the player is
  /// told. Completes as [freed] does.
  Future<void> cancel() {
    _finish(error: const PlayerException(GrpcCode.cancelled, 'cancelled'));
    return freed;
  }

  void _onMessage(Uint8List message) {
    if (_finished) {
      return;
    }
    final player.AssetChunk chunk;
    try {
      chunk = player.AssetChunk.fromBuffer(message);
    } on InvalidProtocolBufferException {
      _finish(
        error: const PlayerException(
          GrpcCode.dataLoss,
          'a chunk of the asset did not decode',
        ),
      );
      return;
    }
    if (_first) {
      _first = false;
      final total = chunk.totalSize;
      if (total > maxBytes) {
        _finish(error: _tooLarge('$total'));
        return;
      }
      _total = total.toInt();
    }
    _received += chunk.data.length;
    if (_received > maxBytes) {
      _finish(error: _tooLarge('over $maxBytes'));
      return;
    }
    if (_total > 0 && _received > _total) {
      _finish(
        error: PlayerException(
          GrpcCode.dataLoss,
          'the asset ran past the $_total bytes it was said to be',
        ),
      );
      return;
    }
    _data.add(chunk.data);
    _onProgress?.call(_received, _total);
  }

  void _onDone() {
    if (_finished) {
      return;
    }
    if (_received < _total) {
      _finish(
        error: PlayerException(
          GrpcCode.dataLoss,
          'the asset ended after $_received of its $_total bytes',
        ),
      );
      return;
    }
    final sent = _data.takeBytes();
    try {
      _finish(bytes: unpackAsset(sent, maxBytes: maxBytes));
    } on PlayerException catch (e) {
      _finish(error: e);
    }
  }

  void _finish({Uint8List? bytes, PlayerException? error}) {
    if (_finished) {
      return;
    }
    _finished = true;
    _data.clear();
    if (error != null) {
      _result.completeError(error);
    } else {
      _result.complete(bytes);
    }
    // Cancelling a stream that has ended sends nothing; one that has not
    // is cancelled at the player, and its id is held until it lets go.
    unawaited(
      _subscription.cancel().whenComplete(() {
        if (!_freed.isCompleted) {
          _freed.complete();
        }
      }),
    );
  }

  PlayerException _tooLarge(String size) => PlayerException(
    GrpcCode.resourceExhausted,
    'the asset is $size bytes, over the $maxBytes the app takes',
  );
}

/// [sent] as it was before the player gzipped it, or as it is when it was
/// not gzipped: no asset kind (JSON, glTF binary, PNG, SVG) starts with
/// gzip's two magic bytes. Throws a [PlayerException] with `DATA_LOSS` for
/// gzip that does not unpack or is cut short, and `RESOURCE_EXHAUSTED` when it unpacks to
/// more than [maxBytes]: it is unpacked a piece at a time and stopped there,
/// so a small file cannot fill the phone's memory.
Uint8List unpackAsset(Uint8List sent, {int maxBytes = maxLightingAssetBytes}) {
  if (sent.length < 2 || sent[0] != 0x1f || sent[1] != 0x8b) {
    return sent;
  }
  final out = _CappedSink(maxBytes);
  try {
    gzip.decoder.startChunkedConversion(out)
      ..add(sent)
      ..close();
  } on _TooLarge {
    throw PlayerException(
      GrpcCode.resourceExhausted,
      'the asset unpacks to over the $maxBytes bytes the app takes',
    );
  } on Object catch (e) {
    throw PlayerException(GrpcCode.dataLoss, 'the asset did not unpack: $e');
  }
  final bytes = out.takeBytes();
  // The decoder takes gzip cut short without a word, so the length gzip
  // ends with (the unpacked size, modulo 2^32, little-endian) is checked.
  final end = sent.length;
  final said = sent.length < 18
      ? -1
      : sent[end - 4] |
            sent[end - 3] << 8 |
            sent[end - 2] << 16 |
            sent[end - 1] << 24;
  if (said != bytes.length & 0xffffffff) {
    throw const PlayerException(
      GrpcCode.dataLoss,
      'the asset did not unpack: it was cut short',
    );
  }
  return bytes;
}

class _TooLarge implements Exception {
  const _TooLarge();
}

/// Keeps what is written to it, and throws once it would hold more than
/// [max] bytes.
class _CappedSink implements Sink<List<int>> {
  _CappedSink(this.max);

  final int max;
  final BytesBuilder _bytes = BytesBuilder();

  @override
  void add(List<int> data) {
    if (_bytes.length + data.length > max) {
      throw const _TooLarge();
    }
    _bytes.add(data);
  }

  @override
  void close() {}

  Uint8List takeBytes() => _bytes.takeBytes();
}
