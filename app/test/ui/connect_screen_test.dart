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

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/ui/widgets.dart';

import '../support/app_harness.dart';

Finder button(String label) => find.byWidgetPredicate(
  (w) => w is ChunkyButton && w.semanticLabel == label,
);

const DiscoveredPlayer near = DiscoveredPlayer(
  id: 'p1',
  name: 'stage',
  rssi: -45,
);
const DiscoveredPlayer far = DiscoveredPlayer(
  id: 'p2',
  name: 'rehearsal room',
  rssi: -88,
);

void main() {
  group('finding a player', () {
    appTest('shows the first-time hint while it looks', (h) async {
      expect(find.text('Find your player'), findsOneWidget);
      expect(find.text('Looking for players…'), findsOneWidget);
      expect(find.textContaining('First time?'), findsOneWidget);
      expect(find.textContaining('Pair a phone'), findsOneWidget);
      expect(find.textContaining('code while pairing'), findsOneWidget);
      expect(find.text('Not connected'), findsOneWidget);
    }, known: false);

    appTest('lists what it finds with a coarse signal', (h) async {
      h.rig.link.discover(far);
      h.rig.link.discover(near);
      await h.settle();
      expect(find.text('stage'), findsOneWidget);
      expect(find.text('rehearsal room'), findsOneWidget);
      expect(find.text('strong'), findsOneWidget);
      expect(find.text('weak'), findsOneWidget);
      expect(
        h.tester.getTopLeft(find.text('stage')).dy,
        lessThan(h.tester.getTopLeft(find.text('rehearsal room')).dy),
      );
    }, known: false);

    appTest('says so when nothing is found, and looks again', (h) async {
      await h.rig.link.endScan();
      await h.settle();
      expect(find.textContaining('No players found'), findsOneWidget);
      await h.tester.tap(button('Look again'));
      await h.settle();
      expect(find.text('Looking for players…'), findsOneWidget);
      expect(h.rig.link.calls.where((c) => c == 'scan'), hasLength(2));
    }, known: false);

    appTest('connects to the player tapped, and shows the tabs', (h) async {
      h.rig.link.discover(near);
      await h.settle();
      await h.tester.tap(find.text('stage'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Find your player'), findsNothing);
      expect(find.text('READY · 1 OF 3'), findsOneWidget);
      expect(h.store.player?.id, 'p1');
    }, known: false);

    appTest('says Bluetooth is off, and tries again', (h) async {
      h.rig.link.adapter = AdapterState.off;
      await h.session.startScan();
      await h.settle();
      expect(find.text('Bluetooth is off'), findsOneWidget);
      expect(find.text('Looking for players…'), findsNothing);
      h.rig.link.adapter = AdapterState.ready;
      await h.tester.tap(button('Try again'));
      await h.settle();
      expect(find.text('Bluetooth is off'), findsNothing);
      expect(find.text('Looking for players…'), findsOneWidget);
    }, known: false);

    appTest('asks again for permission when it was refused', (h) async {
      h.rig.link.adapter = AdapterState.permissionDenied;
      await h.session.startScan();
      await h.settle();
      expect(find.text('mtrack needs Bluetooth'), findsOneWidget);
      h.rig.link.adapter = AdapterState.ready;
      await h.tester.tap(button('Allow Bluetooth'));
      await h.settle();
      expect(h.rig.link.calls, contains('requestAccess'));
      expect(find.text('mtrack needs Bluetooth'), findsNothing);
      expect(find.text('Looking for players…'), findsOneWidget);
    }, known: false);

    appTest('has no retry for a phone without Bluetooth Low Energy', (h) async {
      h.rig.link.adapter = AdapterState.unsupported;
      await h.session.startScan();
      await h.settle();
      expect(find.text('No Bluetooth Low Energy'), findsOneWidget);
      expect(button('Try again'), findsNothing);
    }, known: false);

    appTest('explains a first connection that failed', (h) async {
      h.rig.link.connectFailures = 1;
      h.rig.link.discover(near);
      await h.settle();
      await h.tester.tap(find.text('stage'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(
        find.textContaining('Could not reach that player'),
        findsOneWidget,
      );
      expect(find.text('Find your player'), findsOneWidget);
    }, known: false);
  });

  group('pairing', () {
    appTest('shows pairing in progress and how to cancel it', (h) async {
      h.rig.link.paired.clear();
      h.rig.link.pairingHeld = Completer<void>();
      h.rig.link.discover(near);
      await h.settle();
      await h.tester.tap(find.text('stage'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Pairing with stage…'), findsOneWidget);
      expect(
        find.text('Your phone may show a code. Confirm it.'),
        findsOneWidget,
      );
      await h.tester.tap(button('Cancel'));
      await h.settle();
      expect(find.text('Find your player'), findsOneWidget);
      h.rig.link.pairingHeld!.complete();
      await h.settle();
    }, known: false);

    appTest('says the player is not accepting new phones, and how to fix it', (
      h,
    ) async {
      h.rig.link.paired.clear();
      h.rig.link.pairingRefused = true;
      h.rig.link.discover(near);
      await h.settle();
      await h.tester.tap(find.text('stage'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(
        find.text('This player is not accepting new phones'),
        findsOneWidget,
      );
      expect(find.textContaining('Config → Controllers'), findsOneWidget);
      expect(find.textContaining('Pair a phone'), findsOneWidget);
      h.rig.link.pairingRefused = false;
      await h.tester.tap(button('Try again'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      expect(find.text('READY · 1 OF 3'), findsOneWidget);
    }, known: false);

    appTest('lets the person choose another player after a refusal', (h) async {
      h.rig.link.paired.clear();
      h.rig.link.pairingRefused = true;
      h.rig.link.discover(near);
      await h.settle();
      await h.tester.tap(find.text('stage'));
      await h.settle();
      await h.tester.tap(button('Choose another player'));
      await h.settle();
      expect(find.text('Find your player'), findsOneWidget);
      expect(
        find.text('This player is not accepting new phones'),
        findsNothing,
      );
    }, known: false);
  });

  group('a remembered player', () {
    appTest('is looked for, and can be given up on', (h) async {
      expect(find.text('Looking for stage…'), findsOneWidget);
      expect(find.text('READY · 1 OF 3'), findsNothing);
      await h.tester.tap(button('Choose another player'));
      await h.settle();
      expect(find.text('Find your player'), findsOneWidget);
      expect(h.store.player, isNull);
    }, setup: (rig) => rig.link.connectFailures = 100);

    appTest('connects by itself once it is in reach', (h) async {
      expect(find.text('Looking for stage…'), findsOneWidget);
      h.rig.link.connectFailures = 0;
      await h.advance(const Duration(seconds: 1));
      await h.settle();
      expect(find.text('READY · 1 OF 3'), findsOneWidget);
    }, setup: (rig) => rig.link.connectFailures = 1);

    appTest('keeps the last state on show when the link drops', (h) async {
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.advance(const Duration(seconds: 1));
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      await h.advance(const Duration(seconds: 30));
    });
  });
}
