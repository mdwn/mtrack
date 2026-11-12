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

import 'package:grpc/grpc.dart';
import 'package:protobuf/protobuf.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import 'player_channel.dart';
import 'player_exception.dart';
import 'transport.dart';

/// Carries the player's `PlayerService` calls as gRPC over the network, on
/// the player's own gRPC port: plaintext HTTP/2, which is what that port
/// speaks. Wi-Fi is open to whoever is on the player's network, as its web UI
/// is; see the remote design, section 4.1.
///
/// The messages are the ones the Bluetooth envelope carries, so the calls
/// here are made without generated client stubs: a method is a path and its
/// bytes. gRPC status codes arrive as the codes `PlayerException` carries.
///
/// A channel is opened with [connect], which does not return until the player
/// has answered `WatchStatus` with its first status. That one stream then
/// stays open for as long as the channel does: it is the app's single path
/// for status, and when it ends, or the heartbeat finds the link dead, the
/// channel is down. A down channel stays down.
class GrpcChannel implements PlayerChannel {
  GrpcChannel._(
    this._channel,
    this.host,
    this.port,
    this._beat,
    this._beatTimeout,
  );

  /// The service's path in gRPC.
  static const String servicePath = '/player.v1.PlayerService/';

  /// How long to wait for a player to answer on first contact. Short, since
  /// falling back to Bluetooth waits for it.
  static const Duration connectTimeout = Duration(seconds: 2);

  /// How often a connected channel asks the player a cheap question to see
  /// that it is still there, and how long it waits for the answer before it
  /// calls the link dead. The status stream is silent while a stopped player
  /// has nothing new to say, and a link that goes away (the phone walks out
  /// of range) is not known to TCP for minutes, so without this a dead link
  /// would be found only by the next thing the person tried. (gRPC's own
  /// keepalive is not used: in this version its timeout closes the
  /// connection politely, which waits for the streams that will never end.)
  static const Duration heartbeatInterval = Duration(seconds: 8);

  /// See [heartbeatInterval].
  static const Duration heartbeatTimeout = Duration(seconds: 5);

  /// Opens a channel to the player at [host] and [port] and waits for its
  /// first status. Throws a [PlayerException] when nothing answers within
  /// [timeout] (`UNAVAILABLE`), or when the player answers with an error: an
  /// older player without `WatchStatus` says `UNIMPLEMENTED`.
  ///
  /// [beat] and [beatTimeout] are for tests: how fast a dead link is found.
  static Future<GrpcChannel> connect(
    String host,
    int port, {
    Duration timeout = connectTimeout,
    Duration beat = heartbeatInterval,
    Duration beatTimeout = heartbeatTimeout,
  }) async {
    final made = ClientChannel(
      host,
      port: port,
      options: ChannelOptions(
        credentials: const ChannelCredentials.insecure(),
        connectTimeout: timeout,
        // Never closed for being idle: the status stream is always open.
        idleTimeout: null,
      ),
    );
    final result = GrpcChannel._(made, host, port, beat, beatTimeout);
    try {
      await result._start(timeout);
    } on Object {
      await result.dispose();
      rethrow;
    }
    return result;
  }

  /// The host the channel was opened to.
  final String host;

  /// The port.
  final int port;

  final ClientChannel _channel;
  final Duration _beat;
  final Duration _beatTimeout;
  Timer? _heartbeat;
  bool _beating = false;
  final StreamController<TransportState> _stateController =
      StreamController<TransportState>.broadcast();
  final StreamController<player.RemoteStatus> _statusController =
      StreamController<player.RemoteStatus>.broadcast();
  final Set<StreamController<Uint8List>> _streams = {};

  TransportState _state = TransportState.connecting;
  player.RemoteStatus? _latest;
  StreamSubscription<List<int>>? _statusSubscription;
  StreamSubscription<ConnectionState>? _connectionSubscription;
  bool _terminated = false;
  int _inFlight = 0;

  @override
  Carrier get carrier => Carrier.wifi;

  @override
  TransportState get currentState => _state;

  @override
  Stream<TransportState> get state => _stateController.stream;

  @override
  bool get hasStreams => true;

  @override
  int get callsInFlight => _inFlight;

  @override
  Stream<player.RemoteStatus> get statusStream => _statusController.stream;

  @override
  Future<player.RemoteStatus> readStatus() async {
    final latest = _latest;
    if (latest == null) {
      throw const PlayerException.unavailable();
    }
    return latest;
  }

  ClientMethod<List<int>, List<int>> _method(String name) =>
      ClientMethod<List<int>, List<int>>(
        '$servicePath$name',
        (bytes) => bytes,
        (bytes) => bytes,
      );

  /// Opens `WatchStatus` and takes its first message.
  Future<void> _start(Duration timeout) async {
    final first = Completer<player.RemoteStatus>();
    final call = _channel.createCall<List<int>, List<int>>(
      _method('WatchStatus'),
      Stream.value(player.WatchStatusRequest().writeToBuffer()),
      CallOptions(),
    );
    _statusSubscription = call.response.listen(
      (bytes) {
        final player.RemoteStatus status;
        try {
          status = player.RemoteStatus.fromBuffer(bytes);
        } on InvalidProtocolBufferException {
          return;
        }
        _latest = status;
        if (!first.isCompleted) {
          first.complete(status);
        } else if (!_statusController.isClosed) {
          _statusController.add(status);
        }
      },
      onError: (Object error) {
        final failure = _map(error, 'WatchStatus');
        if (!first.isCompleted) {
          first.completeError(failure);
        }
        _goDown();
      },
      onDone: () {
        if (!first.isCompleted) {
          first.completeError(
            const PlayerException.unavailable('the player ended the stream'),
          );
        }
        _goDown();
      },
      cancelOnError: true,
    );
    try {
      await first.future.timeout(timeout);
    } on TimeoutException {
      throw PlayerException.unavailable(
        'no answer from $host:$port within ${timeout.inSeconds} s',
      );
    }
    _setState(TransportState.ready);
    _heartbeat = Timer.periodic(_beat, (_) => unawaited(_beatOnce()));
    // A connection the transport knows is gone ends the status stream; this is
    // only quicker when it says so first.
    _connectionSubscription = _channel.onConnectionStateChanged.listen((s) {
      if (s == ConnectionState.transientFailure ||
          s == ConnectionState.shutdown) {
        _goDown();
      }
    });
  }

  /// Asks the player something cheap that it answers even when locked
  /// (`GetNetworkInfo`), and takes silence or a broken connection for a dead
  /// link. Any other answer, an error included, shows it is there.
  Future<void> _beatOnce() async {
    if (_state != TransportState.ready || _beating) {
      return;
    }
    _beating = true;
    try {
      await _invoke('GetNetworkInfo', Uint8List(0), _beatTimeout);
    } on PlayerException catch (e) {
      if (e.isTimeout || e.isUnavailable) {
        _goDown();
      }
    } finally {
      _beating = false;
    }
  }

  void _setState(TransportState next) {
    if (_state == next || _stateController.isClosed) {
      return;
    }
    _state = next;
    _stateController.add(next);
  }

  /// The link is gone: nothing more will come over it.
  void _goDown() {
    if (_state == TransportState.disconnected) {
      return;
    }
    _setState(TransportState.disconnected);
    unawaited(
      _releaseStreams(const PlayerException.unavailable('connection lost')),
    );
  }

  Future<void> _releaseStreams(PlayerException error) async {
    for (final controller in _streams.toList()) {
      if (!controller.isClosed) {
        controller.addError(error);
        unawaited(controller.close());
      }
    }
    _streams.clear();
    await _terminate();
  }

  Future<void> _terminate() {
    if (_terminated) {
      return Future<void>.value();
    }
    _terminated = true;
    return _channel.terminate();
  }

  /// A gRPC failure as the exception the app knows.
  PlayerException _map(Object error, String method) {
    if (error is PlayerException) {
      return error;
    }
    if (error is GrpcError) {
      if (error.code == StatusCode.deadlineExceeded) {
        return PlayerException.timeout(method);
      }
      return PlayerException(error.code, error.message ?? '');
    }
    return PlayerException.unavailable('$error');
  }

  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    _inFlight++;
    try {
      return await _invoke(method, request, timeout);
    } finally {
      _inFlight--;
    }
  }

  Future<Uint8List> _invoke(
    String method,
    Uint8List request,
    Duration timeout,
  ) async {
    if (_state != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    try {
      final response = await ResponseFuture<List<int>>(
        _channel.createCall<List<int>, List<int>>(
          _method(method),
          Stream.value(request),
          CallOptions(timeout: timeout),
        ),
      );
      return response is Uint8List ? response : Uint8List.fromList(response);
    } on Object catch (e) {
      throw _map(e, method);
    }
  }

  @override
  Stream<Uint8List> openStream(String method, Uint8List request) {
    ClientCall<List<int>, List<int>>? call;
    StreamSubscription<List<int>>? subscription;
    late final StreamController<Uint8List> controller;
    controller = StreamController<Uint8List>(
      sync: true,
      onListen: () {
        if (_state != TransportState.ready) {
          // Not from inside the listen call: the listener is not set up yet.
          scheduleMicrotask(() {
            controller.addError(const PlayerException.unavailable());
            unawaited(controller.close());
          });
          return;
        }
        _streams.add(controller);
        call = _channel.createCall<List<int>, List<int>>(
          _method(method),
          Stream.value(request),
          CallOptions(),
        );
        subscription = call!.response.listen(
          (bytes) => controller.add(
            bytes is Uint8List ? bytes : Uint8List.fromList(bytes),
          ),
          onError: (Object error) {
            _streams.remove(controller);
            controller.addError(_map(error, method));
            unawaited(controller.close());
          },
          onDone: () {
            _streams.remove(controller);
            unawaited(controller.close());
          },
          cancelOnError: true,
        );
      },
      onPause: () => subscription?.pause(),
      onResume: () => subscription?.resume(),
      onCancel: () async {
        _streams.remove(controller);
        await subscription?.cancel();
        await call?.cancel();
      },
    );
    return controller.stream;
  }

  @override
  Future<void> dispose() async {
    final alreadyDown = _state == TransportState.disconnected;
    _heartbeat?.cancel();
    _heartbeat = null;
    _setState(TransportState.disconnected);
    final subscription = _statusSubscription;
    _statusSubscription = null;
    unawaited(subscription?.cancel());
    unawaited(_connectionSubscription?.cancel());
    _connectionSubscription = null;
    if (!alreadyDown) {
      await _releaseStreams(const PlayerException.unavailable('closed'));
    } else {
      await _terminate();
    }
    await _stateController.close();
    await _statusController.close();
  }
}
