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
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/network_scan.dart';
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/wifi_address.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';

import '../support/fake_probe.dart';
import '../support/session_harness.dart';

const String ip = '192.168.1.40';

pp.GetNetworkInfoResponse info(String name, {int port = 43234}) =>
    pp.GetNetworkInfoResponse(hostname: name, addresses: [ip], grpcPort: port);

void main() {
  group('which interface is the Wi-Fi', () {
    test('is Android wlan and iOS en0, and nothing else', () {
      for (final name in ['wlan0', 'wlan1', 'en0']) {
        expect(isWifiInterface(name), isTrue, reason: name);
      }
      for (final name in [
        // Mobile data.
        'rmnet0', 'rmnet_data1', 'pdp_ip0', 'pdp_ip1', 'ccmni0',
        // VPNs.
        'tun0', 'utun3', 'ipsec0', 'wg0', 'tailscale0',
        // Loopback, wired, hotspot, peer to peer, other iOS interfaces and
        // anything unknown.
        'lo', 'lo0', 'eth0', 'swlan0', 'p2p0', 'ap0', 'en1', 'en2', 'awdl0',
        'bridge100', 'wlan', 'wlan0:1', 'WLAN0', 'EN0', 'wlan0 ', '',
      ]) {
        expect(isWifiInterface(name), isFalse, reason: name);
      }
    });
  });

  group('which addresses may be scanned', () {
    test('accepts 10/8, 172.16/12, 192.168/16 and link-local', () {
      for (final a in [
        '10.0.0.1',
        '10.255.255.254',
        '172.16.0.1',
        '172.20.5.5',
        '172.31.255.254',
        '192.168.0.1',
        '192.168.255.254',
        '169.254.10.20',
      ]) {
        expect(isScannableAddress(a), isTrue, reason: a);
      }
    });

    test('refuses everything else at the edges of those ranges', () {
      for (final a in [
        '9.255.255.255',
        '11.0.0.1',
        '172.15.255.255',
        '172.32.0.1',
        '192.167.1.1',
        '192.169.1.1',
        '169.253.1.1',
        '169.255.1.1',
        // Carrier-grade NAT.
        '100.64.0.1',
        '100.100.100.100',
        '100.127.255.254',
        // Public, loopback, multicast.
        '8.8.8.8',
        '1.1.1.1',
        '127.0.0.1',
        '224.0.0.1',
        '0.0.0.0',
      ]) {
        expect(isScannableAddress(a), isFalse, reason: a);
      }
    });

    test('refuses what is not an IPv4 address', () {
      for (final a in [
        '',
        'fe80::1',
        'fd00::1',
        '10.0.0',
        '10.0.0.1.1',
        '10.0.0.256',
        ' 10.0.0.1',
        '10.0.0.x',
        '-10.0.0.1',
        'mtrack.local',
      ]) {
        expect(isScannableAddress(a), isFalse, reason: a);
      }
    });

    test('takes the phone address from the Wi-Fi interface only', () {
      expect(
        scannableWifiAddress(const [
          LocalAddress('lo', '127.0.0.1'),
          LocalAddress('rmnet_data0', '10.20.30.40'),
          LocalAddress('tun0', '192.168.99.2'),
          LocalAddress('wlan0', '192.168.1.20'),
        ]),
        '192.168.1.20',
      );
      expect(
        scannableWifiAddress(const [LocalAddress('en0', '172.16.4.9')]),
        '172.16.4.9',
      );
      expect(
        scannableWifiAddress(const [
          LocalAddress('rmnet0', '10.20.30.40'),
          LocalAddress('utun2', '10.8.0.2'),
        ]),
        isNull,
      );
      expect(scannableWifiAddress(const []), isNull);
    });

    test('is null when the Wi-Fi address is public or carrier-grade NAT', () {
      expect(
        scannableWifiAddress(const [LocalAddress('wlan0', '100.64.1.5')]),
        isNull,
      );
      expect(
        scannableWifiAddress(const [LocalAddress('wlan0', '203.0.113.9')]),
        isNull,
      );
    });
  });

  group('the addresses of the sweep', () {
    List<String> hostsOf(String phone, int? prefix) =>
        SweepPlan(phone, prefix).hosts.toList();

    /// The same set of addresses, whatever order.
    void expectBlock(List<String> hosts, String from, String to) {
      expect(hosts, contains(from));
      expect(hosts, contains(to));
    }

    test('a /24 is the 253 hosts less the phone, ascending', () {
      final plan = SweepPlan('192.168.1.20', 24);
      final hosts = plan.hosts.toList();
      expect(plan.total, 253);
      expect(plan.ownBlockCount, 253);
      expect(plan.clamped, isFalse);
      expect(plan.prefix, 24);
      expect(hosts, hasLength(253));
      expect(hosts, isNot(contains('192.168.1.20')));
      expect(hosts, isNot(contains('192.168.1.0')));
      expect(hosts, isNot(contains('192.168.1.255')));
      expect(hosts.first, '192.168.1.1');
      expect(hosts.last, '192.168.1.254');
      expect(hosts.toSet(), hasLength(253));
    });

    test('no prefix is a /24', () {
      final plan = SweepPlan('10.4.5.1', null);
      expect(plan.prefix, 24);
      expect(plan.total, 253);
      expect(plan.hosts.first, '10.4.5.2');
      expect(plan.hosts, contains('10.4.5.254'));
      expect(SweepPlan('10.4.5.254', null).hosts, contains('10.4.5.1'));
    });

    test('a /16 starts in the phone\'s block and goes outward', () {
      final plan = SweepPlan('10.1.100.7', 16);
      expect(plan.total, 65534 - 1);
      expect(plan.ownBlockCount, 255);
      expect(plan.clamped, isFalse);
      final hosts = plan.hosts.toList();
      expect(hosts, hasLength(plan.total));
      expect(hosts.toSet(), hasLength(plan.total));
      expect(hosts, isNot(contains('10.1.100.7')));
      expect(hosts, isNot(contains('10.1.0.0')));
      expect(hosts, isNot(contains('10.1.255.255')));
      expect(hosts.every((h) => h.startsWith('10.1.')), isTrue);
      // Own block, .0 to .255 less the phone, ascending.
      expect(hosts.sublist(0, 255), [
        for (var i = 0; i < 256; i++)
          if (i != 7) '10.1.100.$i',
      ]);
      // Then the block below, the block above, two below, two above.
      expect(hosts[255], '10.1.99.0');
      expect(hosts[255 + 255], '10.1.99.255');
      expect(hosts[255 + 256], '10.1.101.0');
      expect(hosts[255 + 512], '10.1.98.0');
      expect(hosts[255 + 768], '10.1.102.0');
    });

    test('a phone in the first block of a /16 goes upward only', () {
      final hosts = hostsOf('172.16.0.9', 16);
      expect(hosts, isNot(contains('172.16.0.0')));
      expect(hosts.first, '172.16.0.1');
      expect(hosts[254], '172.16.1.0');
      expect(hosts[254 + 256], '172.16.2.0');
      expect(hosts.last, '172.16.255.254');
      expect(hosts, hasLength(65534 - 1));
    });

    test('a phone in the last block of a /16 goes downward only', () {
      final hosts = hostsOf('172.16.255.9', 16);
      expect(hosts.first, '172.16.255.0');
      expect(hosts[254], '172.16.254.0');
      expect(hosts[254 + 256], '172.16.253.0');
      expect(hosts.last, '172.16.0.255');
      expect(hosts, hasLength(65534 - 1));
      expect(hosts, isNot(contains('172.16.255.255')));
    });

    test('a /22 is four blocks, its own first', () {
      final plan = SweepPlan('192.168.5.9', 22);
      final hosts = plan.hosts.toList();
      // 192.168.4.0/22 is .4.0 to .7.255.
      expect(plan.total, 1024 - 2 - 1);
      expect(hosts, hasLength(plan.total));
      expect(plan.ownBlockCount, 255);
      expect(hosts, isNot(contains('192.168.4.0')));
      expect(hosts, isNot(contains('192.168.7.255')));
      expectBlock(hosts, '192.168.4.1', '192.168.7.254');
      expect(hosts.first, '192.168.5.0');
      // The block below, then the one above, then two above (none below).
      expect(hosts[255], '192.168.4.1');
      expect(hosts[255 + 255], '192.168.6.0');
      expect(hosts[255 + 255 + 256], '192.168.7.0');
    });

    test('a /20 is sixteen blocks around the phone\'s', () {
      final plan = SweepPlan('10.0.20.200', 20);
      // 10.0.16.0/20 is .16.0 to .31.255.
      expect(plan.total, 4096 - 2 - 1);
      final hosts = plan.hosts.toList();
      expect(hosts, hasLength(plan.total));
      expect(hosts.toSet(), hasLength(plan.total));
      expect(hosts, isNot(contains('10.0.16.0')));
      expect(hosts, isNot(contains('10.0.31.255')));
      expect(hosts.first, '10.0.20.0');
      expect(hosts[255], '10.0.19.0');
      expect(hosts[255 + 256], '10.0.21.0');
    });

    test('a network wider than a /16 is clamped to the /16 of the phone', () {
      for (final (prefix, phone, start) in [
        (12, '172.20.30.40', '172.20.'),
        (8, '10.77.8.9', '10.77.'),
        (0, '192.168.4.4', '192.168.'),
      ]) {
        final plan = SweepPlan(phone, prefix);
        expect(plan.clamped, isTrue, reason: '/$prefix');
        expect(plan.prefix, 16);
        expect(plan.total, 65533, reason: '/$prefix');
        final hosts = plan.hosts.toList();
        expect(hosts, hasLength(65533));
        expect(hosts.every((h) => h.startsWith(start)), isTrue);
      }
    });

    test('a /16 itself is not clamped', () {
      expect(SweepPlan('10.1.2.3', 16).clamped, isFalse);
    });

    test('a /25 is the half block holding the phone', () {
      final upper = SweepPlan('192.168.1.200', 25);
      final hosts = upper.hosts.toList();
      // 192.168.1.128/25: hosts .129 to .254.
      expect(upper.total, 126 - 1);
      expect(upper.ownBlockCount, 125);
      expect(hosts.first, '192.168.1.129');
      expect(hosts.last, '192.168.1.254');
      expect(hosts, isNot(contains('192.168.1.200')));
      final lower = SweepPlan('192.168.1.20', 25).hosts.toList();
      expect(lower.first, '192.168.1.1');
      expect(lower.last, '192.168.1.126');
      expect(lower, hasLength(125));
    });

    test('a /30 is the other host', () {
      expect(hostsOf('192.168.1.5', 30), ['192.168.1.6']);
      expect(hostsOf('192.168.1.6', 30), ['192.168.1.5']);
      expect(SweepPlan('192.168.1.5', 30).total, 1);
    });

    test('a /31 is the other address and a /32 is nothing', () {
      expect(hostsOf('192.168.1.4', 31), ['192.168.1.5']);
      expect(hostsOf('192.168.1.5', 31), ['192.168.1.4']);
      expect(SweepPlan('192.168.1.5', 31).total, 1);
      expect(hostsOf('192.168.1.5', 32), isEmpty);
      expect(SweepPlan('192.168.1.5', 32).total, 0);
      expect(SweepPlan('192.168.1.5', 32).ownBlockCount, 0);
    });

    test('a prefix that makes no sense is a /24', () {
      expect(SweepPlan('10.1.2.3', 33).prefix, 24);
      expect(SweepPlan('10.1.2.3', -1).prefix, 24);
    });

    test('is made as it is read, not held as a list', () {
      final plan = SweepPlan('10.1.100.7', 16);
      expect(plan.hosts.take(3).toList(), [
        '10.1.100.0',
        '10.1.100.1',
        '10.1.100.2',
      ]);
    });

    test('are none for something that is not an IPv4 address', () {
      for (final phone in ['fe80::1', '', '1.2.3']) {
        final plan = SweepPlan(phone, 24);
        expect(plan.hosts, isEmpty);
        expect(plan.total, 0);
      }
    });

    test('are tried on the standard port, and the remembered one if it '
        'differs', () {
      expect(sweepPorts(null), [43234]);
      expect(sweepPorts(43234), [43234]);
      expect(sweepPorts(50051), [43234, 50051]);
    });
  });

  group('scanning the network', () {
    /// Runs [body] over a harness with a network, a browser and a probe, in
    /// fake time, with no player remembered unless [player] says so; the
    /// phone is on 192.168.1.20 and the player at [ip] is on the network.
    /// Afterwards nothing may be left running.
    void scenario(
      String name,
      void Function(Harness h, FakeAsync async) body, {
      RememberedPlayer? player,
    }) {
      test(name, () {
        fakeAsync((async) {
          final h = Harness(
            async,
            withWifi: true,
            withBrowser: true,
            withProbe: true,
            player: player,
            known: player != null,
          );
          h.rig.networkInfo = info('stage');
          h.wifi!.reachable.add(ip);
          h.probe!.open.add('$ip:43234');
          body(h, async);
          h.session.dispose();
          h.settle();
          expect(h.probe!.inFlight, 0);
          h.elapse(const Duration(seconds: 2));
          expect(async.pendingTimers, isEmpty);
        });
      });
    }

    /// The names the list shows, scanned ones marked.
    List<String> names(Harness h) => [
      for (final e in h.session.wifiEntries)
        '${e.player.name}${e.scanned ? '*' : ''}',
    ];

    const quiet = Duration(seconds: 12);

    group('only on the button', () {
      scenario('is not started by opening the screen, looking again or '
          'waiting', (h, _) {
        h.session.startScan();
        h.settle();
        h.elapse(quiet);
        h.session.startScan();
        h.settle();
        h.elapse(quiet);
        expect(h.probe!.listings, 0);
        expect(h.probe!.attempts, isEmpty);
        expect(h.session.networkScan, NetworkScan.idle);
        expect(h.wifi!.attempts, isEmpty);
      });

      scenario('is not started by a player that does not answer, or its '
          'reconnection', (h, _) {
        h.wifi!.reachable.clear();
        h.session.start();
        h.settle();
        h.elapse(const Duration(minutes: 2));
        expect(h.probe!.listings, 0);
        expect(h.probe!.attempts, isEmpty);
      }, player: const RememberedPlayer(id: 'p1', name: 'stage'));

      scenario('is offered to a session with a probe and Wi-Fi', (h, _) {
        expect(h.session.canScanNetwork, isTrue);
      });

      test('is not possible without a probe', () {
        fakeAsync((async) {
          final h = Harness(async, withWifi: true, known: false);
          expect(h.session.canScanNetwork, isFalse);
          h.session.scanNetwork();
          h.settle();
          expect(h.session.networkScan, NetworkScan.idle);
          h.session.dispose();
        });
      });

      scenario('does nothing once a player is chosen', (h, _) {
        h.connect();
        h.session.scanNetwork();
        h.settle();
        expect(h.probe!.listings, 0);
        expect(h.session.networkScan, NetworkScan.idle);
      }, player: const RememberedPlayer(id: 'p1', name: 'stage'));
    });

    group('only the phone\'s own Wi-Fi', () {
      scenario('uses wlan0 and not mobile data, a VPN or loopback', (h, _) {
        h.probe!.addresses = const [
          LocalAddress('lo', '127.0.0.1'),
          LocalAddress('rmnet_data2', '10.50.60.70'),
          LocalAddress('tun0', '172.20.0.2'),
          LocalAddress('tailscale0', '10.9.9.9'),
          LocalAddress('wlan0', '192.168.1.20'),
        ];
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(h.probe!.attempts, isNotEmpty);
        expect(
          h.probe!.attempts.every((a) => a.startsWith('192.168.1.')),
          isTrue,
        );
      });

      scenario('uses en0 on iOS', (h, _) {
        h.probe!.addresses = const [
          LocalAddress('pdp_ip0', '10.50.60.70'),
          LocalAddress('utun1', '10.8.0.2'),
          LocalAddress('en0', '172.16.40.9'),
        ];
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(
          h.probe!.attempts.every((a) => a.startsWith('172.16.40.')),
          isTrue,
        );
        expect(h.probe!.attempts, contains('172.16.40.1:43234'));
      });

      scenario('with only mobile data and a VPN is not scannable', (h, _) {
        h.probe!.addresses = const [
          LocalAddress('rmnet0', '10.50.60.70'),
          LocalAddress('tun0', '192.168.77.2'),
        ];
        h.session.scanNetwork();
        h.settle();
        expect(h.session.networkScan, NetworkScan.notScannable);
        expect(h.probe!.attempts, isEmpty);
      });

      scenario('with no Wi-Fi interface at all is not scannable', (h, _) {
        h.probe!.addresses = const [];
        h.session.scanNetwork();
        h.settle();
        expect(h.session.networkScan, NetworkScan.notScannable);
        expect(h.probe!.attempts, isEmpty);
      });

      scenario('with interfaces that cannot be listed is not scannable', (
        h,
        _,
      ) {
        h.probe!.throwOnList = const FormatException('no');
        h.session.scanNetwork();
        h.settle();
        expect(h.session.networkScan, NetworkScan.notScannable);
        expect(h.probe!.attempts, isEmpty);
      });
    });

    group('only private addresses', () {
      for (final address in [
        '10.0.0.5',
        '10.200.1.9',
        '172.16.0.5',
        '172.31.255.9',
        '192.168.0.7',
        '169.254.3.3',
      ]) {
        scenario('sweeps around $address', (h, _) {
          h.probe!.addresses = [LocalAddress('wlan0', address)];
          h.session.scanNetwork();
          h.settle();
          h.elapse(quiet);
          final prefix = address.substring(0, address.lastIndexOf('.') + 1);
          expect(h.probe!.attempts, hasLength(253));
          expect(h.probe!.attempts.every((a) => a.startsWith(prefix)), isTrue);
        });
      }

      for (final address in [
        '172.15.1.1',
        '172.32.1.1',
        '100.64.12.1',
        '100.127.0.9',
        '8.8.4.4',
        '203.0.113.7',
        '192.169.0.1',
      ]) {
        scenario('refuses $address, saying so and sweeping nothing', (h, _) {
          h.probe!.addresses = [LocalAddress('wlan0', address)];
          h.session.scanNetwork();
          h.settle();
          h.elapse(quiet);
          expect(h.session.networkScan, NetworkScan.notScannable);
          expect(h.probe!.attempts, isEmpty);
          expect(h.wifi!.attempts, isEmpty);
        });
      }
    });

    group('the /24 and the ports', () {
      scenario('tries the 254 hosts less the phone, once, on the standard '
          'port', (h, _) {
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(h.probe!.attempts, hasLength(253));
        expect(h.probe!.attempts.toSet(), hasLength(253));
        expect(h.probe!.attempts, isNot(contains('192.168.1.20:43234')));
        expect(h.probe!.attempts, contains('192.168.1.1:43234'));
        expect(h.probe!.attempts, contains('192.168.1.254:43234'));
        expect(h.probe!.attempts.every((a) => a.endsWith(':43234')), isTrue);
      });

      scenario(
        'also tries the remembered player\'s port when it differs',
        (h, _) {
          h.session.scanNetwork();
          h.settle();
          h.elapse(quiet);
          expect(h.probe!.attempts, hasLength(506));
          expect(h.probe!.attempts, contains('192.168.1.99:50051'));
          expect(h.probe!.attempts, contains('192.168.1.99:43234'));
          expect(h.probe!.attempts.map((a) => a.split(':').last).toSet(), {
            '43234',
            '50051',
          });
        },
        player: const RememberedPlayer(
          id: 'wifi:x:50051',
          name: 'old',
          hosts: ['10.9.9.9'],
          port: 50051,
          bluetooth: false,
        ),
      );

      scenario(
        'does not repeat the standard port when it is the remembered '
        'one',
        (h, _) {
          h.session.scanNetwork();
          h.settle();
          h.elapse(quiet);
          expect(h.probe!.attempts, hasLength(253));
        },
        player: const RememberedPlayer(
          id: 'wifi:x:43234',
          name: 'old',
          hosts: ['10.9.9.9'],
          port: 43234,
          bluetooth: false,
        ),
      );
    });

    group('pace', () {
      const wide = LocalAddress('wlan0', '192.168.1.20', prefixLength: 16);

      scenario('never has more than 512 attempts in flight, and tries all', (
        h,
        _,
      ) {
        h.probe!.open.clear();
        h.probe!.addresses = const [wide];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(minutes: 5));
        expect(h.probe!.maxInFlight, lessThanOrEqualTo(sweepConcurrency));
        expect(h.probe!.attempts, hasLength(65533));
        expect(h.probe!.attempts.toSet(), hasLength(65533));
        expect(h.session.networkScan, NetworkScan.none);
      });

      scenario('gives every address 1 s, the phone\'s own /24 first', (h, _) {
        h.probe!.addresses = const [wide];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(minutes: 5));
        final timeouts = h.probe!.timeouts;
        expect(timeouts, hasLength(65533));
        expect(timeouts.every((t) => t == sweepAttemptTimeout), isTrue);
        expect(sweepAttemptTimeout, const Duration(seconds: 1));
        expect(sweepConcurrency, 512);
        expect(sweepAddressesPerSecond, 256);
        expect(sweepMaxDuration, const Duration(minutes: 10));
        // Own block: .0 to .255 less the phone.
        expect(
          h.probe!.attempts.take(255).every((a) => a.startsWith('192.168.1.')),
          isTrue,
        );
        expect(h.probe!.attempts[255], '192.168.0.1:43234');
      });

      scenario('on a quiet /24 tries all at once and ends in 1 s', (h, _) {
        h.probe!.open.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 900));
        expect(h.session.networkScan, NetworkScan.scanning);
        expect(h.probe!.maxInFlight, 253);
        h.elapse(const Duration(milliseconds: 200));
        expect(h.session.networkScan, NetworkScan.none);
        expect(h.probe!.inFlight, 0);
        expect(h.session.scanTried, 253);
        expect(h.session.scanTotal, 253);
        expect(h.session.scanPartial, isFalse);
      });

      scenario(
        'on two ports tries each host\'s ports one after the other',
        (h, _) {
          h.probe!.open.clear();
          h.session.scanNetwork();
          h.settle();
          h.elapse(const Duration(seconds: 3));
          expect(h.session.networkScan, NetworkScan.none);
          final attempts = h.probe!.attempts;
          expect(attempts, hasLength(506));
          expect(attempts[0], '192.168.1.1:43234');
          expect(attempts[1], '192.168.1.1:50051');
          expect(attempts[2], '192.168.1.2:43234');
          expect(attempts[3], '192.168.1.2:50051');
        },
        player: const RememberedPlayer(
          id: 'wifi:x:50051',
          name: 'old',
          hosts: ['10.9.9.9'],
          port: 50051,
          bluetooth: false,
        ),
      );

      scenario('knows how many addresses it has covered', (h, _) {
        h.probe!.addresses = const [wide];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(seconds: 5));
        expect(h.session.scanTotal, 65533);
        expect(h.session.scanTried, inExclusiveRange(0, 65533));
        h.elapse(const Duration(minutes: 5));
        expect(h.session.scanTried, 65533);
        expect(h.session.scanPartial, isFalse);
      });

      scenario('says a bigger network was only partly scanned', (h, _) {
        h.probe!.addresses = const [
          LocalAddress('wlan0', '10.20.30.40', prefixLength: 12),
        ];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(minutes: 5));
        expect(h.session.scanTotal, 65533);
        expect(h.session.scanPartial, isTrue);
        expect(h.session.scanCutShort, isFalse);
        expect(h.probe!.attempts.every((a) => a.startsWith('10.20.')), isTrue);
      });

      NetworkSweep pacedSweep(
        FakeProbe probe, {
        String phone = '192.168.1.20',
        int prefix = 16,
        List<int> ports = const [43234],
      }) => NetworkSweep(
        probe: probe,
        wifi: _NoWifi(),
        plan: SweepPlan(phone, prefix),
        ports: ports,
        onFound: (_) {},
      );

      test('starts the phone\'s own block at once, unpaced', () {
        fakeAsync((async) {
          final probe = FakeProbe()..clock = () => async.elapsed;
          final sweep = pacedSweep(probe);
          sweep.run();
          async.flushMicrotasks();
          // The own block (255 hosts of the /16 less the phone) before any
          // time passes, and a first eighth of a second's worth beyond it.
          final own = SweepPlan('192.168.1.20', 16).ownBlockCount;
          expect(own, 255);
          expect(
            probe.startTimes.where((t) => t == Duration.zero).length,
            greaterThanOrEqualTo(own),
          );
          expect(
            probe.attempts.take(own).every((a) => a.startsWith('192.168.1.')),
            isTrue,
          );
          sweep.cancel();
          async.flushMicrotasks();
        });
      });

      test('starts no more than 256 new addresses in any second', () {
        fakeAsync((async) {
          final probe = FakeProbe()..clock = () => async.elapsed;
          final sweep = pacedSweep(probe);
          sweep.run();
          async.elapse(const Duration(seconds: 30));
          async.flushMicrotasks();
          final own = SweepPlan('192.168.1.20', 16).ownBlockCount;
          final times = probe.startTimes.skip(own).toList();
          expect(times.length, greaterThan(256 * 25));
          // A window of a second opening at every start.
          var end = 0;
          for (var i = 0; i < times.length; i++) {
            if (end < i) {
              end = i;
            }
            while (end < times.length &&
                times[end] < times[i] + const Duration(seconds: 1)) {
              end++;
            }
            expect(end - i, lessThanOrEqualTo(sweepAddressesPerSecond));
          }
          sweep.cancel();
          async.flushMicrotasks();
        });
      });

      test('a /16 is done in about 256 s, with no timer left', () {
        fakeAsync((async) {
          final probe = FakeProbe();
          final sweep = pacedSweep(probe);
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 250));
          async.flushMicrotasks();
          expect(summary, isNull);
          async.elapse(const Duration(seconds: 10));
          async.flushMicrotasks();
          expect(summary, isNotNull);
          expect(summary!.end, SweepEnd.done);
          expect(summary!.tried, 65533);
          expect(probe.attempts, hasLength(65533));
          expect(async.pendingTimers, isEmpty);
        });
      });

      test('a /22 takes about 4 s and a /24 about 1 s', () {
        fakeAsync((async) {
          for (final (prefix, seconds) in [(24, 1), (22, 4)]) {
            final probe = FakeProbe();
            final sweep = pacedSweep(probe, prefix: prefix);
            SweepSummary? summary;
            sweep.run().then((s) => summary = s);
            async.elapse(Duration(seconds: seconds - 1));
            async.flushMicrotasks();
            expect(summary, isNull, reason: '/$prefix is still going');
            async.elapse(const Duration(seconds: 3));
            async.flushMicrotasks();
            expect(summary?.end, SweepEnd.done, reason: '/$prefix is done');
          }
          expect(async.pendingTimers, isEmpty);
        });
      });

      test('stopping while waiting for tokens lets every waiter go', () {
        fakeAsync((async) {
          final probe = FakeProbe();
          final sweep = pacedSweep(probe);
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 3));
          async.flushMicrotasks();
          expect(summary, isNull);
          final before = probe.attempts.length;
          sweep.cancel();
          async.flushMicrotasks();
          expect(summary!.end, SweepEnd.stopped);
          async.elapse(const Duration(seconds: 5));
          async.flushMicrotasks();
          expect(probe.attempts.length, before);
          expect(probe.inFlight, 0);
          expect(async.pendingTimers, isEmpty);
        });
      });

      test('a second port on a host needs no token', () {
        fakeAsync((async) {
          final probe = FakeProbe()..clock = () => async.elapsed;
          final sweep = pacedSweep(
            probe,
            phone: '10.0.0.20',
            prefix: 22,
            ports: const [43234, 50051],
          );
          sweep.run();
          async.elapse(const Duration(seconds: 3));
          async.flushMicrotasks();
          final own = SweepPlan('10.0.0.20', 22).ownBlockCount * 2;
          // Past the own block, a host's two attempts start together, and
          // the hosts are 256 a second, so 512 attempts.
          final paced = probe.attempts.length - own;
          expect(paced, greaterThan(0));
          for (var i = own; i + 1 < probe.attempts.length; i += 2) {
            expect(probe.attempts[i].endsWith(':43234'), isTrue);
            expect(probe.attempts[i + 1].endsWith(':50051'), isTrue);
            expect(probe.startTimes[i + 1], probe.startTimes[i]);
          }
          // Two ports a host: 256 hosts is 512 attempts a second. (The first
          // second is the own block holding most of the 512 in flight.)
          final inThirdSecond = probe.startTimes
              .where(
                (t) =>
                    t >= const Duration(seconds: 2) &&
                    t < const Duration(seconds: 3),
              )
              .length;
          expect(inThirdSecond, 512);
          sweep.cancel();
          async.flushMicrotasks();
        });
      });

      test('stops trying at the time limit and abandons what is in flight', () {
        fakeAsync((async) {
          final probe = FakeProbe();
          final found = <FoundPlayer>[];
          final sweep = NetworkSweep(
            probe: probe,
            wifi: _NoWifi(),
            plan: SweepPlan('192.168.1.20', 16),
            ports: const [43234],
            onFound: found.add,
            attemptTimeout: const Duration(seconds: 3),
            maxDuration: const Duration(seconds: 8),
          );
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 7, milliseconds: 900));
          async.flushMicrotasks();
          expect(summary, isNull);
          final before = probe.attempts.length;
          expect(before, lessThan(65533));
          async.elapse(const Duration(milliseconds: 200));
          async.flushMicrotasks();
          expect(summary, isNotNull);
          expect(summary!.end, SweepEnd.timeLimit);
          expect(summary!.tried, lessThan(summary!.total));
          expect(probe.inFlight, 0);
          expect(probe.abandoned, greaterThan(0));
          async.elapse(const Duration(seconds: 30));
          expect(probe.attempts.length, lessThanOrEqualTo(before + 256));
          expect(async.pendingTimers, isEmpty);
        });
      });

      test('waits for a player being confirmed before it says it is over', () {
        fakeAsync((async) {
          final probe = FakeProbe()..open.add('192.168.1.5:43234');
          final wifi = _SlowWifi();
          final sweep = NetworkSweep(
            probe: probe,
            wifi: wifi,
            plan: SweepPlan('192.168.1.20', 24),
            ports: const [43234],
            onFound: (_) {},
            maxDuration: const Duration(seconds: 5),
          );
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 4));
          async.flushMicrotasks();
          expect(wifi.asked, ['192.168.1.5']);
          expect(summary, isNull, reason: 'the player is being confirmed');
          async.elapse(const Duration(seconds: 40));
          async.flushMicrotasks();
          expect(summary, isNotNull);
          expect(summary!.accepted, 1);
          expect(summary!.end, SweepEnd.done);
        });
      });

      test('reports progress at most ten times a second, and the end', () {
        fakeAsync((async) {
          final probe = FakeProbe();
          final calls = <(int, int)>[];
          final sweep = NetworkSweep(
            probe: probe,
            wifi: _NoWifi(),
            plan: SweepPlan('192.168.1.20', 16),
            ports: const [43234],
            onFound: (_) {},
            onProgress: (tried, total) => calls.add((tried, total)),
          );
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 3));
          async.flushMicrotasks();
          expect(calls.length, lessThanOrEqualTo(31));
          expect(calls.length, greaterThan(3));
          expect(calls.first, (0, 65533));
          async.elapse(const Duration(minutes: 5));
          async.flushMicrotasks();
          expect(summary!.end, SweepEnd.done);
          expect(calls.last, (65533, 65533));
          expect(
            [
              for (var i = 1; i < calls.length; i++)
                calls[i].$1 >= calls[i - 1].$1,
            ].every((up) => up),
            isTrue,
          );
          expect(async.pendingTimers, isEmpty);
        });
      });

      test('says how it went when it is over', () {
        fakeAsync((async) {
          final probe = FakeProbe()
            ..open.addAll(['192.168.1.5:43234', '192.168.1.6:43234']);
          final sweep = NetworkSweep(
            probe: probe,
            wifi: _NoWifi(),
            plan: SweepPlan('192.168.1.20', 24),
            ports: const [43234],
            onFound: (_) {},
            elapsed: () => const Duration(seconds: 7),
          );
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(seconds: 3));
          async.flushMicrotasks();
          expect(summary!.end, SweepEnd.done);
          expect(summary!.tried, 253);
          expect(summary!.total, 253);
          expect(summary!.accepted, 2);
          expect(summary!.confirmed, 0);
          expect(summary!.elapsed, const Duration(seconds: 7));
        });
      });

      test('says it was stopped when it is cancelled', () {
        fakeAsync((async) {
          final probe = FakeProbe();
          final sweep = NetworkSweep(
            probe: probe,
            wifi: _NoWifi(),
            plan: SweepPlan('192.168.1.20', 16),
            ports: const [43234],
            onFound: (_) {},
          );
          SweepSummary? summary;
          sweep.run().then((s) => summary = s);
          async.elapse(const Duration(milliseconds: 100));
          sweep.cancel();
          async.flushMicrotasks();
          expect(summary!.end, SweepEnd.stopped);
          expect(summary!.tried, lessThan(summary!.total));
          expect(probe.inFlight, 0);
          async.elapse(const Duration(seconds: 5));
          expect(async.pendingTimers, isEmpty);
        });
      });
    });

    group('confirming what answers', () {
      scenario('lists a player by its host name and closes the channel', (
        h,
        _,
      ) {
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['stage*']);
        expect(h.session.networkScan, NetworkScan.found);
        final entry = h.session.wifiEntries.single;
        expect(entry.player.hosts, [ip]);
        expect(entry.player.port, 43234);
        expect(h.wifi!.attempts, ['$ip:43234']);
        expect(h.wifi!.channels, hasLength(1));
        expect(h.wifi!.channels.single.calls, ['GetNetworkInfo']);
        expect(h.wifi!.channels.single.currentState.name, 'disconnected');
      });

      scenario('falls back to the address when the name is empty', (h, _) {
        h.wifi!.infos[ip] = pp.GetNetworkInfoResponse(grpcPort: 43234);
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['$ip*']);
      });

      scenario('drops silently an address that accepts a connection and is '
          'not a player', (h, _) {
        h.probe!.open.add('192.168.1.60:43234');
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(
          h.wifi!.attempts,
          containsAll(['$ip:43234', '192.168.1.60:43234']),
        );
        expect(names(h), ['stage*']);
        // All that was done to it was to try to connect as a player.
        expect(h.wifi!.channels, hasLength(1));
        expect(h.session.connectError, isNull);
        expect(h.session.networkScan, NetworkScan.found);
      });

      scenario('drops a player too old to speak over Wi-Fi', (h, _) {
        h.probe!.open.add('192.168.1.61:43234');
        h.wifi!.reachable.add('192.168.1.61');
        h.wifi!.refusals['192.168.1.61'] = const PlayerException(
          12,
          'unimplemented',
        );
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['stage*']);
        expect(h.wifi!.attempts, contains('192.168.1.61:43234'));
      });

      scenario('says none were found when the only hit is not a player', (
        h,
        _,
      ) {
        h.wifi!.reachable.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(h.session.wifiEntries, isEmpty);
        expect(h.session.networkScan, NetworkScan.none);
      });

      scenario('lists several, in order of name', (h, _) {
        for (final (last, name) in [(70, 'zeta'), (71, 'alpha')]) {
          final host = '192.168.1.$last';
          h.probe!.open.add('$host:43234');
          h.wifi!.reachable.add(host);
          h.wifi!.infos[host] = info(name);
        }
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['alpha*', 'stage*', 'zeta*']);
      });

      scenario('confirms at most four at a time', (h, _) {
        h.wifi!.reachable.clear();
        h.probe!.open
          ..clear()
          ..addAll([for (var i = 2; i < 14; i++) '192.168.1.$i:43234']);
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 100));
        expect(h.wifi!.attempts, hasLength(4));
        h.elapse(const Duration(milliseconds: 1500));
        expect(h.wifi!.attempts, hasLength(8));
      });

      scenario('confirms no more than sixteen in all', (h, _) {
        h.wifi!.reachable.clear();
        h.probe!.open
          ..clear()
          ..addAll([for (var i = 2; i < 52; i++) '192.168.1.$i:43234']);
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(h.wifi!.attempts, hasLength(maxConfirmations));
        expect(h.session.networkScan, NetworkScan.none);
      });

      scenario('gives a connection and a name 1.5 s each', (h, _) {
        h.wifi!.reachable.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 500));
        expect(h.wifi!.attempts, ['$ip:43234']);
        expect(
          h.session.networkScan,
          NetworkScan.scanning,
          reason: 'confirming',
        );
        h.elapse(const Duration(milliseconds: 1500));
        expect(h.session.networkScan, NetworkScan.none);
      });
    });

    group('one list', () {
      scenario('lists a player the advert found once, as the advert row', (
        h,
        _,
      ) {
        h.session.startScan();
        h.settle();
        h.browser!.appear(
          const FoundPlayer(name: 'stage', hosts: ['fe80::1', ip], port: 43234),
        );
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['stage']);
        expect(h.session.wifiEntries.single.scanned, isFalse);
        expect(h.session.networkScan, NetworkScan.found);
      });

      scenario('drops the scan row when the advert later lists it, and '
          'brings it back if the advert row goes', (h, _) {
        h.session.startScan();
        h.settle();
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['stage*']);
        h.browser!.appear(
          const FoundPlayer(name: 'stage', hosts: [ip], port: 43234),
        );
        expect(names(h), ['stage']);
        h.browser!.vanish('stage');
        expect(names(h), ['stage*']);
      });

      scenario('lists the same address on another port as another player', (
        h,
        _,
      ) {
        h.session.startScan();
        h.settle();
        h.browser!.appear(
          const FoundPlayer(name: 'stage', hosts: [ip], port: 50051),
        );
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(names(h), ['stage', 'stage*']);
      });

      scenario('keeps what a scan found through looking again', (h, _) {
        h.session.startScan();
        h.settle();
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        h.session.startScan();
        h.settle();
        expect(names(h), ['stage*']);
        expect(h.session.networkScan, NetworkScan.found);
        expect(h.probe!.listings, 1, reason: 'looking again does not scan');
      });
    });

    group('tapping a scan row', () {
      scenario('connects over Wi-Fi and remembers the player by address '
          'and name', (h, _) {
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        h.wifi!.attempts.clear();
        h.session.connectToScanned(h.session.wifiEntries.single.player);
        h.settle();
        expect(h.session.phase, ConnectionPhase.ready);
        expect(h.session.carrier, Carrier.wifi);
        expect(h.wifi!.attempts, ['$ip:43234']);
        expect(h.link.calls, isEmpty);
        final kept = h.store.player!;
        expect(kept.id, 'wifi:$ip:43234');
        expect(kept.name, 'stage');
        expect(kept.mdnsName, isNull);
        expect(kept.bluetooth, isFalse);
        expect(kept.hosts.first, ip);
        expect(kept.port, 43234);
      });
    });

    group('stopping', () {
      /// Starts a scan on a network that says nothing and lets it get going.
      void startQuiet(Harness h) {
        h.probe!.open.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 100));
        expect(h.probe!.inFlight, 253);
        expect(h.session.networkScan, NetworkScan.scanning);
      }

      void expectStopped(Harness h) {
        expect(h.probe!.inFlight, 0, reason: 'its sockets are closed');
        expect(h.probe!.abandoned, 253);
        final tried = h.probe!.attempts.length;
        h.elapse(quiet);
        expect(h.probe!.attempts, hasLength(tried), reason: 'it went on');
        expect(h.session.networkScan, NetworkScan.idle);
        expect(h.session.wifiEntries, isEmpty);
      }

      scenario('on tapping an advert row', (h, _) {
        startQuiet(h);
        h.session.connectToFound(
          const FoundPlayer(name: 'stage', hosts: [ip], port: 43234),
        );
        h.settle();
        expectStopped(h);
      });

      scenario('on tapping a Bluetooth row', (h, _) {
        h.session.startScan();
        h.settle();
        h.link.discover(
          const DiscoveredPlayer(id: 'p1', name: 'stage', rssi: -40),
        );
        h.settle();
        startQuiet(h);
        h.session.connectTo(h.session.found.first);
        h.settle();
        expectStopped(h);
      });

      scenario('on tapping a scan row', (h, _) {
        h.probe!.open.add('$ip:43234');
        h.probe!.addresses = const [
          LocalAddress('wlan0', '192.168.1.20', prefixLength: 16),
        ];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 500));
        final found = h.session.wifiEntries.single.player;
        h.probe!.open.clear();
        // Scanning goes on for the rest of the network meanwhile.
        expect(h.probe!.inFlight, greaterThan(0));
        h.session.connectToScanned(found);
        h.settle();
        expect(h.probe!.inFlight, 0);
        expect(h.session.networkScan, NetworkScan.idle);
      });

      scenario('on typing an address', (h, _) {
        startQuiet(h);
        h.session.connectToAddress(const WifiAddress(ip, 43234));
        h.settle();
        expectStopped(h);
      });

      scenario('when a remembered player starts connecting', (h, _) {
        startQuiet(h);
        h.session.start();
        h.settle();
        expectStopped(h);
      }, player: const RememberedPlayer(id: 'p1', name: 'stage'));

      scenario('on dispose', (h, _) {
        startQuiet(h);
        h.session.dispose();
        h.settle();
        expect(h.probe!.inFlight, 0);
        expect(h.probe!.abandoned, 253);
      });

      scenario('while a player is being confirmed', (h, _) {
        h.wifi!.reachable.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 500));
        expect(h.wifi!.attempts, ['$ip:43234']);
        h.session.connectToAddress(const WifiAddress('192.168.1.99', 43234));
        h.settle();
        expect(h.session.wifiEntries, isEmpty);
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.idle);
      });

      scenario('does not report a finish after being stopped and started '
          'again', (h, _) {
        startQuiet(h);
        h.session.connectToAddress(const WifiAddress(ip, 43234));
        h.settle();
        expect(h.session.phase, ConnectionPhase.ready);
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.idle);
      });

      scenario('by the person keeps what was found and ends as a finished '
          'scan', (h, _) {
        h.probe!.open.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 100));
        // Held up so that the sweep is still going when it is stopped.
        h.session.stopNetworkScan();
        h.settle();
        expect(h.probe!.inFlight, 0);
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.none);
        expect(h.session.scanCutShort, isTrue);
        expect(h.session.scanPartial, isTrue);
        expect(h.session.scanTotal, 253);
      });

      scenario('by the person keeps the players already shown', (h, _) {
        h.probe!.addresses = const [
          LocalAddress('wlan0', '192.168.1.20', prefixLength: 16),
        ];
        h.session.scanNetwork();
        h.settle();
        h.elapse(const Duration(milliseconds: 500));
        expect(names(h), ['stage*']);
        expect(h.session.networkScan, NetworkScan.scanning);
        h.session.stopNetworkScan();
        h.settle();
        expect(names(h), ['stage*']);
        expect(h.session.networkScan, NetworkScan.found);
        expect(h.session.scanCutShort, isTrue);
        expect(h.session.scanTried, lessThan(h.session.scanTotal));
        final tried = h.probe!.attempts.length;
        h.elapse(quiet);
        expect(h.probe!.attempts, hasLength(tried));
        expect(names(h), ['stage*']);
        // And it can be asked for again.
        h.session.scanNetwork();
        h.settle();
        expect(h.session.networkScan, NetworkScan.scanning);
        h.session.stopNetworkScan();
        h.settle();
      });

      scenario('with nothing running does nothing', (h, _) {
        h.session.stopNetworkScan();
        h.settle();
        expect(h.session.networkScan, NetworkScan.idle);
      });

      scenario('asked before the sweep has begun, ends it unbegun', (h, _) {
        h.session.scanNetwork();
        h.session.stopNetworkScan();
        h.settle();
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.idle);
        expect(h.probe!.attempts, isEmpty);
      });

      scenario('pressing again while it runs does nothing', (h, _) {
        startQuiet(h);
        final listings = h.probe!.listings;
        h.session.scanNetwork();
        h.settle();
        expect(h.probe!.listings, listings);
        expect(h.probe!.inFlight, 253);
        expect(h.probe!.abandoned, 0);
      });

      scenario('can be asked for again once it has ended', (h, _) {
        h.probe!.open.clear();
        h.session.scanNetwork();
        h.settle();
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.none);
        h.session.scanNetwork();
        h.settle();
        expect(h.session.networkScan, NetworkScan.scanning);
        h.elapse(quiet);
        expect(h.session.networkScan, NetworkScan.none);
        expect(h.probe!.attempts, hasLength(506));
      });
    });
  });
}

/// A link that is never reached.
class _NoWifi implements WifiLink {
  @override
  Future<PlayerChannel> connect(
    String host,
    int port, {
    Duration timeout = const Duration(seconds: 2),
  }) => throw const PlayerException.unavailable('no');
}

/// A link that takes its time and then opens a channel, so one can be seen
/// arriving after the end.
class _SlowWifi implements WifiLink {
  final List<String> asked = [];
  int disposed = 0;

  @override
  Future<PlayerChannel> connect(
    String host,
    int port, {
    Duration timeout = const Duration(seconds: 2),
  }) async {
    asked.add(host);
    await Future<void>.delayed(const Duration(seconds: 30));
    return _SlowChannel(this);
  }
}

class _SlowChannel implements PlayerChannel {
  _SlowChannel(this._link);

  final _SlowWifi _link;

  @override
  Future<void> dispose() async => _link.disposed++;

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}
