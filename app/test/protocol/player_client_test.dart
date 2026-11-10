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
import 'dart:math';
import 'dart:typed_data';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as player;
import 'package:mtrack_remote/src/protocol/frame_codec.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';
import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import '../support/fake_player.dart';

/// A random source whose `nextInt` always returns [value].
class FixedRandom implements Random {
  FixedRandom(this.value);

  final int value;

  @override
  bool nextBool() => false;

  @override
  double nextDouble() => 0;

  @override
  int nextInt(int max) => value;
}

player.Song song(String name) => player.Song(name: name);

/// Runs [body] in fake time, letting microtasks settle after each step.
void withFakeTime(
  String description,
  FutureOr<void> Function(FakeAsync async, FakePlayer fake, PlayerClient client)
  body,
) {
  test(description, () {
    fakeAsync((async) {
      final fake = FakePlayer(maxChunk: 20);
      final client = PlayerClient(fake, random: FixedRandom(100));
      body(async, fake, client);
      async.flushMicrotasks();
      unawaited(client.dispose());
    });
  });
}

void main() {
  late FakePlayer fake;
  late PlayerClient client;

  setUp(() {
    fake = FakePlayer();
    client = PlayerClient(fake, random: FixedRandom(1000));
  });

  tearDown(() async {
    await client.dispose();
  });

  group('calls', () {
    test('sends the method and decodes the response payload', () async {
      fake.handlers['Play'] = (_) =>
          FakeReply.ok(player.PlayResponse(song: song('One')));
      final response = await client.play();
      expect(response.song.name, 'One');
      expect(fake.received.single.method, 'Play');
      expect(fake.received.single.id, 1000);
    });

    test('ids increment and wrap at 32 bits', () async {
      await client.dispose();
      client = PlayerClient(fake, random: FixedRandom(0xfffffffe));
      await client.stop();
      await client.stop();
      await client.stop();
      expect(fake.received.map((r) => r.id), [0xfffffffe, 0xffffffff, 0]);
    });

    test('the default id source differs between clients', () async {
      final first = FakePlayer();
      final second = FakePlayer();
      final a = PlayerClient(first);
      final b = PlayerClient(second);
      await a.stop();
      await b.stop();
      // 2^-32 odds of a false failure.
      expect(first.received.single.id, isNot(second.received.single.id));
      await a.dispose();
      await b.dispose();
    });

    test('every typed call sends its method and request message', () async {
      await client.play();
      await client.playFrom(const Duration(seconds: 3, milliseconds: 500));
      await client.playSongFrom('Two', const Duration(seconds: 9));
      await client.stop();
      await client.next();
      await client.previous();
      await client.seek(const Duration(seconds: 42));
      await client.seekToSection('Chorus');
      await client.switchToPlaylist('Friday');
      await client.status();
      await client.loopSection('Verse');
      await client.stopSectionLoop();
      await client.sectionAck();
      await client.stopSamples();
      await client.cancelAutoPlay();
      await client.listPlaylists();
      await client.getPlaylist();
      await client.getPlaylist('Friday');

      expect(fake.received.map((r) => r.method), [
        'Play',
        'PlayFrom',
        'PlaySongFrom',
        'Stop',
        'Next',
        'Previous',
        'Seek',
        'SeekToSection',
        'SwitchToPlaylist',
        'Status',
        'LoopSection',
        'StopSectionLoop',
        'SectionAck',
        'StopSamples',
        'CancelAutoPlay',
        'ListPlaylists',
        'GetPlaylist',
        'GetPlaylist',
      ]);
      Uint8List payload(int i) => Uint8List.fromList(fake.received[i].payload);
      expect(
        player.PlayFromRequest.fromBuffer(payload(1)).startTime.toDart(),
        const Duration(seconds: 3, milliseconds: 500),
      );
      final playSong = player.PlaySongFromRequest.fromBuffer(payload(2));
      expect(playSong.songName, 'Two');
      expect(playSong.startTime.toDart(), const Duration(seconds: 9));
      expect(
        player.SeekRequest.fromBuffer(payload(6)).position.toDart(),
        const Duration(seconds: 42),
      );
      expect(
        player.SeekToSectionRequest.fromBuffer(payload(7)).sectionName,
        'Chorus',
      );
      expect(
        player.SwitchToPlaylistRequest.fromBuffer(payload(8)).playlistName,
        'Friday',
      );
      expect(
        player.LoopSectionRequest.fromBuffer(payload(10)).sectionName,
        'Verse',
      );
      expect(player.GetPlaylistRequest.fromBuffer(payload(16)).name, '');
      expect(player.GetPlaylistRequest.fromBuffer(payload(17)).name, 'Friday');
      // Every typed call is on the allowlist the player enforces.
      expect(
        fake.received.every((r) => FakePlayer.allowlist.contains(r.method)),
        isTrue,
      );
    });

    test('decodes list responses', () async {
      fake.handlers['ListPlaylists'] = (_) => FakeReply.ok(
        player.ListPlaylistsResponse(names: ['A', 'B'], active: 'B'),
      );
      final response = await client.listPlaylists();
      expect(response.names, ['A', 'B']);
      expect(response.active, 'B');
    });

    test('a non-zero code throws a PlayerException', () async {
      fake.handlers['SwitchToPlaylist'] = (_) =>
          FakeReply.error(GrpcCode.notFound, 'no such playlist: Friday');
      await expectLater(
        client.switchToPlaylist('Friday'),
        throwsA(
          isA<PlayerException>()
              .having((e) => e.code, 'code', 5)
              .having((e) => e.message, 'message', 'no such playlist: Friday'),
        ),
      );
    });

    test('a locked player refusal surfaces as failed precondition', () async {
      fake.handlers['Stop'] = (_) =>
          FakeReply.error(GrpcCode.failedPrecondition, 'locked');
      await expectLater(
        client.stop(),
        throwsA(
          isA<PlayerException>().having(
            (e) => e.code,
            'code',
            GrpcCode.failedPrecondition,
          ),
        ),
      );
    });

    test(
      'a method off the allowlist is refused with PERMISSION_DENIED',
      () async {
        await expectLater(
          client.call(
            'GetConfig',
            player.StatusRequest(),
            player.StatusResponse.fromBuffer,
          ),
          throwsA(
            isA<PlayerException>()
                .having((e) => e.code, 'code', GrpcCode.permissionDenied)
                .having(
                  (e) => e.isPermissionDenied,
                  'isPermissionDenied',
                  isTrue,
                ),
          ),
        );
      },
    );

    test('the gRPC codes are the standard numbers', () {
      expect(GrpcCode.invalidArgument, 3);
      expect(GrpcCode.deadlineExceeded, 4);
      expect(GrpcCode.notFound, 5);
      expect(GrpcCode.permissionDenied, 7);
      expect(GrpcCode.resourceExhausted, 8);
      expect(GrpcCode.failedPrecondition, 9);
      expect(GrpcCode.unavailable, 14);
    });
  });

  group('framing', () {
    test('multi-chunk requests and responses round trip', () async {
      fake.maxChunk = 20;
      final names = [
        for (var i = 0; i < 60; i++) 'Song number $i with a long name',
      ];
      fake.handlers['GetPlaylist'] = (r) => FakeReply.ok(
        player.GetPlaylistResponse(
          name: player.GetPlaylistRequest.fromBuffer(r.payload).name,
          entries: [
            for (final n in names)
              player.PlaylistEntry(
                songName: n,
                duration: pb.Duration.fromDart(const Duration(minutes: 3)),
              ),
          ],
        ),
      );
      final longName = 'P' * 300;
      final response = await client.getPlaylist(longName);
      expect(response.name, longName);
      expect(response.entries.map((e) => e.songName), names);
      // The request really did span many chunks.
      expect(fake.writes.length, greaterThan(15));
      expect(fake.writes.every((c) => c.length <= 20), isTrue);
    });

    test('uses the transport maxChunk as it is when called', () async {
      fake.maxChunk = 185;
      await client.playSongFrom('S' * 400, Duration.zero);
      final big = fake.writes.length;
      fake.maxChunk = 30;
      await client.playSongFrom('S' * 400, Duration.zero);
      expect(fake.writes.length - big, greaterThan(big));
    });

    test(
      'a request that cannot be chunked fails as invalid argument',
      () async {
        fake.maxChunk = 2;
        await expectLater(
          client.playSongFrom('S' * 400, Duration.zero),
          throwsA(
            isA<PlayerException>().having(
              (e) => e.code,
              'code',
              GrpcCode.invalidArgument,
            ),
          ),
        );
      },
    );
  });

  group('matching', () {
    test('ignores a response whose id is not pending', () async {
      fake.handlers['Play'] = (_) {
        // Another phone's answer lands first.
        fake.sendResponse(ble.Response(id: 7, code: 7, message: 'not yours'));
        fake.sendResponse(ble.Response(id: 1001, payload: [1, 2]));
        return FakeReply.ok(player.PlayResponse(song: song('Mine')));
      };
      final response = await client.play();
      expect(response.song.name, 'Mine');
    });

    test('matches responses that arrive out of order', () async {
      fakeAsync((async) {
        final f = FakePlayer();
        final c = PlayerClient(f, random: FixedRandom(10));
        f.handlers['Play'] = (_) => FakeReply.ok(
          player.PlayResponse(song: song('slow')),
          delay: const Duration(seconds: 2),
        );
        f.handlers['Next'] = (_) => FakeReply.ok(
          player.NextResponse(song: song('fast')),
          delay: const Duration(milliseconds: 100),
        );
        String? slow;
        String? fast;
        final order = <String>[];
        unawaited(
          c.play().then((r) {
            slow = r.song.name;
            order.add('play');
          }),
        );
        unawaited(
          c.next().then((r) {
            fast = r.song.name;
            order.add('next');
          }),
        );
        async.elapse(const Duration(seconds: 3));
        expect(order, ['next', 'play']);
        expect(slow, 'slow');
        expect(fast, 'fast');
        unawaited(c.dispose());
        async.flushMicrotasks();
      });
    });

    test('serialises requests so chunks never interleave', () async {
      fakeAsync((async) {
        final f = FakePlayer(maxChunk: 20)
          ..writeDelay = const Duration(milliseconds: 5);
        final c = PlayerClient(f, random: FixedRandom(10));
        final results = <String>[];
        unawaited(
          c
              .playSongFrom('A' * 100, Duration.zero)
              .then((_) => results.add('a')),
        );
        unawaited(
          c
              .playSongFrom('B' * 100, Duration.zero)
              .then((_) => results.add('b')),
        );
        unawaited(c.seekToSection('C' * 100).then((_) => results.add('c')));
        async.elapse(const Duration(seconds: 2));
        expect(results, ['a', 'b', 'c']);
        expect(f.interleavedStarts, 0);
        expect(f.received.map((r) => r.method), [
          'PlaySongFrom',
          'PlaySongFrom',
          'SeekToSection',
        ]);
        unawaited(c.dispose());
        async.flushMicrotasks();
      });
    });
  });

  group('failure', () {
    withFakeTime('a call times out and the late response is ignored', (
      async,
      f,
      c,
    ) {
      f.handlers['Play'] = (_) => FakeReply.drop();
      Object? error;
      unawaited(c.play().then<void>((_) {}, onError: (Object e) => error = e));
      async.elapse(const Duration(seconds: 4));
      expect(error, isNull);
      async.elapse(const Duration(seconds: 2));
      expect(
        error,
        isA<PlayerException>().having((e) => e.isTimeout, 'isTimeout', isTrue),
      );
      // The answer comes after all; nothing happens.
      f.sendResponse(
        ble.Response(id: 100, payload: player.PlayResponse().writeToBuffer()),
      );
      async.flushMicrotasks();
      // A fresh call still works and gets its own id.
      f.handlers['Play'] = (_) =>
          FakeReply.ok(player.PlayResponse(song: song('ok')));
      String? name;
      unawaited(c.play().then((r) => name = r.song.name));
      async.flushMicrotasks();
      expect(name, 'ok');
      expect(f.received.map((r) => r.id), [100, 101]);
    });

    withFakeTime('a per-call timeout is honoured', (async, f, c) {
      f.handlers['Seek'] = (_) => FakeReply.drop();
      Object? error;
      unawaited(
        c
            .call(
              'Seek',
              player.SeekRequest(),
              player.SeekResponse.fromBuffer,
              timeout: const Duration(milliseconds: 200),
            )
            .then<void>((_) {}, onError: (Object e) => error = e),
      );
      async.elapse(const Duration(milliseconds: 199));
      expect(error, isNull);
      async.elapse(const Duration(milliseconds: 2));
      expect(error, isA<PlayerException>());
    });

    withFakeTime('a disconnect fails every pending call', (async, f, c) {
      f.handlers['Play'] = (_) => FakeReply.drop();
      f.handlers['Stop'] = (_) => FakeReply.drop();
      final errors = <Object>[];
      unawaited(c.play().then<void>((_) {}, onError: errors.add));
      unawaited(c.stop().then<void>((_) {}, onError: errors.add));
      async.elapse(const Duration(seconds: 1));
      expect(errors, isEmpty);
      f.disconnect();
      async.flushMicrotasks();
      expect(errors, hasLength(2));
      expect(
        errors.every((e) => e is PlayerException && e.isUnavailable),
        isTrue,
      );
      // Their timers are gone: nothing fires later.
      async.elapse(const Duration(seconds: 10));
      expect(errors, hasLength(2));
    });

    withFakeTime('a disconnect during a multi-chunk write fails the call', (
      async,
      f,
      c,
    ) {
      f.writeDelay = const Duration(milliseconds: 10);
      Object? error;
      unawaited(
        c
            .playSongFrom('Z' * 200, Duration.zero)
            .then<void>((_) {}, onError: (Object e) => error = e),
      );
      async.elapse(const Duration(milliseconds: 35));
      f.disconnect();
      async.elapse(const Duration(seconds: 1));
      expect(
        error,
        isA<PlayerException>().having(
          (e) => e.isUnavailable,
          'unavailable',
          isTrue,
        ),
      );
      // No further chunks went out after the link dropped.
      final written = f.writes.length;
      async.elapse(const Duration(seconds: 1));
      expect(f.writes.length, written);
    });

    test('calling while not connected fails at once', () async {
      fake.setState(TransportState.connecting);
      await expectLater(
        client.play(),
        throwsA(
          isA<PlayerException>().having(
            (e) => e.isUnavailable,
            'unavailable',
            isTrue,
          ),
        ),
      );
      expect(fake.received, isEmpty);
    });

    test('calls work again after reconnecting', () async {
      fake.disconnect();
      fake.setState(TransportState.connecting);
      fake.setState(TransportState.ready);
      await Future<void>.delayed(Duration.zero);
      await client.play();
      expect(fake.received, hasLength(1));
    });

    withFakeTime('a disconnect drops a half-received response', (async, f, c) {
      f.handlers['Play'] = (_) => FakeReply.drop();
      f.handlers['Stop'] = (_) => FakeReply.drop();
      unawaited(c.play().then<void>((_) {}, onError: (Object _) {}));
      async.flushMicrotasks();
      // The answer to the *next* call (id 101), cut into several chunks, one in.
      final answer = chunks(
        ble.Response(id: 101, message: 'xxxxxxxxxxxx').writeToBuffer(),
        4,
      );
      expect(answer.length, greaterThan(2));
      f.sendResponseChunks([answer.first]);
      async.flushMicrotasks();
      f.disconnect();
      f.setState(TransportState.ready);
      async.flushMicrotasks();

      var completed = false;
      Object? error;
      unawaited(
        c.stop().then(
          (_) => completed = true,
          onError: (Object e) => error = e,
        ),
      );
      async.flushMicrotasks();
      // The tail of the old frame must not complete anything.
      f.sendResponseChunks(answer.skip(1));
      async.flushMicrotasks();
      expect(completed, isFalse);
      async.elapse(const Duration(seconds: 6));
      expect(
        error,
        isA<PlayerException>().having((e) => e.isTimeout, 'timeout', isTrue),
      );
    });

    test('dispose fails pending calls and stops listening', () async {
      fake.handlers['Play'] = (_) => FakeReply.drop();
      final pending = client.play();
      final failed = expectLater(
        pending,
        throwsA(
          isA<PlayerException>().having(
            (e) => e.isUnavailable,
            'unavailable',
            isTrue,
          ),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      await client.dispose();
      await failed;
      expect(fake.hasResponseListener, isFalse);
      expect(fake.hasStatusListener, isFalse);
      await expectLater(client.play(), throwsA(isA<PlayerException>()));
    });
  });

  group('status stream', () {
    ble.Status status(String name, {bool playing = true}) => ble.Status(
      playlistName: 'Friday',
      songName: name,
      playing: playing,
      songDuration: pb.Duration.fromDart(const Duration(minutes: 3)),
      elapsed: pb.Duration.fromDart(const Duration(seconds: 12)),
      position: 2,
      autoPlay: player.AutoPlayStatus(
        song: 'Next one',
        startsIn: pb.Duration.fromDart(const Duration(seconds: 4)),
      ),
    );

    test('decodes multi-chunk status frames', () async {
      final seen = <ble.Status>[];
      final sub = client.statusStream.listen(seen.add);
      fake.emitStatus(
        status('A song with a rather long name indeed'),
        chunkSize: 20,
      );
      fake.emitStatus(status('B'), chunkSize: 20);
      await Future<void>.delayed(Duration.zero);
      expect(seen.map((s) => s.songName), [
        'A song with a rather long name indeed',
        'B',
      ]);
      expect(seen.first.position, 2);
      expect(seen.first.autoPlay.song, 'Next one');
      expect(seen.first.elapsed.toDart(), const Duration(seconds: 12));
      await sub.cancel();
    });

    test('ignores continuation chunks before the first start', () async {
      final seen = <ble.Status>[];
      final sub = client.statusStream.listen(seen.add);
      final cut = chunks(status('Cut').writeToBuffer(), 20);
      expect(cut.length, greaterThan(2));
      // Subscribed mid-frame: only the tail arrives.
      fake.emitStatusChunks(cut.skip(1));
      await Future<void>.delayed(Duration.zero);
      expect(seen, isEmpty);
      fake.emitStatus(status('Whole'), chunkSize: 20);
      await Future<void>.delayed(Duration.zero);
      expect(seen.map((s) => s.songName), ['Whole']);
      await sub.cancel();
    });

    test('drops a malformed frame without ending the stream', () async {
      final seen = <ble.Status>[];
      final errors = <Object>[];
      final sub = client.statusStream.listen(seen.add, onError: errors.add);
      // A well-framed message that is not a protobuf.
      fake.emitStatusFrame(Uint8List.fromList([0xff, 0xff, 0xff, 0xff, 0xff]));
      // A bad header.
      fake.emitStatusChunks([
        Uint8List.fromList([0x09, 0x01]),
      ]);
      fake.emitStatus(status('After'), chunkSize: 20);
      await Future<void>.delayed(Duration.zero);
      expect(errors, isEmpty);
      expect(seen.map((s) => s.songName), ['After']);
      await sub.cancel();
    });

    test('a start mid-frame replaces the partial frame', () async {
      final seen = <ble.Status>[];
      final sub = client.statusStream.listen(seen.add);
      final cut = chunks(status('Dropped').writeToBuffer(), 20);
      fake.emitStatusChunks(cut.take(2));
      fake.emitStatus(status('Kept'), chunkSize: 20);
      await Future<void>.delayed(Duration.zero);
      expect(seen.map((s) => s.songName), ['Kept']);
      await sub.cancel();
    });

    test('readStatus decodes the bare message', () async {
      fake.statusBytes = Uint8List.fromList(status('Read').writeToBuffer());
      final read = await client.readStatus();
      expect(read.songName, 'Read');
      expect(read.playing, isTrue);
    });
  });
}
