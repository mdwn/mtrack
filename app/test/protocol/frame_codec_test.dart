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
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/protocol/frame_codec.dart';

/// The smallest chunk size that can carry a frame of [len] bytes.
int minChunk(int len) {
  var varint = 1;
  for (var rest = len >> 7; rest > 0; rest >>= 7) {
    varint++;
  }
  return 1 + varint + (len > 0 ? 1 : 0);
}

Uint8List frameOf(int len) =>
    Uint8List.fromList([for (var i = 0; i < len; i++) i % 251]);

Uint8List unhex(String text) {
  expect(text.length.isEven, isTrue, reason: 'odd hex $text');
  return Uint8List.fromList([
    for (var i = 0; i < text.length; i += 2)
      int.parse(text.substring(i, i + 2), radix: 16),
  ]);
}

String hex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

Uint8List varintStart(int length, [List<int> body = const []]) {
  final out = <int>[0x01];
  var rest = length;
  while (rest >= 0x80) {
    out.add((rest & 0x7f) | 0x80);
    rest >>= 7;
  }
  out.add(rest);
  return Uint8List.fromList([...out, ...body]);
}

/// Feeds every chunk to a reassembler; the frame its last one completes.
Uint8List reassemble(List<Uint8List> chunks) {
  final reassembler = Reassembler();
  Uint8List? frame;
  for (var i = 0; i < chunks.length; i++) {
    final done = reassembler.push(chunks[i]);
    if (i + 1 < chunks.length) {
      expect(done, isNull, reason: 'frame completed early at chunk $i');
    } else {
      frame = done;
    }
  }
  expect(frame, isNotNull, reason: 'a complete frame');
  return frame!;
}

/// The error name a call raises, or null when it does not raise one.
String? errorOf(void Function() body) {
  try {
    body();
  } on FrameError catch (e) {
    return e.name;
  }
  return null;
}

void main() {
  test('round trips every chunk size over a spread of frames', () {
    for (final len in [
      0,
      1,
      2,
      17,
      18,
      19,
      127,
      128,
      129,
      1000,
      16383,
      16384,
      16385,
      maxFrame,
    ]) {
      final frame = frameOf(len);
      final sizes = len <= 1000
          ? [for (var s = minChunk(len); s <= 517; s++) s]
          : [
              minChunk(len),
              minChunk(len) + 1,
              20,
              23,
              185,
              244,
              512,
              517,
            ].where((s) => s >= minChunk(len)).toList();
      for (final size in sizes) {
        final cut = chunks(frame, size);
        expect(cut.every((c) => c.isNotEmpty && c.length <= size), isTrue);
        expect(cut.first[0], 0x01);
        expect(cut.skip(1).every((c) => c[0] == 0x00), isTrue);
        expect(reassemble(cut), frame, reason: 'len $len at $size');
      }
    }
  });

  test('a chunk size below the minimum is refused', () {
    for (final len in [0, 1, 127, 128, 16384, maxFrame]) {
      final frame = frameOf(len);
      final min = minChunk(len);
      expect(chunks(frame, min), isNotEmpty, reason: 'len $len at $min');
      expect(
        errorOf(() => chunks(frame, min - 1)),
        'chunk_too_small',
        reason: 'len $len',
      );
    }
    expect(errorOf(() => chunks(Uint8List(0), 0)), 'chunk_too_small');
  });

  test('a frame over the limit is refused both ways', () {
    expect(
      errorOf(() => chunks(frameOf(maxFrame + 1), 517)),
      'frame_too_large',
    );
    expect(
      errorOf(() => Reassembler().push(varintStart(maxFrame + 1))),
      'frame_too_large',
    );
    // The limit itself is fine.
    expect(reassemble(chunks(frameOf(maxFrame), 517)).length, maxFrame);
  });

  test('a hostile length takes nothing but what arrives', () {
    final reassembler = Reassembler();
    expect(reassembler.push(varintStart(maxFrame, [0xaa])), isNull);
    // The rest of the frame still completes it.
    final rest = Uint8List.fromList([0x00, ...List.filled(maxFrame - 1, 1)]);
    expect(reassembler.push(rest), hasLength(maxFrame));
  });

  test('a new start drops the frame in progress', () {
    final reassembler = Reassembler();
    final abandoned = chunks(frameOf(100), 20);
    for (final chunk in abandoned.take(3)) {
      expect(reassembler.push(chunk), isNull);
    }
    final fresh = frameOf(30);
    Uint8List? done;
    for (final chunk in chunks(fresh, 20)) {
      done = reassembler.push(chunk);
    }
    expect(done, fresh);
    // Nothing of the dropped frame is left to continue.
    expect(
      errorOf(() => reassembler.push(abandoned[3])),
      'continuation_without_start',
    );
  });

  test('reset drops the frame in progress', () {
    final reassembler = Reassembler();
    final cut = chunks(frameOf(100), 20);
    expect(reassembler.push(cut[0]), isNull);
    reassembler.reset();
    expect(
      errorOf(() => reassembler.push(cut[1])),
      'continuation_without_start',
    );
  });

  test('each error resets the state', () {
    final errors = <(List<int>, String)>[
      (const [], 'empty_chunk'),
      (const [0x07], 'bad_header'),
      (const [0x00, 1, 2, 3, 4, 5], 'too_many_bytes'),
    ];
    for (final (bad, name) in errors) {
      final reassembler = Reassembler();
      expect(reassembler.push(Uint8List.fromList([0x01, 0x03, 0x01])), isNull);
      expect(errorOf(() => reassembler.push(Uint8List.fromList(bad))), name);
      expect(
        errorOf(() => reassembler.push(Uint8List.fromList([0x00, 0x02]))),
        'continuation_without_start',
      );
      expect(
        reassembler.push(Uint8List.fromList([0x01, 0x01, 0x09])),
        Uint8List.fromList([0x09]),
      );
    }

    final reassembler = Reassembler();
    expect(
      errorOf(() => reassembler.push(Uint8List.fromList([0x01, 0x80]))),
      'truncated_length',
    );
    expect(
      errorOf(() => reassembler.push(Uint8List.fromList([0x00, 0x01]))),
      'continuation_without_start',
    );
  });

  test('a frame completes exactly once', () {
    final reassembler = Reassembler();
    expect(
      reassembler.push(Uint8List.fromList([0x01, 0x01, 0x05])),
      Uint8List.fromList([0x05]),
    );
    expect(
      errorOf(() => reassembler.push(Uint8List.fromList([0x00, 0x05]))),
      'continuation_without_start',
    );
  });

  group('golden vectors', () {
    // Read from the repository, not a copy, so the two implementations cannot
    // drift. `flutter test` runs from app/.
    final file = File('../src/proto/ble/v1/testdata/frames.json');
    final vectors = jsonDecode(file.readAsStringSync()) as Map<String, Object?>;
    final valid = (vectors['valid']! as List<Object?>)
        .cast<Map<String, Object?>>();
    final invalid = (vectors['invalid']! as List<Object?>)
        .cast<Map<String, Object?>>();

    test('there are vectors', () {
      expect(valid, isNotEmpty);
      expect(invalid, isNotEmpty);
    });

    for (final vector in valid) {
      test('valid: ${vector['name']}', () {
        final frame = unhex(vector['frame_hex']! as String);
        final expected = (vector['chunks_hex']! as List<Object?>)
            .cast<String>();
        final cut = chunks(frame, vector['max_chunk']! as int);
        expect(cut.map(hex).toList(), expected);
        expect(
          hex(reassemble(expected.map(unhex).toList())),
          vector['frame_hex'],
        );
      });
    }

    test('every error a chunk can cause has a vector', () {
      final named = invalid.map((v) => v['error']).toSet();
      for (final name in [
        'empty_chunk',
        'bad_header',
        'continuation_without_start',
        'truncated_length',
        'overlong_length',
        'frame_too_large',
        'too_many_bytes',
      ]) {
        expect(named, contains(name));
      }
    });

    for (final vector in invalid) {
      test('invalid: ${vector['name']}', () {
        final reassembler = Reassembler();
        final cut = (vector['chunks_hex']! as List<Object?>)
            .cast<String>()
            .map(unhex)
            .toList();
        for (final chunk in cut.take(cut.length - 1)) {
          reassembler.push(chunk);
        }
        expect(errorOf(() => reassembler.push(cut.last)), vector['error']);
      });
    }
  });
}
