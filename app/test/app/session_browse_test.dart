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
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/wifi_address.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';

import '../support/session_harness.dart';

const String ip = '192.168.1.40';

const FoundPlayer stage = FoundPlayer(
  name: 'stage',
  hosts: [ip],
  port: 43234,
  version: '0.18.3',
);

const FoundPlayer rehearsal = FoundPlayer(
  name: 'rehearsal',
  hosts: ['192.168.1.41'],
  port: 43234,
);

pp.GetNetworkInfoResponse info() => pp.GetNetworkInfoResponse(
  hostname: 'stage',
  addresses: [ip],
  grpcPort: 43234,
);

void main() {
  /// Runs [body] with a harness that has a network and a browser, in fake
  /// time, with no player remembered unless [player] says so, and checks
  /// nothing is left running after the session is disposed.
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
          player: player,
          known: player != null,
        );
        h.rig.networkInfo = info();
        h.wifi!.reachable.add(ip);
        body(h, async);
        h.session.dispose();
        h.settle();
        expect(h.browser!.browsing, isFalse);
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  group('ordering a player\'s addresses', () {
    test('puts IPv4 first, link-local IPv6 last, and keeps the rest', () {
      const found = FoundPlayer(
        name: 'stage',
        hosts: ['fe80::1', '2001:db8::7', '10.0.0.7', 'stage.local', ip],
        port: 43234,
      );
      expect(found.orderedHosts, [
        '10.0.0.7',
        ip,
        'stage.local',
        '2001:db8::7',
        'fe80::1',
      ]);
    });
  });

  group('browsing the network', () {
    scenario('starts with the scan, lists what answers by name, and drops '
        'what is lost', (h, _) {
      h.session.startScan();
      h.settle();
      expect(h.browser!.starts, 1);
      expect(h.session.searching, isTrue);
      h.browser!.appear(stage);
      h.browser!.appear(rehearsal);
      expect(h.session.wifiFound.map((p) => p.name), ['rehearsal', 'stage']);
      expect(h.session.wifiFound.last.version, '0.18.3');
      h.browser!.vanish('rehearsal');
      expect(h.session.wifiFound.map((p) => p.name), ['stage']);
      expect(h.session.browseFailed, isFalse);
    });

    scenario('says it is searching for a few seconds, and goes on listening', (
      h,
      _,
    ) {
      h.session.startScan();
      h.settle();
      h.link.endScan();
      h.settle();
      expect(h.session.scanning, isFalse);
      expect(h.session.searching, isTrue, reason: 'the network is still new');
      h.elapse(browseSearchWindow);
      expect(h.session.searching, isFalse);
      h.browser!.appear(stage);
      expect(h.session.wifiFound, hasLength(1), reason: 'late ones still show');
    });

    scenario('looking again keeps what is found and starts the window over', (
      h,
      _,
    ) {
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      h.link.endScan();
      h.elapse(browseSearchWindow);
      expect(h.session.searching, isFalse);
      h.session.startScan();
      h.settle();
      expect(h.browser!.starts, 1, reason: 'one browse, not two');
      expect(h.session.searching, isTrue);
      expect(h.session.wifiFound, hasLength(1));
    });

    scenario('works with Bluetooth off', (h, _) {
      h.link.adapter = AdapterState.off;
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      expect(h.session.adapter, AdapterState.off);
      expect(h.link.calls, isNot(contains('scan')));
      expect(h.session.wifiFound, [stage]);
      expect(h.session.searching, isTrue);
    });

    scenario('works with Bluetooth refused', (h, _) {
      h.link.adapter = AdapterState.permissionDenied;
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      expect(h.session.adapter, AdapterState.permissionDenied);
      expect(h.session.wifiFound, [stage]);
    });

    scenario('works with no Bluetooth at all', (h, _) {
      h.link.adapter = AdapterState.unsupported;
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      expect(h.session.wifiFound, [stage]);
    });

    scenario('a browser that cannot start leaves Bluetooth and the rest '
        'working', (h, _) {
      h.browser!.failOnStart = StateError('refused');
      h.session.startScan();
      h.settle();
      expect(h.session.browseFailed, isTrue);
      expect(h.session.wifiFound, isEmpty);
      expect(h.session.searching, isTrue, reason: 'Bluetooth is scanning');
      h.link.discover(const DiscoveredPlayer(id: 'a', name: 'near', rssi: -40));
      h.settle();
      expect(h.session.found, hasLength(1));
      expect(h.session.connectError, isNull);
    });

    scenario('a browser that throws is a failure to browse, not a crash', (
      h,
      _,
    ) {
      h.browser!.throwOnBrowse = StateError('no plugin');
      h.session.startScan();
      h.settle();
      expect(h.session.browseFailed, isTrue);
      expect(h.session.scanning, isTrue);
    });

    scenario('a browse that fails part way clears the list, and looking again '
        'tries once more', (h, _) {
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      h.browser!.fail(StateError('lost the network service'));
      h.settle();
      expect(h.session.browseFailed, isTrue);
      expect(h.session.wifiFound, isEmpty);
      h.link.endScan();
      h.session.startScan();
      h.settle();
      expect(h.session.browseFailed, isFalse);
      expect(h.browser!.starts, 2);
      h.browser!.appear(stage);
      expect(h.session.wifiFound, [stage]);
    });

    scenario('does not browse for a session with no browser', (h, _) {
      // The harness has one; a session without is the Bluetooth-only app.
      final bare = Session(
        link: h.link,
        store: h.store,
        monotonic: () => h.async.elapsed,
      );
      bare.startScan();
      h.settle();
      expect(bare.wifiFound, isEmpty);
      expect(bare.browseFailed, isFalse);
      bare.dispose();
    });
  });

  group('when browsing stops', () {
    scenario('on choosing a Wi-Fi player', (h, _) {
      h.session.startScan();
      h.settle();
      h.browser!.appear(stage);
      h.session.connectToFound(stage);
      h.settle();
      expect(h.browser!.browsing, isFalse);
      expect(h.session.wifiFound, isEmpty);
      expect(h.session.searching, isFalse);
    });

    scenario('on choosing a Bluetooth player', (h, _) {
      h.session.startScan();
      h.settle();
      h.link.discover(
        const DiscoveredPlayer(id: 'p1', name: 'stage', rssi: -40),
      );
      h.settle();
      h.session.connectTo(h.session.found.first);
      h.settle();
      expect(h.browser!.browsing, isFalse);
    });

    scenario('on typing an address', (h, _) {
      h.session.startScan();
      h.settle();
      expect(h.browser!.browsing, isTrue);
      h.session.connectToAddress(const WifiAddress(ip, 43234));
      h.settle();
      expect(h.browser!.browsing, isFalse);
    });

    scenario('when a remembered player starts connecting under the screen', (
      h,
      _,
    ) {
      // The connect screen is up for the moments before the store answers.
      h.session.startScan();
      h.settle();
      expect(h.browser!.browsing, isTrue);
      h.session.start();
      h.settle();
      expect(h.browser!.browsing, isFalse);
      expect(h.link.scanning, isFalse);
    }, player: const RememberedPlayer(id: 'p1', name: 'stage'));

    scenario('on dispose', (h, _) {
      h.session.startScan();
      h.settle();
      expect(h.browser!.browsing, isTrue);
    });

    scenario('is not started once a player is connected', (h, _) {
      h.connect();
      h.session.startScan();
      h.settle();
      expect(h.browser!.starts, 0);
    }, player: const RememberedPlayer(id: 'p1', name: 'stage'));
  });

  group('connecting to a player found on the network', () {
    scenario('remembers it as a Wi-Fi player under its name, with no '
        'pairing', (h, _) {
      h.session.connectToFound(stage);
      h.settle();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.wifi);
      expect(h.wifi!.attempts, ['$ip:43234']);
      expect(h.link.calls, isEmpty);
      final kept = h.store.player!;
      expect(kept.id, 'mdns:stage');
      expect(kept.name, 'stage');
      expect(kept.mdnsName, 'stage');
      expect(kept.bluetooth, isFalse);
      expect(kept.hosts, [ip]);
      expect(kept.port, 43234);
    });

    scenario('tries IPv4 first and at most three addresses', (h, _) {
      const crowded = FoundPlayer(
        name: 'stage',
        hosts: [
          'fe80::1',
          '2001:db8::7',
          '10.0.0.7',
          '10.0.0.8',
          '10.0.0.9',
          '10.0.0.10',
        ],
        port: 50051,
      );
      h.session.connectToFound(crowded);
      h.elapse(wifiConnectTimeout * 3 + const Duration(milliseconds: 500));
      expect(h.wifi!.attempts, [
        '10.0.0.7:50051',
        '10.0.0.8:50051',
        '10.0.0.9:50051',
      ]);
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.session.connectError, contains('10.0.0.7'));
      expect(h.store.player, isNull);
      expect(
        h.browser!.starts,
        0,
        reason: 'a player just found is not looked for again',
      );
    });

    scenario('tries the later addresses when the first does not answer', (
      h,
      _,
    ) {
      const two = FoundPlayer(
        name: 'stage',
        hosts: ['10.9.9.9', ip],
        port: 43234,
      );
      h.session.connectToFound(two);
      h.elapse(wifiConnectTimeout);
      expect(h.wifi!.attempts, ['10.9.9.9:43234', '$ip:43234']);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.store.player!.hosts.first, ip);
    });
  });

  group('finding a remembered player again by its name', () {
    const moved = RememberedPlayer(
      id: 'mdns:stage',
      name: 'stage',
      hosts: ['192.168.1.99'],
      port: 43234,
      hostname: 'stage',
      bluetooth: false,
      mdnsName: 'stage',
    );

    scenario('browses for the name when no address answers, and remembers '
        'where it is now', (h, _) {
      h.browser!.present['stage'] = stage;
      h.browser!.answerAfter = const Duration(seconds: 1);
      h.connect();
      h.elapse(wifiConnectTimeout + const Duration(seconds: 2));
      expect(h.wifi!.attempts, ['192.168.1.99:43234', '$ip:43234']);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.wifi);
      final kept = h.store.player!;
      expect(kept.hosts.first, ip);
      expect(kept.mdnsName, 'stage');
      expect(kept.id, 'mdns:stage');
      expect(h.browser!.browsing, isFalse, reason: 'the browse was short');
    }, player: moved);

    scenario('does not browse when an address answers', (h, _) {
      h.wifi!.reachable.add('192.168.1.99');
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.browser!.starts, 0);
    }, player: moved);

    scenario('gives up after a few seconds if the name is not found, and '
        'goes on trying', (h, _) {
      h.connect();
      h.elapse(
        wifiConnectTimeout + refindWindow + const Duration(milliseconds: 500),
      );
      expect(h.browser!.starts, 1);
      expect(h.browser!.cancels, 1);
      expect(h.session.phase, ConnectionPhase.lost);
      expect(h.store.player, moved);
    }, player: moved);

    scenario('does not try again the addresses that just failed', (h, _) {
      h.browser!.present['stage'] = const FoundPlayer(
        name: 'stage',
        hosts: ['192.168.1.99'],
        port: 43234,
      );
      h.connect();
      h.elapse(wifiConnectTimeout + const Duration(milliseconds: 500));
      expect(h.wifi!.attempts, ['192.168.1.99:43234']);
    }, player: moved);

    scenario('a browse that fails is a miss, not an error', (h, _) {
      h.browser!.failOnStart = StateError('refused');
      h.connect();
      h.elapse(wifiConnectTimeout + const Duration(milliseconds: 500));
      expect(h.session.phase, ConnectionPhase.lost);
      expect(h.session.connectError, isNull);
    }, player: moved);

    scenario(
      'is not tried for a player typed by address',
      (h, _) {
        h.connect();
        h.elapse(wifiConnectTimeout);
        expect(h.browser!.starts, 0);
      },
      player: const RememberedPlayer(
        id: 'wifi:192.168.1.99:43234',
        name: 'stage',
        hosts: ['192.168.1.99'],
        port: 43234,
        bluetooth: false,
      ),
    );

    scenario('is ended by giving the player up', (h, async) {
      h.connect();
      h.elapse(wifiConnectTimeout + const Duration(milliseconds: 500));
      expect(h.browser!.browsing, isTrue);
      h.session.forget();
      h.settle();
      expect(h.browser!.browsing, isFalse);
    }, player: moved);
  });
}
