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

import 'package:flutter_test/flutter_test.dart';
import 'package:grpc/grpc.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/protocol/grpc_channel.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

/// The few methods of `PlayerService` the tests need, served for real over
/// HTTP/2 on the loopback, in raw bytes as the channel speaks them.
class FakeGrpcPlayer extends Service {
  FakeGrpcPlayer() {
    _unary('GetNetworkInfo', (_) async {
      return pp.GetNetworkInfoResponse(
        hostname: 'stage',
        addresses: ['192.168.1.40'],
        grpcPort: 43234,
      ).writeToBuffer();
    });
    _unary('Play', (_) async => pp.PlayResponse().writeToBuffer());
    _unary('Stop', (_) async {
      throw const GrpcError.failedPrecondition('nothing is playing');
    });
    _unary('Seek', (_) async {
      throw const GrpcError.permissionDenied('the player is locked');
    });
    _unary('Next', (_) async {
      // Never answers in time.
      await Future<void>.delayed(const Duration(seconds: 5));
      return pp.NextResponse().writeToBuffer();
    });
    _stream('WatchStatus', (request) {
      final controller = StreamController<List<int>>();
      statusWatchers.add(controller);
      controller.onCancel = () => statusWatchers.remove(controller);
      controller.add(status.writeToBuffer());
      return controller.stream;
    });
    _stream('WatchStage', (request) {
      stageRequests.add(pp.WatchStageRequest.fromBuffer(request));
      final controller = StreamController<List<int>>();
      stages.add(controller);
      controller.onCancel = () {
        stagesCancelled++;
        stages.remove(controller);
      };
      return controller.stream;
    });
  }

  @override
  String get $name => 'player.v1.PlayerService';

  /// The status a new `WatchStatus` starts with.
  pp.RemoteStatus status = pp.RemoteStatus(
    songName: 'First',
    playlistName: 'Set',
    locked: false,
  );

  /// The `WatchStatus` calls open now.
  final List<StreamController<List<int>>> statusWatchers = [];

  /// The `WatchStage` calls open now, and how many were cancelled.
  final List<StreamController<List<int>>> stages = [];
  int stagesCancelled = 0;
  final List<pp.WatchStageRequest> stageRequests = [];

  /// Methods not served, so the call is `UNIMPLEMENTED`, as a player from
  /// before them would answer.
  final Set<String> unserved = {};

  void _unary(String name, Future<List<int>> Function(List<int>) handler) {
    $addMethod(
      ServiceMethod<List<int>, List<int>>(
        name,
        (ServiceCall call, Future<List<int>> request) async =>
            handler(await request),
        false,
        false,
        (bytes) => bytes,
        (bytes) => bytes,
      ),
    );
  }

  void _stream(String name, Stream<List<int>> Function(List<int>) handler) {
    $addMethod(
      ServiceMethod<List<int>, List<int>>(
        name,
        (ServiceCall call, Future<List<int>> request) async* {
          yield* handler(await request);
        },
        false,
        true,
        (bytes) => bytes,
        (bytes) => bytes,
      ),
    );
  }

  /// Pushes a status to every watcher.
  void push(pp.RemoteStatus next) {
    for (final watcher in statusWatchers) {
      watcher.add(next.writeToBuffer());
    }
  }
}

/// Waits, in real time, until [done] is true.
Future<void> waitFor(bool Function() done) async {
  final until = DateTime.now().add(const Duration(seconds: 5));
  while (!done() && DateTime.now().isBefore(until)) {
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
}

/// A TCP relay in front of the server, to cut the link or let it go silent
/// the way a network does.
class Proxy {
  Proxy._(this._listener);

  final ServerSocket _listener;
  final List<Socket> _sockets = [];
  bool _silent = false;

  int get port => _listener.port;

  static Future<Proxy> start(int target) async {
    final listener = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final proxy = Proxy._(listener);
    listener.listen((client) async {
      final upstream = await Socket.connect('127.0.0.1', target);
      proxy._sockets
        ..add(client)
        ..add(upstream);
      client.listen(
        (data) => proxy._silent ? null : upstream.add(data),
        onError: (_) {},
        onDone: upstream.destroy,
      );
      upstream.listen(
        (data) => proxy._silent ? null : client.add(data),
        onError: (_) {},
        onDone: client.destroy,
      );
    });
    return proxy;
  }

  /// Closes every connection, abruptly.
  void cut() {
    for (final socket in _sockets) {
      socket.destroy();
    }
  }

  /// Stops passing anything either way, and closes nothing.
  void blackhole() => _silent = true;

  Future<void> close() async {
    cut();
    await _listener.close();
  }
}

void main() {
  late FakeGrpcPlayer service;
  late Server server;
  late GrpcChannel channel;

  Future<GrpcChannel> open({Duration timeout = const Duration(seconds: 2)}) =>
      GrpcChannel.connect('127.0.0.1', server.port!, timeout: timeout);

  final serverErrors = <Object>[];

  /// Serves [s] on the loopback. The HTTP/2 package asserts, in debug builds
  /// only, when a client drops a connection with an answer still unread; that
  /// is the test server's own business, and not what is being tested, so
  /// that alone is let go.
  Future<void> serve(Server s) async {
    await runZonedGuarded(
      () => s.serve(address: InternetAddress.loopbackIPv4, port: 0),
      (error, stack) {
        if (error is! AssertionError) {
          serverErrors.add(error);
        }
      },
    );
  }

  setUp(() async {
    service = FakeGrpcPlayer();
    server = Server.create(services: [service]);
    await serve(server);
  });

  tearDown(() async {
    await server.shutdown();
    expect(serverErrors, isEmpty);
  });

  test('opens on the first status, and says it is a Wi-Fi channel', () async {
    channel = await open();
    expect(channel.carrier, Carrier.wifi);
    expect(channel.currentState, TransportState.ready);
    expect(channel.hasStreams, isTrue);
    expect((await channel.readStatus()).songName, 'First');
    await channel.dispose();
    expect(channel.currentState, TransportState.disconnected);
  });

  test('follows the status as the player pushes it', () async {
    channel = await open();
    final seen = <String>[];
    final sub = channel.statusStream.listen((s) => seen.add(s.songName));
    service.push(pp.RemoteStatus(songName: 'Second'));
    service.push(pp.RemoteStatus(songName: 'Third'));
    await pumpEventQueue();
    await Future<void>.delayed(const Duration(milliseconds: 100));
    expect(seen, ['Second', 'Third']);
    expect((await channel.readStatus()).songName, 'Third');
    await sub.cancel();
    await channel.dispose();
  });

  test(
    'makes a unary call and decodes the answer through the client',
    () async {
      channel = await open();
      final client = PlayerClient(channel);
      final info = await client.getNetworkInfo();
      expect(info.hostname, 'stage');
      expect(info.addresses, ['192.168.1.40']);
      expect(info.grpcPort, 43234);
      expect(await client.play(), isA<pp.PlayResponse>());
      expect(channel.callsInFlight, 0);
      await channel.dispose();
    },
  );

  test('maps a status the player answers with onto the same codes', () async {
    channel = await open();
    final client = PlayerClient(channel);
    await expectLater(
      client.stop(),
      throwsA(
        isA<PlayerException>()
            .having((e) => e.code, 'code', GrpcCode.failedPrecondition)
            .having((e) => e.message, 'message', 'nothing is playing'),
      ),
    );
    await expectLater(
      client.seek(Duration.zero),
      throwsA(
        isA<PlayerException>().having(
          (e) => e.isPermissionDenied,
          'locked',
          true,
        ),
      ),
    );
    // A method the player does not have is how an older one answers.
    await expectLater(
      client.listLightShows(),
      throwsA(
        isA<PlayerException>().having(
          (e) => e.code,
          'code',
          GrpcCode.unimplemented,
        ),
      ),
    );
    await channel.dispose();
  });

  test('a call that is not answered in time times out', () async {
    channel = await open();
    final started = Stopwatch()..start();
    await expectLater(
      channel.call(
        'Next',
        Uint8List(0),
        timeout: const Duration(milliseconds: 200),
      ),
      throwsA(
        isA<PlayerException>().having((e) => e.isTimeout, 'timeout', true),
      ),
    );
    expect(started.elapsed, lessThan(const Duration(seconds: 2)));
    expect(channel.callsInFlight, 0);
    await channel.dispose();
  });

  test('a server stream delivers its messages in order and ends', () async {
    channel = await open();
    final messages = <int>[];
    final done = Completer<void>();
    channel
        .openStream(
          'WatchStage',
          pp.WatchStageRequest(maxRate: 0).writeToBuffer(),
        )
        .listen((m) => messages.add(m.length), onDone: done.complete);
    while (service.stages.isEmpty) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    service.stages.single
      ..add(Uint8List(3))
      ..add(Uint8List(5));
    await service.stages.single.close();
    await done.future.timeout(const Duration(seconds: 2));
    expect(messages, [3, 5]);
    expect(service.stageRequests.single.maxRate, 0);
    await channel.dispose();
  });

  test('cancelling a stream cancels the call at the player', () async {
    channel = await open();
    final sub = channel.openStream('WatchStage', Uint8List(0)).listen((_) {});
    while (service.stages.isEmpty) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    await sub.cancel();
    final until = DateTime.now().add(const Duration(seconds: 2));
    while (service.stagesCancelled == 0 && DateTime.now().isBefore(until)) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    expect(service.stagesCancelled, 1);
    // The channel is as good as before.
    expect(channel.currentState, TransportState.ready);
    expect((await PlayerClient(channel).getNetworkInfo()).hostname, 'stage');
    await channel.dispose();
  });

  test('a stream that the player refuses ends with its code', () async {
    channel = await open();
    final errors = <Object>[];
    final done = Completer<void>();
    channel
        .openStream('GetLightingAsset', Uint8List(0))
        .listen((_) {}, onError: errors.add, onDone: done.complete);
    await done.future.timeout(const Duration(seconds: 2));
    expect(errors.single, isA<PlayerException>());
    expect((errors.single as PlayerException).isStreamTooOld, isTrue);
    await channel.dispose();
  });

  test('disposing ends open streams, fails nothing silently, and refuses '
      'new calls', () async {
    channel = await open();
    final errors = <Object>[];
    final done = Completer<void>();
    channel
        .openStream('WatchStage', Uint8List(0))
        .listen((_) {}, onError: errors.add, onDone: done.complete);
    while (service.stages.isEmpty) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    await channel.dispose();
    await done.future.timeout(const Duration(seconds: 2));
    expect((errors.single as PlayerException).isUnavailable, isTrue);
    await expectLater(
      channel.call('Play', Uint8List(0)),
      throwsA(
        isA<PlayerException>().having((e) => e.isUnavailable, 'down', true),
      ),
    );
  });

  test('a link that is cut is noticed at once and is final', () async {
    final proxy = await Proxy.start(server.port!);
    channel = await GrpcChannel.connect('127.0.0.1', proxy.port);
    final states = <TransportState>[];
    channel.state.listen(states.add);
    proxy.cut();
    await waitFor(() => channel.currentState != TransportState.ready);
    expect(channel.currentState, TransportState.disconnected);
    await pumpEventQueue();
    expect(states, [TransportState.disconnected]);
    await expectLater(
      channel.call('Play', Uint8List(0)),
      throwsA(
        isA<PlayerException>().having((e) => e.isUnavailable, 'down', true),
      ),
    );
    await channel.dispose();
    await proxy.close();
  });

  test('a link that goes silent is found by the heartbeat', () async {
    final proxy = await Proxy.start(server.port!);
    channel = await GrpcChannel.connect(
      '127.0.0.1',
      proxy.port,
      beat: const Duration(milliseconds: 100),
      beatTimeout: const Duration(milliseconds: 200),
    );
    expect(channel.currentState, TransportState.ready);
    proxy.blackhole();
    await waitFor(() => channel.currentState != TransportState.ready);
    expect(channel.currentState, TransportState.disconnected);
    await channel.dispose();
    await proxy.close();
  });

  test('nothing listening is UNAVAILABLE, and quickly', () async {
    // A port that was open a moment ago and has nothing on it now.
    final socket = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final port = socket.port;
    await socket.close();
    final started = Stopwatch()..start();
    await expectLater(
      GrpcChannel.connect('127.0.0.1', port),
      throwsA(
        isA<PlayerException>().having((e) => e.isUnavailable, 'down', true),
      ),
    );
    expect(
      started.elapsed,
      lessThan(GrpcChannel.connectTimeout + const Duration(seconds: 1)),
    );
  });

  test('something that accepts the connection and says nothing times out '
      'within the connect timeout', () async {
    final silent = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final held = <Socket>[];
    silent.listen(held.add);
    final started = Stopwatch()..start();
    await expectLater(
      GrpcChannel.connect(
        '127.0.0.1',
        silent.port,
        timeout: const Duration(milliseconds: 400),
      ),
      throwsA(
        isA<PlayerException>().having((e) => e.isUnavailable, 'down', true),
      ),
    );
    expect(started.elapsed, lessThan(const Duration(seconds: 2)));
    for (final socket in held) {
      socket.destroy();
    }
    await silent.close();
  });

  test(
    'a player that answers WatchStatus with UNIMPLEMENTED is an older one',
    () async {
      final old = Server.create(services: [_NoStatus()]);
      await serve(old);
      try {
        await expectLater(
          GrpcChannel.connect('127.0.0.1', old.port!),
          throwsA(
            isA<PlayerException>().having(
              (e) => e.code,
              'code',
              GrpcCode.unimplemented,
            ),
          ),
        );
      } finally {
        await old.shutdown();
      }
    },
  );
}

/// A player that serves a method or two, but not `WatchStatus`.
class _NoStatus extends Service {
  _NoStatus() {
    $addMethod(
      ServiceMethod<List<int>, List<int>>(
        'GetNetworkInfo',
        (ServiceCall call, Future<List<int>> request) async => <int>[],
        false,
        false,
        (bytes) => bytes,
        (bytes) => bytes,
      ),
    );
  }

  @override
  String get $name => 'player.v1.PlayerService';
}
