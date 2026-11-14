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

import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import 'fake_player.dart';
import 'rig.dart';

/// A [WifiLink] to the players of a [Rig]: the hosts in [reachable] answer
/// and the rest do not, as a network does.
class FakeWifi implements WifiLink {
  /// A network whose one player is [rig]'s.
  FakeWifi(this.rig);

  /// The player every reachable host leads to.
  final Rig rig;

  /// The hosts that answer. Any other host is silent until the timeout.
  final Set<String> reachable = {};

  /// When set, a connection is refused with this: what a player from before
  /// Wi-Fi does (`UNIMPLEMENTED`), or a port with nothing behind it.
  PlayerException? refusal;

  /// Every connection attempted, as `host:port`, in order.
  final List<String> attempts = [];

  /// Every channel opened, in order.
  final List<FakeWifiChannel> channels = [];

  /// How long a connection that answers takes.
  Duration latency = Duration.zero;

  /// When set, what `GetNetworkInfo` answers over Wi-Fi in place of the
  /// rig's: another device at the address, or a player renamed.
  pp.GetNetworkInfoResponse? networkInfo;

  /// Per host, a refusal in place of [refusal], and what `GetNetworkInfo`
  /// answers in place of [networkInfo].
  final Map<String, PlayerException> refusals = {};
  final Map<String, pp.GetNetworkInfoResponse> infos = {};

  /// The channel most recently opened that is still up.
  FakeWifiChannel? get live =>
      channels.where((c) => c.currentState == TransportState.ready).lastOrNull;

  @override
  Future<PlayerChannel> connect(
    String host,
    int port, {
    Duration timeout = const Duration(seconds: 2),
  }) async {
    attempts.add('$host:$port');
    if (!reachable.contains(host)) {
      await Future<void>.delayed(timeout);
      throw PlayerException.unavailable('no answer from $host');
    }
    if (latency > Duration.zero) {
      await Future<void>.delayed(latency);
    }
    final refused = refusals[host] ?? refusal;
    if (refused != null) {
      throw refused;
    }
    final channel = FakeWifiChannel(
      rig,
      networkInfo: infos[host] ?? networkInfo,
    );
    channels.add(channel);
    return channel;
  }
}

/// A [PlayerChannel] on the Wi-Fi carrier that answers from a [Rig]'s
/// handlers, as the player would over gRPC: nothing is on an allowlist.
class FakeWifiChannel implements PlayerChannel {
  /// A channel to [rig]'s player, up.
  FakeWifiChannel(this.rig, {this.networkInfo}) {
    rig.watchers.add(_onStatus);
  }

  /// What `GetNetworkInfo` answers, in place of the rig's.
  final pp.GetNetworkInfoResponse? networkInfo;

  /// The player.
  final Rig rig;

  /// What the player was asked for over this channel, in order. (The rig's
  /// own record, which Bluetooth fills too, has these as well.)
  final List<String> calls = [];

  /// Every stream opened, with the controller that feeds it.
  final List<
    ({String method, Uint8List request, StreamController<Uint8List> feed})
  >
  opened = [];

  /// Set to make a method never answer.
  final Set<String> silent = {};

  int _inFlight = 0;
  TransportState _state = TransportState.ready;
  final StreamController<TransportState> _states =
      StreamController<TransportState>.broadcast();
  final StreamController<pp.RemoteStatus> _statuses =
      StreamController<pp.RemoteStatus>.broadcast();

  void _onStatus(pp.RemoteStatus status) {
    if (_state == TransportState.ready) {
      _statuses.add(status);
    }
  }

  /// Drops the link, as keepalive finding it dead would.
  void drop() {
    if (_state == TransportState.disconnected) {
      return;
    }
    _state = TransportState.disconnected;
    rig.watchers.remove(_onStatus);
    _states.add(TransportState.disconnected);
  }

  @override
  Carrier get carrier => Carrier.wifi;

  @override
  TransportState get currentState => _state;

  @override
  Stream<TransportState> get state => _states.stream;

  @override
  bool get hasStreams => true;

  @override
  int get callsInFlight => _inFlight;

  @override
  Stream<pp.RemoteStatus> get statusStream => _statuses.stream;

  @override
  Future<pp.RemoteStatus> readStatus() async => rig.status;

  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    if (_state != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    calls.add(method);
    rig.player.received.add(ble.Request(method: method, payload: request));
    _inFlight++;
    try {
      if (silent.contains(method)) {
        await Future<void>.delayed(timeout);
        throw PlayerException.timeout(method);
      }
      final handler = rig.player.handlers[method];
      final reply = method == 'GetNetworkInfo' && networkInfo != null
          ? FakeReply.ok(networkInfo!)
          : handler == null
          ? FakeReply.empty()
          : handler(ble.Request(method: method, payload: request));
      if (reply.drop) {
        await Future<void>.delayed(timeout);
        throw PlayerException.timeout(method);
      }
      if (reply.delay > Duration.zero) {
        await Future<void>.delayed(reply.delay);
      }
      if (reply.code != 0) {
        throw PlayerException(reply.code, reply.text);
      }
      return Uint8List.fromList(reply.payload);
    } finally {
      _inFlight--;
    }
  }

  @override
  Stream<Uint8List> openStream(String method, Uint8List request) {
    late final StreamController<Uint8List> feed;
    feed = StreamController<Uint8List>(sync: true);
    opened.add((method: method, request: request, feed: feed));
    return feed.stream;
  }

  @override
  Future<void> dispose() async {
    drop();
    for (final stream in opened) {
      if (!stream.feed.isClosed) {
        unawaited(stream.feed.close());
      }
    }
  }
}
