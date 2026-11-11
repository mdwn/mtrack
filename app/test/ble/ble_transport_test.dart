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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/ble/ble_transport.dart';
import 'package:universal_ble/universal_ble.dart';

BleCharacteristic notify(String uuid) =>
    BleCharacteristic(uuid, [CharacteristicProperty.notify], []);

void main() {
  // What discovery reports is all that can be checked without a radio: the
  // rest of the transport runs only on a phone.
  group('the stream characteristic in a discovery', () {
    test('is found in the player service', () {
      expect(
        BlePlayerTransport.hasStreamCharacteristic([
          BleService('1800', []),
          BleService(serviceUuid, [
            notify(responseUuid),
            notify(statusUuid),
            notify(streamUuid),
          ]),
        ]),
        isTrue,
      );
    });

    test('is found however the platform cases the UUIDs', () {
      expect(
        BlePlayerTransport.hasStreamCharacteristic([
          BleService(serviceUuid.toUpperCase(), [
            notify(streamUuid.toUpperCase()),
          ]),
        ]),
        isTrue,
      );
    });

    test('is missing from a player from before streams', () {
      expect(
        BlePlayerTransport.hasStreamCharacteristic([
          BleService(serviceUuid, [notify(responseUuid), notify(statusUuid)]),
        ]),
        isFalse,
      );
    });

    test('does not count under another service, or with none at all', () {
      expect(
        BlePlayerTransport.hasStreamCharacteristic([
          BleService('6d747261-636b-0009-8000-00805f9b34fb', [
            notify(streamUuid),
          ]),
        ]),
        isFalse,
      );
      expect(BlePlayerTransport.hasStreamCharacteristic([]), isFalse);
    });
  });
}
