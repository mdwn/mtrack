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
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';

Finder button(String label) => find.byWidgetPredicate(
  (w) => w is ChunkyButton && w.semanticLabel == label,
);

const String ip = '192.168.1.40';

void setupNetwork(AppHarness h) {
  h.rig.networkInfo = pp.GetNetworkInfoResponse(
    hostname: 'stage',
    addresses: [ip],
    grpcPort: 43234,
  );
  h.wifi!.reachable.add(ip);
  h.probe!.open.add('$ip:43234');
}

/// Lets a whole scan run its course.
Future<void> finishScan(AppHarness h) async {
  for (var i = 0; i < 14; i++) {
    await h.advance(const Duration(seconds: 1));
  }
}

void main() {
  group('the scan button', () {
    appTest(
      'sits beside "Enter an address" in the same row, and scans nothing '
      'until pressed',
      (h) async {
        await finishScan(h);
        expect(button('Scan this network'), findsOneWidget);
        expect(button('Enter an address'), findsOneWidget);
        final row = find.ancestor(
          of: button('Enter an address'),
          matching: find.byType(Row),
        );
        expect(row, findsOneWidget);
        expect(
          find.descendant(of: row, matching: button('Scan this network')),
          findsOneWidget,
        );
        expect(h.probe!.listings, 0);
        expect(h.probe!.attempts, isEmpty);
        expect(find.textContaining('Scanning'), findsNothing);
        expect(find.textContaining('on this network'), findsNothing);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'is not there when the app cannot scan',
      (h) async {
        expect(button('Scan this network'), findsNothing);
        expect(button('Enter an address'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      '"Look again" does not scan',
      (h) async {
        await h.rig.link.endScan();
        await finishScan(h);
        await h.tester.tap(button('Look again'));
        await h.settle();
        await finishScan(h);
        expect(h.probe!.listings, 0);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'is disabled while scanning, says so, and a second press does nothing',
      (h) async {
        setupNetwork(h);
        h.probe!.open.clear();
        await h.tester.tap(button('Scan this network'));
        await h.settle();
        await h.advance(const Duration(milliseconds: 100));
        expect(button('Scanning this network'), findsOneWidget);
        expect(find.text('Scanning…'), findsOneWidget);
        expect(find.text('Scanning this network…'), findsOneWidget);
        final scanning = h.tester.widget<ChunkyButton>(
          button('Scanning this network'),
        );
        expect(scanning.onPressed, isNull);
        await h.tester.tap(button('Scanning this network'));
        await h.settle();
        expect(h.probe!.listings, 1);
        expect(h.probe!.maxInFlight, 32);
        await finishScan(h);
        expect(button('Scan this network'), findsOneWidget);
        expect(find.text('Scanning this network…'), findsNothing);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'is out of the way while an address is being typed',
      (h) async {
        await h.tester.tap(button('Enter an address'));
        await h.settle();
        expect(button('Scan this network'), findsNothing);
        expect(button('Connect to this address'), findsOneWidget);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );
  });

  group('what a scan says', () {
    appTest(
      'says none were found on an empty network, and keeps "Look again"',
      (h) async {
        h.probe!.open.clear();
        await h.rig.link.endScan();
        await h.tester.tap(button('Scan this network'));
        await finishScan(h);
        expect(find.text('No players found on this network.'), findsOneWidget);
        expect(find.textContaining('Make sure the player is on'), findsNothing);
        expect(button('Look again'), findsOneWidget);
        expect(button('Scan this network'), findsOneWidget);
        expect(
          h.tester.widget<ChunkyButton>(button('Scan this network')).onPressed,
          isNotNull,
        );
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'says the phone is not on a network it can scan, and sweeps nothing',
      (h) async {
        h.probe!.addresses = const [LocalAddress('wlan0', '100.64.3.3')];
        await h.tester.tap(button('Scan this network'));
        await h.settle();
        await finishScan(h);
        expect(
          find.textContaining('This phone is not on a network it can scan.'),
          findsOneWidget,
        );
        expect(h.probe!.attempts, isEmpty);
        expect(button('Scan this network'), findsOneWidget);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'says the same with Wi-Fi off',
      (h) async {
        h.probe!.addresses = const [LocalAddress('rmnet_data0', '10.1.2.3')];
        await h.tester.tap(button('Scan this network'));
        await h.settle();
        expect(
          find.textContaining('This phone is not on a network it can scan.'),
          findsOneWidget,
        );
        expect(find.byType(AlertDialog), findsNothing);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'shows what it finds as a Wi-Fi row named by its host name, and a tap '
      'connects',
      (h) async {
        setupNetwork(h);
        await h.tester.tap(button('Scan this network'));
        await finishScan(h);
        expect(find.text('stage'), findsOneWidget);
        expect(find.text('Wi-Fi'), findsOneWidget);
        expect(find.text(ip), findsOneWidget);
        expect(find.bySemanticsLabel('stage, Wi-Fi, $ip'), findsOneWidget);
        expect(find.textContaining('No players found'), findsNothing);
        await h.tester.tap(find.text('stage'));
        await h.settle();
        await h.advance(const Duration(milliseconds: 300));
        expect(find.text('Find your player'), findsNothing);
        expect(h.session.phase.name, 'ready');
        expect(h.store.player!.id, 'wifi:$ip:43234');
        expect(h.store.player!.name, 'stage');
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );

    appTest(
      'lists a player the network advertises once',
      (h) async {
        setupNetwork(h);
        h.browser!.appear(
          const FoundPlayer(name: 'stage', hosts: [ip], port: 43234),
        );
        await h.settle();
        await h.tester.tap(button('Scan this network'));
        await finishScan(h);
        expect(find.text('stage'), findsOneWidget);
        expect(find.text('Wi-Fi'), findsOneWidget);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
      withProbe: true,
    );
  });

  group('the screen fits', () {
    for (final look in Look.all) {
      appTest(
        'with the scan button, a long name and each line, ${look.toString()}',
        (h) async {
          setupNetwork(h);
          h.rig.networkInfo = pp.GetNetworkInfoResponse(
            hostname: 'the-main-stage-rack-player-with-a-long-name',
            grpcPort: 43234,
          );
          expect(button('Scan this network'), findsOneWidget);
          expect(button('Enter an address'), findsOneWidget);
          await h.tester.tap(button('Scan this network'));
          await h.settle();
          await h.advance(const Duration(milliseconds: 100));
          expect(find.text('Scanning this network…'), findsOneWidget);
          expect(button('Scanning this network'), findsOneWidget);
          await finishScan(h);
          expect(find.textContaining('the-main-stage'), findsOneWidget);
          h.probe!.addresses = const [LocalAddress('wlan0', '8.8.8.8')];
          await h.tester.tap(button('Scan this network'));
          await h.settle();
          expect(
            find.textContaining('not on a network it can scan'),
            findsOneWidget,
          );
          h.probe!.addresses = const [LocalAddress('wlan0', '192.168.1.20')];
          h.probe!.open.clear();
          h.wifi!.reachable.clear();
          await h.tester.tap(button('Scan this network'));
          await finishScan(h);
          expect(
            find.text('No players found on this network.'),
            findsOneWidget,
          );
        },
        known: false,
        withWifi: true,
        withBrowser: true,
        withProbe: true,
        look: look,
      );
    }
  });
}
