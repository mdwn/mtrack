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
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/ui/demo_banner.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';
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

final Finder tryTheDemo = find.text('Try the demo');
final Finder banner = find.text(demoBannerMessage);
final Finder leave = find.bySemanticsLabel('Leave the demo');

Future<void> enter(AppHarness h) async {
  await h.tester.tap(tryTheDemo);
  await h.settle();
  await h.tester.pump(const Duration(milliseconds: 50));
}

void main() {
  group('the connect screen', () {
    for (final adapter in AdapterState.values) {
      appTest(
        'offers the demo with Bluetooth ${adapter.name}',
        (h) async {
          expect(find.text('Find your player'), findsOneWidget);
          expect(tryTheDemo, findsOneWidget);
          await h.tester.tap(tryTheDemo);
          await h.settle();
          expect(banner, findsOneWidget);
          // It asked nothing of Bluetooth: no permission, no connection.
          expect(h.rig.link.calls, isNot(contains('requestAccess')));
          expect(
            h.rig.link.calls.where((c) => c.startsWith('connect')),
            isEmpty,
          );
        },
        known: false,
        setup: (rig) => rig.link.adapter = adapter,
      );
    }

    appTest('offers the demo beside a note about a refused pairing', (h) async {
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
      expect(tryTheDemo, findsOneWidget);
      await enter(h);
      expect(banner, findsOneWidget);
    }, known: false);

    appTest('offers the demo while looking for a player it cannot reach', (
      h,
    ) async {
      expect(find.textContaining('Looking for stage'), findsOneWidget);
      expect(tryTheDemo, findsOneWidget);
    }, setup: (rig) => rig.link.connectFailures = 1000);

    appTest('offers the demo while an address is being entered', (h) async {
      await h.tester.tap(button('Enter an address'));
      await h.settle();
      expect(find.text('Player address'), findsOneWidget);
      expect(tryTheDemo, findsOneWidget);
    }, known: false);

    appTest('makes the demo a quiet, big enough target', (h) async {
      final size = h.tester.getSize(
        find.ancestor(of: tryTheDemo, matching: find.byType(TextButton)).first,
      );
      expect(size.height, greaterThanOrEqualTo(48));
      expect(size.width, greaterThanOrEqualTo(48));
      expect(find.bySemanticsLabel('Try the demo'), findsOneWidget);
    }, known: false);

    for (final look in Look.all) {
      appTest(
        'fits a small phone, $look',
        (h) async {
          expect(tryTheDemo, findsOneWidget);
          await h.tester.ensureVisible(tryTheDemo);
          await enter(h);
          expect(banner, findsOneWidget);
          expect(find.text('Leave'), findsOneWidget);
        },
        known: false,
        look: look,
        size: const Size(320, 480),
      );
    }
  });

  group('in the demo', () {
    appTest('shows the tabs of a player, with nothing about an old player', (
      h,
    ) async {
      await enter(h);
      expect(h.session.inDemo, isTrue);
      for (final tab in ['Now', 'Library', 'Sections', 'Levels', 'Lights']) {
        expect(find.text(tab), findsWidgets, reason: tab);
      }
      expect(find.text('Harbor Lights'), findsWidgets);
      expect(find.text('Demo player'), findsOneWidget);
      expect(find.byIcon(demoIcon), findsWidgets);
      expect(find.bySemanticsLabel('Demo'), findsOneWidget);
      expect(find.bySemanticsLabel('Connected to the demo'), findsOneWidget);
      expect(find.textContaining('older mtrack'), findsNothing);
      expect(find.text('Reconnecting…'), findsNothing);
    }, known: false);

    appTest('says plainly on every tab that none of it is real', (h) async {
      await enter(h);
      for (final tab in ['Now', 'Library', 'Sections', 'Levels', 'Lights']) {
        await h.openTab(tab);
        expect(banner, findsOneWidget, reason: tab);
        expect(find.text('Leave'), findsOneWidget, reason: tab);
      }
      // It stays over a sheet.
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(banner, findsOneWidget);
    }, known: false);

    appTest('puts the banner under the status bar, and the screens under it', (
      h,
    ) async {
      // A status bar 24 logical pixels tall.
      h.tester.view.padding = const FakeViewPadding(top: 24 * 3);
      h.tester.view.viewPadding = const FakeViewPadding(top: 24 * 3);
      await enter(h);
      final bannerTop = h.tester.getTopLeft(banner).dy;
      expect(bannerTop, greaterThanOrEqualTo(24));
      final size = h.tester.getSize(
        find
            .ancestor(of: find.text('Leave'), matching: find.byType(InkWell))
            .first,
      );
      expect(size.height, greaterThanOrEqualTo(48));
      expect(size.width, greaterThanOrEqualTo(48));
      // The bar below starts under the banner, with no second gap for the
      // status bar the banner already stands clear of.
      final bannerBottom = h.tester
          .getBottomLeft(
            find.ancestor(of: banner, matching: find.byType(Material)).first,
          )
          .dy;
      final settings = h.tester.getTopLeft(find.byTooltip('Settings')).dy;
      expect(settings, greaterThanOrEqualTo(bannerBottom));
      expect(settings - bannerBottom, lessThan(24));
    }, known: false);

    for (final look in Look.all) {
      appTest(
        'is readable and fits, $look',
        (h) async {
          await enter(h);
          expect(banner, findsOneWidget);
          final text = h.tester.widget<Text>(banner);
          expect(text.style!.color, isNot(Colors.transparent));
          await h.openTab('Lights');
          await h.openTab('Now');
        },
        known: false,
        look: look,
        size: const Size(320, 480),
      );
    }

    appTest('leaves on Leave, back to finding a player', (h) async {
      await enter(h);
      await h.tester.tap(find.text('Leave'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(h.session.inDemo, isFalse);
      expect(banner, findsNothing);
      expect(find.text('Find your player'), findsOneWidget);
      expect(tryTheDemo, findsOneWidget);
      expect(h.store.player, isNull);
      expect(h.store.saves, 0);
    }, known: false);

    appTest('offers to leave the demo, not to forget the player', (h) async {
      appVersion = () async => '0.1.0 (1)';
      await enter(h);
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Demo player'), findsWidgets);
      expect(find.textContaining('simulated player'), findsOneWidget);
      expect(find.text('Forget this player'), findsNothing);
      expect(button('Leave the demo'), findsOneWidget);
      expect(find.textContaining('Connected over'), findsNothing);
      await h.tester.tap(button('Leave the demo'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(h.session.inDemo, isFalse);
      expect(find.text('Find your player'), findsOneWidget);
    }, known: false);

    appTest('leaves the remembered player as it was', (h) async {
      // The player is remembered but cannot be reached.
      final remembered = h.store.player;
      expect(find.textContaining('Looking for stage'), findsOneWidget);
      await enter(h);
      expect(banner, findsOneWidget);
      expect(h.store.player, same(remembered));
      await h.tester.tap(find.text('Leave'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(banner, findsNothing);
      expect(find.textContaining('stage'), findsWidgets);
      expect(h.store.player, same(remembered));
      expect(h.store.clears, 0);
    }, setup: (rig) => rig.link.connectFailures = 1000);

    appTest('plays: the Now tab runs a clock', (h) async {
      await enter(h);
      await h.tester.tap(button('Play'));
      await h.settle();
      expect(find.text('STOP'), findsOneWidget);
      await h.advance(const Duration(seconds: 3));
      await h.advance(const Duration(seconds: 1));
      expect(h.session.playing, isTrue);
      expect(h.session.clock.position.inSeconds, greaterThanOrEqualTo(3));
      await h.session.stop();
      await h.settle();
      expect(find.text('PLAY'), findsOneWidget);
    }, known: false);

    appTest('lists the demo\'s songs and playlists in the Library', (h) async {
      await enter(h);
      await h.openTab('Library');
      expect(find.text('Copper Wire'), findsWidgets);
      expect(find.text('Sundial'), findsWidgets);
    }, known: false);

    appTest('draws the lights: a layout is handed to the stage page', (
      h,
    ) async {
      await enter(h);
      await h.openTab('Lights');
      h.stage.surface.ready();
      await h.advance(const Duration(seconds: 1));
      await h.advance(const Duration(milliseconds: 500));
      final calls = h.stage.surface.calls;
      expect(calls.first, 'reset');
      expect(calls.where((c) => c.startsWith('L')), isNotEmpty);
      expect(calls.where((c) => c.startsWith('F')), isNotEmpty);
      // The layout the page was given is the demo's rig, with no models.
      final updates = [
        for (final m in RegExp(
          r"update\('([^']*)'\)",
        ).allMatches(h.stage.surface.scripts.join()))
          pp.StageUpdate.fromBuffer(base64.decode(m.group(1)!)),
      ];
      final layout = updates.firstWhere((u) => u.hasLayout()).layout;
      expect(layout.fixtures.length, inInclusiveRange(8, 12));
      expect(layout.fixtures.every((f) => !f.hasRig()), isTrue);
      expect(updates.where((u) => u.hasFrame()), isNotEmpty);
      expect(find.text('Ambient light'), findsOneWidget);
      expect(find.textContaining('Could not show'), findsNothing);
      expect(find.textContaining('older mtrack'), findsNothing);
      expect(find.textContaining('3D models'), findsNothing);
      // The ambient light follows its switch.
      await h.tester.tap(find.bySemanticsLabel('Ambient light Off'));
      await h.settle();
      expect(h.session.ambientMode, 'off');
    }, known: false);

    appTest('tells a locked player apart, and unlocking is allowed', (h) async {
      await enter(h);
      expect(find.text('Unlocked'), findsOneWidget);
      await h.session.setLock(true);
      await h.settle();
      expect(find.text('Locked'), findsOneWidget);
      expect(h.session.startEditing(), isNotNull);
      await h.session.setLock(false);
      await h.settle();
      expect(find.text('Unlocked'), findsOneWidget);
    }, known: false);

    appTest('has levels to move', (h) async {
      await enter(h);
      await h.openTab('Levels');
      expect(find.text('click'), findsWidgets);
      expect(find.text('cues'), findsWidgets);
    }, known: false);
  });
}
