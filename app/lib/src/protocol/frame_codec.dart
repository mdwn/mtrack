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
import 'dart:typed_data';

/// The largest frame either side handles.
const int maxFrame = 64 * 1024;

/// The header byte of the first chunk of a frame.
const int _start = 0x01;

/// The header byte of every later chunk of a frame.
const int _continue = 0x00;

/// A length varint longer than this many bytes is refused. A frame length
/// within [maxFrame] takes three, so this leaves room for padding only.
const int _maxVarintLen = 5;

/// Why a frame could not be cut into chunks or put back together.
///
/// [name] is the stable identifier the golden vectors use, shared with the
/// player's implementation.
class FrameError implements Exception {
  const FrameError._(this.name, this.message);

  /// A chunk with no bytes, so no header.
  const FrameError.emptyChunk() : this._('empty_chunk', 'empty chunk');

  /// A header byte that is neither a start nor a continuation.
  FrameError.badHeader(int byte)
    : this._(
        'bad_header',
        'bad chunk header 0x${byte.toRadixString(16).padLeft(2, '0')}',
      );

  /// A continuation chunk with no frame in progress.
  const FrameError.continuationWithoutStart()
    : this._(
        'continuation_without_start',
        'continuation chunk with no frame in progress',
      );

  /// A start chunk that ends inside its length varint.
  const FrameError.truncatedLength()
    : this._('truncated_length', 'frame length cut short');

  /// A length varint of more than five bytes.
  const FrameError.overlongLength()
    : this._('overlong_length', 'frame length varint too long');

  /// A frame longer than [maxFrame], declared or offered.
  FrameError.frameTooLarge(int length)
    : this._(
        'frame_too_large',
        'frame of $length bytes exceeds the $maxFrame byte limit',
      );

  /// More frame bytes than the start chunk declared.
  const FrameError.tooManyBytes()
    : this._('too_many_bytes', 'more bytes than the frame declared');

  /// A chunk size too small to carry a header, a length and a frame byte.
  FrameError.chunkTooSmall(int size)
    : this._('chunk_too_small', 'chunk size $size is too small');

  /// The stable name of the error, as the golden vectors give it.
  final String name;

  /// A human-readable description.
  final String message;

  @override
  String toString() => 'FrameError($name): $message';
}

/// How many bytes [value] takes as a varint.
int _varintLen(int value) {
  var len = 1;
  var rest = value >> 7;
  while (rest > 0) {
    len++;
    rest >>= 7;
  }
  return len;
}

/// Cuts [frame] into chunks of at most [maxChunk] bytes each. A frame with no
/// bytes is one chunk.
///
/// Throws [FrameError] when the frame is longer than [maxFrame] or [maxChunk]
/// cannot carry the header, the length and one frame byte.
List<Uint8List> chunks(Uint8List frame, int maxChunk) {
  if (frame.length > maxFrame) {
    throw FrameError.frameTooLarge(frame.length);
  }
  final head = 1 + _varintLen(frame.length);
  if (maxChunk < head + (frame.isEmpty ? 0 : 1)) {
    throw FrameError.chunkTooSmall(maxChunk);
  }

  final firstRoom = maxChunk - head;
  final firstBody = frame.length < firstRoom ? frame.length : firstRoom;
  final first = BytesBuilder(copy: false)..addByte(_start);
  var length = frame.length;
  while (length >= 0x80) {
    first.addByte((length & 0x7f) | 0x80);
    length >>= 7;
  }
  first.addByte(length);
  first.add(Uint8List.sublistView(frame, 0, firstBody));

  final out = <Uint8List>[first.takeBytes()];
  var offset = firstBody;
  while (offset < frame.length) {
    final end = offset + (maxChunk - 1) < frame.length
        ? offset + (maxChunk - 1)
        : frame.length;
    final chunk = Uint8List(1 + end - offset)
      ..[0] = _continue
      ..setRange(1, 1 + end - offset, frame, offset);
    out.add(chunk);
    offset = end;
  }
  return out;
}

/// Puts chunks back together into frames.
///
/// It holds the frame in progress and grows its buffer only as bytes arrive,
/// never to a length a peer declared.
class Reassembler {
  final BytesBuilder _buf = BytesBuilder(copy: false);
  int? _expected;

  /// Drops the frame in progress, for a disconnect.
  void reset() {
    _buf.clear();
    _expected = null;
  }

  /// Takes one chunk and returns the frame when it completes one, else null.
  ///
  /// Throws [FrameError], after dropping the frame in progress, for a chunk
  /// that makes no sense. A start chunk while a frame is in progress drops
  /// that frame and begins the new one.
  Uint8List? push(Uint8List chunk) {
    try {
      return _push(chunk);
    } on FrameError {
      reset();
      rethrow;
    }
  }

  Uint8List? _push(Uint8List chunk) {
    if (chunk.isEmpty) {
      throw const FrameError.emptyChunk();
    }
    final header = chunk[0];
    var bodyStart = 1;
    switch (header) {
      case _start:
        reset();
        var len = 0;
        var used = 0;
        var done = false;
        for (var i = 1; i < chunk.length; i++) {
          if (used >= _maxVarintLen) {
            throw const FrameError.overlongLength();
          }
          final byte = chunk[i];
          len |= (byte & 0x7f) << (7 * used);
          used++;
          if (byte & 0x80 == 0) {
            done = true;
            break;
          }
        }
        if (!done) {
          throw used >= _maxVarintLen
              ? const FrameError.overlongLength()
              : const FrameError.truncatedLength();
        }
        if (len > maxFrame) {
          throw FrameError.frameTooLarge(len);
        }
        _expected = len;
        bodyStart = 1 + used;
      case _continue:
        if (_expected == null) {
          throw const FrameError.continuationWithoutStart();
        }
      default:
        throw FrameError.badHeader(header);
    }

    final expected = _expected!;
    final bodyLen = chunk.length - bodyStart;
    if (_buf.length + bodyLen > expected) {
      throw const FrameError.tooManyBytes();
    }
    _buf.add(Uint8List.sublistView(chunk, bodyStart));
    if (_buf.length == expected) {
      _expected = null;
      return _buf.takeBytes();
    }
    return null;
  }
}
