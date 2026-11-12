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
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/wifi_address.dart';

void main() {
  group('parsing what a person typed', () {
    WifiAddress ok(String text) => parseWifiAddress(text);

    String? refused(String text) {
      try {
        parseWifiAddress(text);
        return null;
      } on FormatException catch (e) {
        return e.message;
      }
    }

    test('a name gets the default port', () {
      expect(ok('mtrack.local'), const WifiAddress('mtrack.local', 43234));
      expect(ok('stage-pi'), const WifiAddress('stage-pi', 43234));
    });

    test('a number gets the default port', () {
      expect(ok('192.168.1.40'), const WifiAddress('192.168.1.40', 43234));
    });

    test('a port after a colon is taken', () {
      expect(
        ok('mtrack.local:50051'),
        const WifiAddress('mtrack.local', 50051),
      );
      expect(ok('10.0.0.2:1'), const WifiAddress('10.0.0.2', 1));
      expect(ok('10.0.0.2:65535'), const WifiAddress('10.0.0.2', 65535));
    });

    test('spaces around it are let go', () {
      expect(ok('  mtrack.local \n'), const WifiAddress('mtrack.local', 43234));
    });

    test('an IPv6 address is read bare, or in brackets with a port', () {
      expect(ok('fe80::1'), const WifiAddress('fe80::1', 43234));
      expect(ok('[fd00::2]'), const WifiAddress('fd00::2', 43234));
      expect(ok('[fd00::2]:50051'), const WifiAddress('fd00::2', 50051));
    });

    test('nothing, or only spaces, is asked for again', () {
      expect(refused(''), contains("Type the player's address"));
      expect(refused('   '), contains("Type the player's address"));
      expect(refused(':43234'), contains("Type the player's address"));
    });

    test('a web address is not taken, since its port is the web page\'s', () {
      expect(
        refused('http://mtrack.local:8080/'),
        contains('not a web address'),
      );
      expect(refused('mtrack.local/ui'), contains('not a web address'));
      expect(refused('my player'), contains('not a web address'));
    });

    test('a bad port is refused with the range', () {
      for (final bad in ['a:0', 'a:65536', 'a:-1', 'a:x', 'a:', 'a:4 3']) {
        expect(refused(bad), isNotNull, reason: bad);
      }
      expect(refused('a:0'), contains('1 to 65535'));
    });

    test('a number that is not an address is refused', () {
      for (final bad in [
        '192.168.1',
        '192.168.1.256',
        '1.2.3.4.5',
        '999.1.1.1',
      ]) {
        expect(refused(bad), contains('not a valid number'), reason: bad);
      }
    });

    test('a name with characters a host cannot have is refused', () {
      for (final bad in ['mtrack!.local', '-bad.local', 'a..b', 'bad-.local']) {
        expect(
          refused(bad),
          contains('letters, numbers and hyphens'),
          reason: bad,
        );
      }
    });

    test('a broken IPv6 address is refused', () {
      expect(refused('[fd00::2'), contains('closing ]'));
      expect(refused('[fd00::zz]'), contains('not a valid IPv6'));
      expect(refused('1:2:3'), contains('not a valid IPv6'));
    });

    test('it reads back as it would be typed', () {
      expect(
        const WifiAddress('mtrack.local', 43234).toString(),
        'mtrack.local',
      );
      expect(const WifiAddress('10.0.0.2', 50051).toString(), '10.0.0.2:50051');
      expect(const WifiAddress('fd00::2', 50051).toString(), '[fd00::2]:50051');
    });
  });

  group('what a player remembers of the network', () {
    pp.GetNetworkInfoResponse info({
      String hostname = 'stage',
      List<String> addresses = const ['192.168.1.40', 'fd00::40'],
      int? port = 43234,
    }) => pp.GetNetworkInfoResponse(
      hostname: hostname,
      addresses: addresses,
      grpcPort: port,
    );

    test('a Bluetooth player takes the addresses and port it is told', () {
      const before = RememberedPlayer(id: 'AA', name: 'stage');
      final after = before.learned(info(), null);
      expect(after.hosts, ['192.168.1.40', 'fd00::40']);
      expect(after.port, 43234);
      expect(after.hostname, 'stage');
      expect(after.id, 'AA');
      expect(after.name, 'stage');
      expect(after.bluetooth, isTrue);
      expect(after.hasWifi, isTrue);
    });

    test('new numbers replace old ones, and a typed name stays, first', () {
      const before = RememberedPlayer(
        id: 'wifi:mtrack.local:43234',
        name: 'mtrack.local',
        hosts: ['mtrack.local', '10.0.0.5'],
        port: 43234,
        bluetooth: false,
      );
      final after = before.learned(info(), 'mtrack.local');
      expect(after.hosts, ['mtrack.local', '192.168.1.40', 'fd00::40']);
      expect(
        after.name,
        'stage',
        reason:
            'a player known by address takes the '
            'name it gives itself',
      );
    });

    test('the address that answered goes first', () {
      const before = RememberedPlayer(id: 'AA', name: 'stage');
      final after = before.learned(info(), 'fd00::40');
      expect(after.hosts, ['fd00::40', '192.168.1.40']);
    });

    test('a player with no gRPC controller leaves nothing to try', () {
      const before = RememberedPlayer(
        id: 'AA',
        name: 'stage',
        hosts: ['192.168.1.40'],
        port: 43234,
      );
      final after = before.learned(info(port: null), null);
      expect(after.hasWifi, isFalse);
      expect(after.hostname, 'stage');
    });

    test('a Bluetooth player keeps the name it advertised', () {
      const before = RememberedPlayer(id: 'AA', name: 'Stage Box');
      expect(
        before.learned(info(hostname: 'raspberrypi'), null).name,
        'Stage Box',
      );
    });

    test('no more than six addresses are kept', () {
      const before = RememberedPlayer(id: 'AA', name: 'stage');
      final after = before.learned(
        info(addresses: [for (var i = 1; i <= 9; i++) '10.0.0.$i']),
        null,
      );
      expect(after.hosts, hasLength(RememberedPlayer.maxHosts));
    });

    test('equality takes in the addresses', () {
      const a = RememberedPlayer(
        id: 'AA',
        name: 'x',
        hosts: ['1.1.1.1'],
        port: 1,
      );
      const same = RememberedPlayer(
        id: 'AA',
        name: 'x',
        hosts: ['1.1.1.1'],
        port: 1,
      );
      const other = RememberedPlayer(
        id: 'AA',
        name: 'x',
        hosts: ['1.1.1.2'],
        port: 1,
      );
      expect(a, same);
      expect(a.hashCode, same.hashCode);
      expect(a, isNot(other));
    });

    test('a player known by address has an id of its own and no Bluetooth', () {
      final player = RememberedPlayer.byAddress(
        const WifiAddress('mtrack.local', 50051),
      );
      expect(player.id, 'wifi:mtrack.local:50051');
      expect(player.name, 'mtrack.local');
      expect(player.bluetooth, isFalse);
      expect(player.hasWifi, isTrue);
    });
  });
}
