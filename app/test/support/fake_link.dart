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

import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import 'fake_player.dart';

/// A [PlayerLink] that connects to an in-memory [FakePlayer] and records what
/// the session asks of it, in order.
class FakeLink implements PlayerLink {
  /// A link whose player is [player].
  FakeLink(this.player);

  /// The player every connection reaches.
  final FakePlayer player;

  /// Each call, as `name` or `name:id`, in order.
  final List<String> calls = [];

  /// What [adapterState] says.
  AdapterState adapter = AdapterState.ready;

  /// Which players this phone is already paired with.
  final Set<String> paired = {};

  /// True to refuse every pairing.
  bool pairingRefused = false;

  /// How many of the next connects fail.
  int connectFailures = 0;

  /// How long a connect takes.
  Duration connectDelay = Duration.zero;

  /// A pairing that never finishes, until completed.
  Completer<void>? pairingHeld;

  /// Whether the scan is being listened to.
  bool get scanning => _scan?.hasListener ?? false;

  StreamController<DiscoveredPlayer>? _scan;

  /// Sends [player] to a running scan.
  void discover(DiscoveredPlayer player) => _scan!.add(player);

  /// Ends a running scan, as its time running out would.
  Future<void> endScan() => _scan!.close();

  /// Fails a running scan.
  void failScan(Object error) => _scan!.addError(error);

  @override
  Future<AdapterState> adapterState() async {
    calls.add('adapterState');
    return adapter;
  }

  @override
  Future<AdapterState> requestAccess() async {
    calls.add('requestAccess');
    return adapter;
  }

  @override
  Stream<DiscoveredPlayer> scan({
    Duration timeout = const Duration(seconds: 10),
  }) {
    calls.add('scan');
    _scan = StreamController<DiscoveredPlayer>();
    return _scan!.stream;
  }

  @override
  Future<PlayerTransport> connect(String id) async {
    calls.add('connect:$id');
    if (connectDelay > Duration.zero) {
      await Future<void>.delayed(connectDelay);
    }
    if (connectFailures > 0) {
      connectFailures--;
      throw StateError('no such player');
    }
    player.setState(TransportState.connecting);
    return player;
  }

  @override
  Future<bool> isPaired(String id) async {
    calls.add('isPaired:$id');
    return paired.contains(id);
  }

  @override
  Future<void> pair(String id) async {
    calls.add('pair:$id');
    final held = pairingHeld;
    if (held != null) {
      await held.future;
    }
    if (pairingRefused) {
      throw const PairingRefusedException('window closed');
    }
    paired.add(id);
  }

  @override
  Future<void> subscribe(String id) async {
    calls.add('subscribe:$id');
    player.setState(TransportState.ready);
  }
}

/// A [PlayerStore] in memory.
class MemoryStore implements PlayerStore {
  /// A store holding [player], if any.
  MemoryStore([this.player]);

  /// What is remembered.
  RememberedPlayer? player;

  @override
  Future<RememberedPlayer?> load() async => player;

  @override
  Future<void> save(RememberedPlayer player) async => this.player = player;

  @override
  Future<void> clear() async => player = null;
}

/// A [ScreenWake] that records what it was told.
class FakeWake implements ScreenWake {
  /// Every value passed to [setEnabled], in order.
  final List<bool> history = [];

  /// The last value, or false before any.
  bool get enabled => history.isNotEmpty && history.last;

  @override
  Future<void> setEnabled(bool enabled) async => history.add(enabled);
}
