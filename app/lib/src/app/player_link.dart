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
import '../protocol/transport.dart';

/// A player seen while scanning.
class DiscoveredPlayer {
  /// A player advertising the service.
  const DiscoveredPlayer({
    required this.id,
    required this.name,
    required this.rssi,
  });

  /// The platform's identifier for the peripheral: a MAC address on Android, a
  /// per-phone UUID on iOS. Stable enough to reconnect to.
  final String id;

  /// The name the player advertises.
  final String name;

  /// Signal strength in dBm at the last advertisement; 0 when unknown.
  final int rssi;

  /// A coarse reading of [rssi] for people: strong, good or weak.
  String get signalLabel {
    if (rssi == 0) {
      return '';
    }
    if (rssi >= -60) {
      return 'strong';
    }
    if (rssi >= -80) {
      return 'good';
    }
    return 'weak';
  }
}

/// Whether the phone's Bluetooth can be used.
enum AdapterState {
  /// Bluetooth is on and the app may use it.
  ready,

  /// Bluetooth is switched off.
  off,

  /// The user has not allowed the app to use Bluetooth.
  permissionDenied,

  /// The phone has no Bluetooth Low Energy.
  unsupported,
}

/// The player refused to pair: its pairing window is closed, or the phone
/// declined. The player drops a phone that stays connected without pairing.
class PairingRefusedException implements Exception {
  /// A refusal, with [detail] from the platform for diagnosis.
  const PairingRefusedException([this.detail = '']);

  /// What the platform said, if anything. Not shown to people.
  final String detail;

  @override
  String toString() => 'PairingRefusedException($detail)';
}

/// Everything the app needs of the radio beyond moving chunks, which
/// [PlayerTransport] covers. The real implementation is in `lib/src/ble/`;
/// tests use a fake.
abstract class PlayerLink {
  /// Whether Bluetooth can be used now.
  Future<AdapterState> adapterState();

  /// Asks the user for the permissions Bluetooth needs, and says where that
  /// leaves the adapter.
  Future<AdapterState> requestAccess();

  /// Scans for players until [timeout] passes or the subscription is
  /// cancelled. Each player is emitted again when its signal is heard again.
  Stream<DiscoveredPlayer> scan({
    Duration timeout = const Duration(seconds: 10),
  });

  /// Opens a link to the player [id]. The transport that comes back is
  /// connecting: its link is up, but it is not ready for calls until
  /// [subscribe] has been called, which happens after pairing.
  Future<PlayerTransport> connect(String id);

  /// Whether this phone is already paired with [id].
  Future<bool> isPaired(String id);

  /// Pairs with [id], which is connected. Throws [PairingRefusedException]
  /// when the player does not accept the phone.
  Future<void> pair(String id);

  /// Subscribes to the connected player's notifications, which moves its
  /// transport to ready.
  Future<void> subscribe(String id);
}
