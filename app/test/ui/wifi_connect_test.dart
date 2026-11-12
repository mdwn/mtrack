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
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/wifi_address.dart';
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';

Finder button(String label) => find.byWidgetPredicate(
  (w) => w is ChunkyButton && w.semanticLabel == label,
);

Finder addressField() => find.widgetWithText(TextField, 'Player address');

const String ip = '192.168.1.40';

WifiAddress parse(String text) => parseWifiAddress(text);

void setupNetwork(AppHarness h) {
  h.rig.networkInfo = pp.GetNetworkInfoResponse(
    hostname: 'stage',
    addresses: [ip],
    grpcPort: 43234,
  );
  h.wifi!.reachable.add(ip);
}

/// Types [text] in the address field and presses Connect.
Future<void> enter(AppHarness h, String text) async {
  await h.tester.tap(button('Enter an address'));
  await h.settle();
  await h.tester.enterText(addressField(), text);
  await h.tester.tap(button('Connect to this address'));
  await h.settle();
  await h.tester.pump(const Duration(milliseconds: 100));
}

void main() {
  group('entering an address', () {
    appTest(
      'is offered beside the scan, with its own label',
      (h) async {
        expect(button('Enter an address'), findsOneWidget);
        expect(addressField(), findsNothing);
        await h.tester.tap(button('Enter an address'));
        await h.settle();
        expect(addressField(), findsOneWidget);
        expect(find.textContaining('mtrack.local'), findsWidgets);
        expect(find.textContaining('43234'), findsOneWidget);
        expect(button('Connect to this address'), findsOneWidget);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'is offered when Bluetooth is off, since it needs none',
      (h) async {
        h.rig.link.adapter = AdapterState.off;
        await h.session.startScan();
        await h.settle();
        expect(find.text('Bluetooth is off'), findsOneWidget);
        expect(button('Enter an address'), findsOneWidget);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'asks for an address when none is typed',
      (h) async {
        await enter(h, '   ');
        expect(
          find.textContaining("Type the player's address"),
          findsOneWidget,
        );
        expect(h.wifi!.attempts, isEmpty);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'says what is wrong with an address, and clears it on typing',
      (h) async {
        await enter(h, 'mtrack.local:99999');
        expect(find.textContaining('1 to 65535'), findsOneWidget);
        expect(h.wifi!.attempts, isEmpty);
        await h.tester.enterText(addressField(), 'mtrack.local');
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 400));
        expect(find.textContaining('1 to 65535'), findsNothing);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'connects to the address, remembers it as a Wi-Fi player and shows '
      'the tabs',
      (h) async {
        setupNetwork(h);
        await enter(h, ip);
        expect(find.text('Find your player'), findsNothing);
        expect(find.text('READY · 1 OF 3'), findsOneWidget);
        expect(h.wifi!.attempts, ['$ip:43234']);
        expect(h.store.player!.bluetooth, isFalse);
        expect(h.store.player!.name, 'stage');
        expect(h.rig.link.calls.where((c) => c.startsWith('connect')), isEmpty);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'takes a port after the address',
      (h) async {
        setupNetwork(h);
        h.wifi!.reachable.add('mtrack.local');
        await enter(h, 'mtrack.local:50051');
        expect(h.wifi!.attempts.first, 'mtrack.local:50051');
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'says plainly when nothing answers, and offers the choices '
      'again',
      (h) async {
        await enter(h, '10.9.9.9');
        await h.advance(const Duration(seconds: 3));
        await h.settle();
        expect(
          find.textContaining('Nothing answered at 10.9.9.9'),
          findsOneWidget,
        );
        expect(find.textContaining("player's network"), findsOneWidget);
        expect(find.text('Find your player'), findsOneWidget);
        expect(
          button('Enter an address'),
          findsNothing,
          reason: 'the typed address is still there to correct',
        );
        expect(addressField(), findsOneWidget);
        expect(h.store.player, isNull);
      },
      known: false,
      withWifi: true,
    );

    appTest(
      'shows connecting to the address while it waits',
      (h) async {
        await h.tester.tap(button('Enter an address'));
        await h.settle();
        await h.tester.enterText(addressField(), '10.9.9.9');
        await h.tester.tap(button('Connect to this address'));
        await h.settle();
        expect(find.text('Connecting to 10.9.9.9…'), findsOneWidget);
        expect(button('Cancel'), findsOneWidget);
        await h.advance(const Duration(seconds: 3));
      },
      known: false,
      withWifi: true,
    );
  });

  group('which radio the app says', () {
    appTest('says Wi-Fi in the bar and the settings when on Wi-Fi', (h) async {
      setupNetwork(h);
      await h.session.forget();
      await h.settle();
      await h.session.connectToAddress(parse(ip));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.bySemanticsLabel('Connected over Wi-Fi'), findsOneWidget);
      expect(find.byIcon(Icons.wifi), findsOneWidget);
      expect(find.byIcon(Icons.bluetooth), findsNothing);
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Connected over Wi-Fi'), findsOneWidget);
    }, withWifi: true);

    appTest('says Bluetooth in the bar when on Bluetooth', (h) async {
      expect(find.bySemanticsLabel('Connected over Bluetooth'), findsOneWidget);
      expect(find.byIcon(Icons.bluetooth), findsOneWidget);
      expect(find.byIcon(Icons.wifi), findsNothing);
    });

    appTest('says not connected when the link is lost', (h) async {
      setupNetwork(h);
      await h.session.forget();
      await h.session.connectToAddress(parse(ip));
      await h.settle();
      h.wifi!.live!.drop();
      await h.settle();
      expect(find.bySemanticsLabel('Connected over Wi-Fi'), findsNothing);
      expect(find.byIcon(Icons.wifi), findsNothing);
      await h.advance(const Duration(seconds: 1));
    }, withWifi: true);

    appTest('forgetting a player known by address goes back to finding one', (
      h,
    ) async {
      setupNetwork(h);
      await h.session.forget();
      await h.session.connectToAddress(parse(ip));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      await h.tester.tap(button('Forget this player'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(h.store.player, isNull);
      expect(find.text('Find your player'), findsOneWidget);
      expect(button('Enter an address'), findsOneWidget);
    }, withWifi: true);

    appTest(
      'a player paired by Bluetooth that is on the network moves to Wi-Fi '
      'without the person doing anything',
      (h) async {
        expect(find.byIcon(Icons.bluetooth), findsOneWidget);
        h.wifi!.reachable.add(ip);
        await h.advance(wifiRetryInterval + const Duration(seconds: 1));
        await h.settle();
        await h.tester.pump(const Duration(seconds: 1));
        expect(find.byIcon(Icons.wifi), findsOneWidget);
        expect(find.byIcon(Icons.bluetooth), findsNothing);
      },
      withWifi: true,
      setup: (rig) => rig.networkInfo = pp.GetNetworkInfoResponse(
        hostname: 'stage',
        addresses: [ip],
        grpcPort: 43234,
      ),
    );
  });
}
