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

import 'package:universal_ble/universal_ble.dart';

import '../app/player_link.dart';
import '../protocol/transport.dart';
import 'ble_transport.dart';

/// The real [PlayerLink], over `universal_ble`. It adapts the plugin to the
/// app's seam and decides nothing; nothing here has run on a device (see
/// README.md for what is unverified).
class BlePlayerLink implements PlayerLink {
  /// A link that has no connection yet.
  BlePlayerLink();

  BlePlayerTransport? _transport;
  String? _transportId;

  @override
  Future<AdapterState> adapterState() async {
    final AvailabilityState state;
    try {
      state = await UniversalBle.getBluetoothAvailabilityState();
    } on Object {
      return AdapterState.ready;
    }
    switch (state) {
      case AvailabilityState.poweredOff:
        return AdapterState.off;
      case AvailabilityState.unauthorized:
        return AdapterState.permissionDenied;
      case AvailabilityState.unsupported:
        return AdapterState.unsupported;
      case AvailabilityState.unknown:
      case AvailabilityState.resetting:
      case AvailabilityState.poweredOn:
        // Unknown and resetting settle in moments; a scan or connect that
        // cannot go ahead says so by failing.
        return AdapterState.ready;
    }
  }

  @override
  Future<AdapterState> requestAccess() async {
    try {
      await UniversalBle.requestPermissions();
    } on Object {
      return AdapterState.permissionDenied;
    }
    return adapterState();
  }

  @override
  Stream<DiscoveredPlayer> scan({
    Duration timeout = const Duration(seconds: 10),
  }) {
    late final StreamController<DiscoveredPlayer> controller;
    StreamSubscription<BleDevice>? results;
    Timer? ender;

    Future<void> stop() async {
      ender?.cancel();
      await results?.cancel();
      try {
        await UniversalBle.stopScan();
      } on Object {
        // The scan was not running, or the radio went off.
      }
    }

    controller = StreamController<DiscoveredPlayer>(
      onListen: () async {
        results = UniversalBle.scanStream.listen((device) {
          final name = device.name;
          controller.add(
            DiscoveredPlayer(
              id: device.deviceId,
              name: name != null && name.isNotEmpty ? name : device.deviceId,
              rssi: device.rssi ?? 0,
            ),
          );
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
  Future<PlayerTransport> connect(String id) async {
    await _transport?.close();
    final transport = BlePlayerTransport();
    _transport = transport;
    _transportId = id;
    await transport.connect(id);
    return transport;
  }

  BlePlayerTransport _connected(String id) {
    final transport = _transport;
    if (transport == null || _transportId != id) {
      throw StateError('not connected to $id');
    }
    return transport;
  }

  @override
  Future<bool> isPaired(String id) async {
    _connected(id);
    final paired = await UniversalBle.isPaired(
      id,
      pairingCommand: BleCommand(
        service: serviceUuid,
        characteristic: statusUuid,
      ),
    );
    return paired ?? false;
  }

  @override
  Future<void> pair(String id) async {
    final transport = _connected(id);
    try {
      await transport.pair();
    } on Object catch (e) {
      throw PairingRefusedException('$e');
    }
  }

  @override
  Future<void> subscribe(String id) => _connected(id).subscribe();
}
