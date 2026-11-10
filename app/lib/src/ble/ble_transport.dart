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

/// A player seen while scanning.
class DiscoveredPlayer {
  /// A player advertising the service.
  const DiscoveredPlayer({
    required this.id,
    required this.name,
    required this.rssi,
  });

  /// The platform's identifier for the peripheral: a MAC address on Android, a
  /// per-phone UUID on iOS. Stable enough to reconnect to with
  /// [BlePlayerTransport.connect].
  final String id;

  /// The name the player advertises.
  final String name;

  /// Signal strength in dBm at the last advertisement.
  final int rssi;
}

/// A [PlayerTransport] over Bluetooth LE, through `universal_ble`.
///
/// This is the only file that touches the plugin. It moves chunks and reports
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

  /// Scans for players advertising the service, emitting the full list of
  /// those seen so far each time it changes, until the scan ends after
  /// [timeout] or the subscription is cancelled. Asks for the Bluetooth
  /// permissions first on Android and iOS.
  static Stream<List<DiscoveredPlayer>> scan({
    Duration timeout = const Duration(seconds: 10),
  }) {
    late final StreamController<List<DiscoveredPlayer>> controller;
    final found = <String, DiscoveredPlayer>{};
    StreamSubscription<BleDevice>? results;
    Timer? ender;

    Future<void> stop() async {
      ender?.cancel();
      await results?.cancel();
      await UniversalBle.stopScan();
    }

    controller = StreamController<List<DiscoveredPlayer>>(
      onListen: () async {
        results = UniversalBle.scanStream.listen((device) {
          final name = device.name;
          found[device.deviceId] = DiscoveredPlayer(
            id: device.deviceId,
            name: name != null && name.isNotEmpty ? name : device.deviceId,
            rssi: device.rssi ?? 0,
          );
          controller.add(found.values.toList(growable: false));
        });
        try {
          await UniversalBle.startScan(
            scanFilter: ScanFilter(withServices: [serviceUuid]),
          );
        } on Object catch (e, stack) {
          controller.addError(e, stack);
          await controller.close();
          return;
        }
        ender = Timer(timeout, () => unawaited(controller.close()));
      },
      onCancel: stop,
    );
    return controller.stream;
  }

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
  /// the MTU, discovers the service and subscribes to `response` and `status`,
  /// then reports [TransportState.ready].
  ///
  /// Throws when any step fails, leaving the transport disconnected. Pairing
  /// is left to the operating system, which starts it on the first encrypted
  /// access; [pair] starts it deliberately.
  Future<void> connect(String deviceId) async {
    await _teardown();
    _setState(TransportState.connecting);
    _deviceId = deviceId;
    try {
      final lost = Completer<void>();
      _subscriptions.add(
        UniversalBle.connectionStream(deviceId).listen((connected) {
          if (!connected) {
            if (!lost.isCompleted) {
              lost.complete();
            }
            _onLinkLost();
          }
        }),
      );
      await UniversalBle.connect(deviceId);
      _mtu = await UniversalBle.requestMtu(deviceId, requestedMtu);
      await UniversalBle.discoverServices(deviceId);

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
      if (lost.isCompleted) {
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
