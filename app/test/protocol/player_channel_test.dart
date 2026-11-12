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

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/protocol/envelope_channel.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import '../support/fake_player.dart';

/// A channel that records what it is asked and answers from a table: the
/// seam, with nothing under it.
class RecordingChannel implements PlayerChannel {
  final List<({String method, Uint8List request, Duration timeout})> calls = [];
  final List<({String method, Uint8List request})> streams = [];
  final Map<String, Uint8List Function()> answers = {};
  final StreamController<pp.RemoteStatus> statuses =
      StreamController<pp.RemoteStatus>.broadcast();
  int inFlight = 0;
  bool disposed = false;

  @override
  Carrier carrier = Carrier.wifi;

  @override
  TransportState get currentState => TransportState.ready;

  @override
  Stream<TransportState> get state => const Stream.empty();

  @override
  bool hasStreams = true;

  @override
  int get callsInFlight => inFlight;

  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    calls.add((method: method, request: request, timeout: timeout));
    final answer = answers[method];
    if (answer == null) {
      throw PlayerException(GrpcCode.unimplemented, '$method is not served');
    }
    return answer();
  }

  @override
  Stream<Uint8List> openStream(String method, Uint8List request) {
    streams.add((method: method, request: request));
    return const Stream.empty();
  }

  @override
  Stream<pp.RemoteStatus> get statusStream => statuses.stream;

  @override
  Future<pp.RemoteStatus> readStatus() async =>
      pp.RemoteStatus(songName: 'Now');

  @override
  Future<void> dispose() async => disposed = true;
}

void main() {
  group('the client over the seam', () {
    late RecordingChannel channel;
    late PlayerClient client;

    setUp(() {
      channel = RecordingChannel();
      client = PlayerClient(channel);
    });

    test('calls a method by its bare name with the request\'s bytes', () async {
      channel.answers['Seek'] = () => pp.SeekResponse().writeToBuffer();
      await client.seek(const Duration(seconds: 12, milliseconds: 500));
      final call = channel.calls.single;
      expect(call.method, 'Seek');
      final sent = pp.SeekRequest.fromBuffer(call.request);
      expect(sent.position.seconds.toInt(), 12);
      expect(sent.position.nanos, 500000000);
    });

    test('decodes what comes back', () async {
      channel.answers['GetNetworkInfo'] = () => pp.GetNetworkInfoResponse(
        hostname: 'stage',
        addresses: ['10.0.0.2'],
        grpcPort: 43234,
      ).writeToBuffer();
      final info = await client.getNetworkInfo();
      expect(info.hostname, 'stage');
      expect(info.addresses, ['10.0.0.2']);
      expect(info.grpcPort, 43234);
    });

    test('hands a call\'s timeout to the channel', () async {
      channel.answers['GetNetworkInfo'] = () => Uint8List(0);
      await client.getNetworkInfo(timeout: const Duration(seconds: 3));
      await client.getNetworkInfo();
      expect(channel.calls[0].timeout, const Duration(seconds: 3));
      expect(channel.calls[1].timeout, PlayerChannel.defaultTimeout);
    });

    test('lets the channel\'s failures through as they are', () async {
      await expectLater(
        client.stop(),
        throwsA(
          isA<PlayerException>().having(
            (e) => e.code,
            'code',
            GrpcCode.unimplemented,
          ),
        ),
      );
    });

    test('opens a stream by name with the request\'s bytes', () {
      client.watchStage(maxRate: 7).listen((_) {});
      final stream = channel.streams.single;
      expect(stream.method, 'WatchStage');
      expect(pp.WatchStageRequest.fromBuffer(stream.request).maxRate, 7);
    });

    test('reports the carrier, the streams and the calls in flight', () {
      expect(client.carrier, Carrier.wifi);
      channel.carrier = Carrier.bluetooth;
      expect(client.carrier, Carrier.bluetooth);
      channel.hasStreams = false;
      expect(client.hasStreams, isFalse);
      channel.inFlight = 2;
      expect(client.callsInFlight, 2);
    });

    test('follows the status the channel pushes and reads it once', () async {
      expect((await client.readStatus()).songName, 'Now');
      final seen = <String>[];
      client.statusStream.listen((s) => seen.add(s.songName));
      channel.statuses.add(pp.RemoteStatus(songName: 'Next'));
      await pumpEventQueue();
      expect(seen, ['Next']);
    });

    test('disposing disposes the channel', () async {
      await client.dispose();
      expect(channel.disposed, isTrue);
    });
  });

  group('the Bluetooth envelope as a channel', () {
    late FakePlayer fake;
    late EnvelopeChannel channel;

    setUp(() {
      fake = FakePlayer();
      channel = EnvelopeChannel(fake);
    });

    tearDown(() => channel.dispose());

    test('says what it is and mirrors the transport', () {
      expect(channel.carrier, Carrier.bluetooth);
      expect(channel.currentState, TransportState.ready);
      expect(channel.hasStreams, isTrue);
      fake.hasStreams = false;
      expect(channel.hasStreams, isFalse);
    });

    test(
      'carries a call as request bytes and gives back the payload',
      () async {
        fake.handlers['Seek'] = (_) => FakeReply.ok(pp.SeekResponse());
        final bytes = await channel.call(
          'Seek',
          pp.SeekRequest().writeToBuffer(),
        );
        expect(bytes, isEmpty);
        expect(fake.received.single.method, 'Seek');
      },
    );

    test('counts the calls waiting for an answer', () async {
      fake.handlers['Seek'] = (_) => FakeReply.ok(
        pp.SeekResponse(),
        delay: const Duration(milliseconds: 50),
      );
      final pending = channel.call('Seek', Uint8List(0));
      await pumpEventQueue();
      expect(channel.callsInFlight, 1);
      await pending;
      expect(channel.callsInFlight, 0);
    });

    test('fails with the code the player sent', () async {
      fake.handlers['Seek'] = (_) =>
          FakeReply.error(GrpcCode.failedPrecondition, 'a song is playing');
      await expectLater(
        channel.call('Seek', Uint8List(0)),
        throwsA(
          isA<PlayerException>()
              .having((e) => e.code, 'code', GrpcCode.failedPrecondition)
              .having((e) => e.message, 'message', 'a song is playing'),
        ),
      );
    });

    test('says the link is gone when it is', () async {
      fake.disconnect();
      await expectLater(
        channel.call('Seek', Uint8List(0)),
        throwsA(
          isA<PlayerException>().having((e) => e.isUnavailable, 'down', true),
        ),
      );
    });
  });
}
