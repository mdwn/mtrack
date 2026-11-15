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

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/platform/interface_prefixes.dart';
import 'package:mtrack_remote/src/platform/io_network_probe.dart';

void main() {
  group('the prefix length of a netmask', () {
    test('counts the ones of a contiguous mask', () {
      expect(prefixLengthOfMask(0xFFFFFFFF), 32);
      expect(prefixLengthOfMask(0xFFFFFFFE), 31);
      expect(prefixLengthOfMask(0xFFFFFFFC), 30);
      expect(prefixLengthOfMask(0xFFFFFF80), 25);
      expect(prefixLengthOfMask(0xFFFFFF00), 24);
      expect(prefixLengthOfMask(0xFFFFFE00), 23);
      expect(prefixLengthOfMask(0xFFFFF000), 20);
      expect(prefixLengthOfMask(0xFFFF0000), 16);
      expect(prefixLengthOfMask(0xFFF00000), 12);
      expect(prefixLengthOfMask(0xFF000000), 8);
      expect(prefixLengthOfMask(0x80000000), 1);
      expect(prefixLengthOfMask(0), 0);
    });

    test('is every prefix from 0 to 32 for its own mask', () {
      for (var n = 0; n <= 32; n++) {
        final mask = n == 0 ? 0 : (0xFFFFFFFF << (32 - n)) & 0xFFFFFFFF;
        expect(prefixLengthOfMask(mask), n, reason: '/$n');
      }
    });

    test('is null for a mask with a gap', () {
      for (final mask in [
        0xFF00FF00,
        0xFFFFFF01,
        0xFEFFFFFF,
        0x00FFFFFF,
        0x00000001,
        0xAAAAAAAA,
      ]) {
        expect(
          prefixLengthOfMask(mask),
          isNull,
          reason: mask.toRadixString(16),
        );
      }
    });

    test('is null for what is not 32 bits', () {
      expect(prefixLengthOfMask(-1), isNull);
      expect(prefixLengthOfMask(0x1FFFFFFFF), isNull);
    });
  });

  group('the real lookup', () {
    test('does not throw, and loopback is a /8 when it is there', () {
      final prefixes = interfacePrefixes();
      for (final entry in prefixes.entries) {
        expect(entry.value, inInclusiveRange(0, 32), reason: entry.key);
      }
      final loopback = prefixes[prefixKey('lo', '127.0.0.1')];
      if (loopback != null) {
        expect(loopback, 8);
      }
    }, skip: Platform.isLinux ? false : 'getifaddrs is read on Linux hosts');

    test(
      'the probe attaches a prefix to the addresses dart:io lists',
      () async {
        final probe = IoNetworkProbe();
        final locals = await probe.localAddresses();
        for (final local in locals.where((l) => l.address == '127.0.0.1')) {
          expect(local.prefixLength, anyOf(isNull, 8));
        }
      },
    );

    test('the probe leaves a prefix null when none is known, or the lookup '
        'throws', () async {
      final none = await IoNetworkProbe(prefixes: () => {}).localAddresses();
      expect(none.every((l) => l.prefixLength == null), isTrue);
      final thrown = await IoNetworkProbe(
        prefixes: () => throw const FormatException('no'),
      ).localAddresses();
      expect(thrown.every((l) => l.prefixLength == null), isTrue);
      final zero = await IoNetworkProbe(
        prefixes: () => {
          for (final l in none) prefixKey(l.interface, l.address): 0,
        },
      ).localAddresses();
      expect(zero.every((l) => l.prefixLength == null), isTrue);
    });
  });

  group('what failed attempts say', () {
    test(
      'a refused connection is counted by errno, and asking resets',
      () async {
        final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
        final port = server.port;
        await server.close();
        final probe = IoNetworkProbe();
        expect(probe.takeDiagnostics(), '');
        final never = Completer<void>().future;
        expect(
          await probe.canConnect(
            '127.0.0.1',
            port,
            timeout: const Duration(seconds: 2),
            cancelled: never,
          ),
          isFalse,
        );
        if (Platform.isLinux) {
          expect(probe.takeDiagnostics(), 'errno111=1');
        }
        expect(probe.takeDiagnostics(), '');
      },
    );

    test('an accepted connection is true and not counted', () async {
      final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      final probe = IoNetworkProbe();
      expect(
        await probe.canConnect(
          '127.0.0.1',
          server.port,
          timeout: const Duration(seconds: 2),
          cancelled: Completer<void>().future,
        ),
        isTrue,
      );
      await server.close();
      expect(probe.takeDiagnostics(), '');
    });
  });
}
