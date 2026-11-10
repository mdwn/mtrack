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

import 'package:flutter/foundation.dart';
import 'package:universal_ble/universal_ble.dart';

import '../protocol/transport.dart';

/// The player's GATT service.
const String serviceUuid = '6d747261-636b-0001-8000-00805f9b34fb';

/// Where requests are written: write, encrypted link.
const String requestUuid = '6d747261-636b-0002-8000-00805f9b34fb';

/// Where responses are notified.
const String responseUuid = '6d747261-636b-0003-8000-00805f9b34fb';

/// Where the status is read and notified: encrypted link.
const String statusUuid = '6d747261-636b-0004-8000-00805f9b34fb';

/// The ATT MTU to ask for, the most Android allows. iOS negotiates its own and
/// reports what it got.
const int requestedMtu = 517;

/// The most bytes an attribute value can hold, whatever the MTU. A chunk
/// larger than this cannot be written.
const int maxAttributeValue = 512;

/// A [PlayerTransport] over Bluetooth LE, through `universal_ble`.
///
/// With `ble_link.dart` this is where the app touches the plugin. It moves chunks and reports
/// the link's state and nothing else; framing, requests and timeouts live in
/// the protocol layer where they can be tested. Nothing here has run on a
/// device (see README.md for what is unverified).
class BlePlayerTransport implements PlayerTransport {
  /// A transport that is not connected to anything yet.
  BlePlayerTransport();

  final StreamController<TransportState> _state =
      StreamController<TransportState>.broadcast();
  final StreamController<Uint8List> _responses =
      StreamController<Uint8List>.broadcast();
  final StreamController<Uint8List> _statuses =
      StreamController<Uint8List>.broadcast();
  final List<StreamSubscription<Object?>> _subscriptions = [];

  TransportState _currentState = TransportState.disconnected;
  String? _deviceId;
  int _mtu = 23;
  bool _linkLost = false;

  @override
  TransportState get currentState => _currentState;

  @override
  Stream<TransportState> get state => _state.stream;

  @override
  int get maxChunk => min(_mtu - 3, maxAttributeValue);

  @override
  Stream<Uint8List> get responseChunks => _responses.stream;

  @override
  Stream<Uint8List> get statusChunks => _statuses.stream;

  void _setState(TransportState next) {
    if (_currentState == next) {
      return;
    }
    _currentState = next;
    if (!_state.isClosed) {
      _state.add(next);
    }
  }

  /// Connects to the player with the platform identifier [deviceId], requests
  /// the MTU and discovers the service. The transport stays
  /// [TransportState.connecting]: the player drops a phone that stays
  /// connected without pairing, so pairing comes next, and [subscribe]
  /// finishes the setup.
  ///
  /// Throws when any step fails, leaving the transport disconnected.
  Future<void> connect(String deviceId) async {
    await _teardown();
    _setState(TransportState.connecting);
    _deviceId = deviceId;
    try {
      _subscriptions.add(
        UniversalBle.connectionStream(deviceId).listen((connected) {
          if (!connected) {
            _linkLost = true;
            _onLinkLost();
          }
        }),
      );
      await UniversalBle.connect(deviceId);
      _mtu = await UniversalBle.requestMtu(deviceId, requestedMtu);
      await _askForShortInterval(deviceId);
      await UniversalBle.discoverServices(deviceId);
    } on Object {
      await _teardown();
      rethrow;
    }
  }

  /// Asks Android for its low-latency connection interval (about 7.5 to
  /// 15 ms in place of 30 to 50), so a tap is answered within a frame or two.
  /// It costs battery on both ends while connected, which a remote in use
  /// can afford. Only Android has the call: elsewhere, and when the phone
  /// refuses, the link keeps the interval it has.
  Future<void> _askForShortInterval(String deviceId) async {
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) {
      return;
    }
    try {
      await UniversalBle.requestConnectionPriority(
        deviceId,
        BleConnectionPriority.highPerformance,
      );
    } on Object {
      // A hint, not a need.
    }
  }

  /// Subscribes to `response` and `status`, then reports
  /// [TransportState.ready]. The characteristics need an encrypted link, so
  /// this comes after pairing. Throws when a step fails, leaving the
  /// transport disconnected.
  Future<void> subscribe() async {
    final deviceId = _deviceId;
    if (deviceId == null || _linkLost) {
      throw StateError('not connected');
    }
    try {
      // Listen before subscribing, so the status the player sends on
      // subscribe is not missed.
      _subscriptions
        ..add(
          UniversalBle.characteristicValueStream(
            deviceId,
            responseUuid,
          ).listen((value) => _responses.add(Uint8List.fromList(value))),
        )
        ..add(
          UniversalBle.characteristicValueStream(
            deviceId,
            statusUuid,
          ).listen((value) => _statuses.add(Uint8List.fromList(value))),
        );
      await UniversalBle.subscribeNotifications(
        deviceId,
        serviceUuid,
        responseUuid,
      );
      await UniversalBle.subscribeNotifications(
        deviceId,
        serviceUuid,
        statusUuid,
      );
      if (_linkLost) {
        throw StateError('connection lost during setup');
      }
      _setState(TransportState.ready);
    } on Object {
      await _teardown();
      rethrow;
    }
  }

  /// Starts pairing with the connected player.
  ///
  /// Android has a system pairing call and is asked to bond. Elsewhere pairing
  /// is started by the operating system when an encrypted characteristic is
  /// accessed, so this reads `status`, which is one.
  Future<void> pair() async {
    final deviceId = _deviceId;
    if (deviceId == null) {
      throw StateError('not connected');
    }
    await UniversalBle.pair(
      deviceId,
      pairingCommand: BleCommand(
        service: serviceUuid,
        characteristic: statusUuid,
      ),
    );
  }

  void _onLinkLost() {
    _setState(TransportState.disconnected);
  }

  Future<void> _teardown() async {
    _linkLost = false;
    for (final subscription in _subscriptions) {
      await subscription.cancel();
    }
    _subscriptions.clear();
    final deviceId = _deviceId;
    _deviceId = null;
    if (deviceId != null) {
      try {
        await UniversalBle.disconnect(deviceId);
      } on Object {
        // Already gone, or the radio is off: the aim was to be disconnected.
      }
    }
    _setState(TransportState.disconnected);
  }

  @override
  Future<void> write(Uint8List chunk) async {
    final deviceId = _deviceId;
    if (deviceId == null || _currentState != TransportState.ready) {
      throw StateError('not connected');
    }
    await UniversalBle.write(
      deviceId,
      serviceUuid,
      requestUuid,
      chunk,
      withoutResponse: false,
    );
  }

  @override
  Future<Uint8List> readStatus() async {
    final deviceId = _deviceId;
    if (deviceId == null || _currentState != TransportState.ready) {
      throw StateError('not connected');
    }
    return Uint8List.fromList(
      await UniversalBle.read(deviceId, serviceUuid, statusUuid),
    );
  }

  @override
  Future<void> close() async {
    await _teardown();
    await _state.close();
    await _responses.close();
    await _statuses.close();
  }
}
