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
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';

Finder button(String label) => find.byWidgetPredicate(
  (w) => w is ChunkyButton && w.semanticLabel == label,
);

const String ip = '192.168.1.40';

const FoundPlayer stage = FoundPlayer(
  name: 'stage',
  hosts: ['fe80::1', ip],
  port: 43234,
  version: '0.18.3',
);

const FoundPlayer rehearsal = FoundPlayer(
  name: 'rehearsal',
  hosts: ['192.168.1.41'],
  port: 43234,
);

const DiscoveredPlayer paired = DiscoveredPlayer(
  id: 'p1',
  name: 'stage',
  rssi: -45,
);

void setupNetwork(AppHarness h) {
  h.rig.networkInfo = pp.GetNetworkInfoResponse(
    hostname: 'stage',
    addresses: [ip],
    grpcPort: 43234,
  );
  h.wifi!.reachable.add(ip);
}

void main() {
  group('the list of players', () {
    appTest(
      'shows Wi-Fi players as they are found, with the first IPv4 address, '
      'and drops one that is lost',
      (h) async {
        expect(find.text('Searching…'), findsOneWidget);
        h.browser!.appear(stage);
        h.browser!.appear(rehearsal);
        await h.settle();
        expect(find.text('stage'), findsOneWidget);
        expect(find.text('rehearsal'), findsOneWidget);
        expect(find.text('Wi-Fi'), findsNWidgets(2));
        expect(find.text(ip), findsOneWidget);
        expect(find.text('192.168.1.41'), findsOneWidget);
        expect(find.textContaining('fe80'), findsNothing);
        expect(
          h.tester.getTopLeft(find.text('rehearsal')).dy,
          lessThan(h.tester.getTopLeft(find.text('stage')).dy),
        );
        h.browser!.vanish('rehearsal');
        await h.settle();
        expect(find.text('rehearsal'), findsNothing);
        expect(find.text('stage'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'shows Wi-Fi and Bluetooth rows together, and the same player found '
      'both ways as two rows',
      (h) async {
        h.browser!.appear(stage);
        h.rig.link.discover(paired);
        await h.settle();
        expect(find.text('stage'), findsNWidgets(2));
        expect(find.text('Wi-Fi'), findsOneWidget);
        expect(find.text('Bluetooth'), findsOneWidget);
        expect(find.text('strong'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'gives every row and button a label for a screen reader',
      (h) async {
        h.browser!.appear(stage);
        h.rig.link.discover(paired);
        await h.settle();
        expect(find.bySemanticsLabel('stage, Wi-Fi, $ip'), findsOneWidget);
        expect(
          find.bySemanticsLabel('stage, Bluetooth, strong signal'),
          findsOneWidget,
        );
        expect(button('Enter an address'), findsOneWidget);
        expect(button('Look again'), findsNothing);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'says Searching… while either search runs, then says so when nothing '
      'is found',
      (h) async {
        expect(find.text('Searching…'), findsOneWidget);
        await h.rig.link.endScan();
        await h.settle();
        expect(
          find.text('Searching…'),
          findsOneWidget,
          reason: 'the network is still being listened to',
        );
        await h.advance(browseSearchWindow + const Duration(seconds: 1));
        await h.settle();
        expect(find.text('Searching…'), findsNothing);
        expect(find.textContaining('No players found'), findsOneWidget);
        expect(button('Look again'), findsOneWidget);
        expect(find.textContaining('Could not search'), findsNothing);
        // A late answer is added to the list.
        h.browser!.appear(stage);
        await h.settle();
        expect(find.textContaining('No players found'), findsNothing);
        expect(find.text('stage'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'looking again searches again',
      (h) async {
        await h.rig.link.endScan();
        await h.advance(browseSearchWindow + const Duration(seconds: 1));
        await h.settle();
        await h.tester.tap(button('Look again'));
        await h.settle();
        expect(find.text('Searching…'), findsOneWidget);
        expect(h.rig.link.calls.where((c) => c == 'scan'), hasLength(2));
      },
      known: false,
      withBrowser: true,
    );
  });

  group('without Bluetooth', () {
    appTest(
      'still lists Wi-Fi players when Bluetooth is off',
      (h) async {
        h.rig.link.adapter = AdapterState.off;
        await h.session.startScan();
        h.browser!.appear(stage);
        await h.settle();
        expect(find.text('Bluetooth is off'), findsOneWidget);
        expect(find.text('stage'), findsOneWidget);
        expect(find.text('Wi-Fi'), findsOneWidget);
        expect(find.text('Searching…'), findsOneWidget);
        expect(button('Enter an address'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'still lists Wi-Fi players when Bluetooth is refused',
      (h) async {
        h.rig.link.adapter = AdapterState.permissionDenied;
        await h.session.startScan();
        h.browser!.appear(stage);
        await h.settle();
        expect(find.text('mtrack needs Bluetooth'), findsOneWidget);
        expect(button('Allow Bluetooth'), findsOneWidget);
        expect(find.text('stage'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );
  });

  group('when the network cannot be searched', () {
    appTest(
      'says so in a line, and Bluetooth and an address still work',
      (h) async {
        h.browser!.fail(StateError('refused'));
        h.rig.link.discover(paired);
        await h.settle();
        expect(find.text('Could not search the network.'), findsOneWidget);
        expect(find.text('stage'), findsOneWidget);
        expect(find.text('Bluetooth'), findsOneWidget);
        expect(button('Enter an address'), findsOneWidget);
        await h.tester.tap(find.text('stage'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 100));
        expect(find.text('READY · 1 OF 3'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'a plugin that throws does not break the screen',
      (h) async {
        // The first browse ends, so that the next one has to start afresh.
        h.browser!.fail(StateError('ended'));
        await h.settle();
        h.browser!.throwOnBrowse = StateError('no plugin');
        await h.session.startScan();
        await h.settle();
        expect(find.text('Could not search the network.'), findsOneWidget);
        expect(find.text('Find your player'), findsOneWidget);
        expect(button('Enter an address'), findsOneWidget);
      },
      known: false,
      withBrowser: true,
    );

    appTest(
      'says nothing about it when there is just no network',
      (h) async {
        await h.rig.link.endScan();
        await h.advance(browseSearchWindow + const Duration(seconds: 1));
        await h.settle();
        expect(find.textContaining('Could not'), findsNothing);
      },
      known: false,
      withBrowser: true,
    );
  });

  group('choosing a Wi-Fi player', () {
    appTest(
      'connects on a tap with no pairing, remembers it, and shows the tabs',
      (h) async {
        setupNetwork(h);
        h.browser!.appear(stage);
        await h.settle();
        await h.tester.tap(find.text('stage'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 100));
        expect(find.text('Find your player'), findsNothing);
        expect(find.text('READY · 1 OF 3'), findsOneWidget);
        expect(find.byIcon(Icons.wifi), findsOneWidget);
        expect(h.rig.link.calls.where((c) => c.startsWith('connect')), isEmpty);
        expect(h.rig.link.calls, isNot(contains('pair:p1')));
        expect(h.store.player!.mdnsName, 'stage');
        expect(h.store.player!.bluetooth, isFalse);
        expect(h.browser!.browsing, isFalse);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
    );

    appTest(
      'a player that does not answer goes back to the list with the reason',
      (h) async {
        h.browser!.appear(
          const FoundPlayer(name: 'ghost', hosts: ['10.9.9.9'], port: 43234),
        );
        await h.settle();
        await h.tester.tap(find.text('ghost'));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 100));
        expect(find.text('Connecting to ghost…'), findsOneWidget);
        await h.advance(const Duration(seconds: 3));
        await h.settle();
        await h.tester.pump(const Duration(milliseconds: 100));
        expect(
          find.textContaining('Nothing answered at 10.9.9.9'),
          findsOneWidget,
        );
        expect(find.text('Find your player'), findsOneWidget);
      },
      known: false,
      withWifi: true,
      withBrowser: true,
    );
  });

  group('the buttons', () {
    appTest(
      'keep "Enter an address" in a row, with no scan button when the app cannot scan',
      (h) async {
        expect(
          find.ancestor(
            of: button('Enter an address'),
            matching: find.byType(Row),
          ),
          findsOneWidget,
        );
        expect(find.textContaining('Scan this network'), findsNothing);
      },
      known: false,
      withBrowser: true,
    );
  });

  group('the list fits', () {
    for (final look in Look.all) {
      appTest(
        'a phone with long names and both kinds of row, ${look.toString()}',
        (h) async {
          h.browser!.appear(
            const FoundPlayer(
              name: 'the-main-stage-rack-player-with-a-long-name',
              hosts: ['2001:db8:1234:5678:9abc:def0:1234:5678'],
              port: 43234,
            ),
          );
          h.rig.link.discover(
            const DiscoveredPlayer(
              id: 'p9',
              name: 'rehearsal room upstairs and then some more',
              rssi: -88,
            ),
          );
          await h.settle();
          expect(find.text('Bluetooth'), findsOneWidget);
          expect(button('Enter an address'), findsOneWidget);
        },
        known: false,
        withBrowser: true,
        look: look,
      );
    }
  });
}
