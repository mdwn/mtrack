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
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/ui/licences.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';
import 'package:mtrack_remote/src/ui/theme.dart';

import '../support/app_harness.dart';

/// WCAG relative luminance of an opaque colour.
double luminance(Color c) {
  double channel(double v) =>
      v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4).toDouble();
  return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
}

/// The contrast ratio of [a] on [b], as WCAG states it.
double contrast(Color a, Color b) {
  final la = luminance(a);
  final lb = luminance(b);
  final hi = la > lb ? la : lb;
  final lo = la > lb ? lb : la;
  return (hi + 0.05) / (lo + 0.05);
}

/// A colour over a background, as a translucent tint is drawn.
Color over(Color tint, Color background) => Color.alphaBlend(tint, background);

void main() {
  group('contrast', () {
    for (final entry in {
      'dark': AppColors.dark,
      'light': AppColors.light,
    }.entries) {
      final c = entry.value;
      final name = entry.key;
      final onPage = {
        'text': c.text,
        'muted': c.muted,
        'dim': c.dim,
        'cyan': c.cyan,
        'amber': c.amber,
        'green': c.green,
        'red': c.red,
      };
      for (final text in onPage.entries) {
        test('$name: ${text.key} text on the page is at least 4.5:1', () {
          expect(contrast(text.value, c.page), greaterThanOrEqualTo(4.5));
        });
        test('$name: ${text.key} text on a card is at least 4.5:1', () {
          expect(contrast(text.value, c.surface), greaterThanOrEqualTo(4.5));
        });
      }
      test('$name: text on Play is at least 4.5:1', () {
        expect(contrast(c.onPlay, c.play), greaterThanOrEqualTo(4.5));
      });
      test('$name: text on Stop is at least 4.5:1, filled or not', () {
        expect(contrast(c.onStop, c.stop), greaterThanOrEqualTo(4.5));
        expect(contrast(c.onStop, c.stopFill), greaterThanOrEqualTo(4.5));
      });
      test('$name: text on the current and looping rows is at least 4.5:1', () {
        final current = over(c.currentTint, c.surface);
        final looping = over(c.loopTint, c.surface);
        expect(contrast(c.text, current), greaterThanOrEqualTo(4.5));
        expect(contrast(c.cyan, current), greaterThanOrEqualTo(4.5));
        expect(contrast(c.text, looping), greaterThanOrEqualTo(4.5));
        expect(contrast(c.amber, looping), greaterThanOrEqualTo(4.5));
      });
      test('$name: the reconnect banner text reads on its tint', () {
        expect(
          contrast(c.amber, over(c.loopTint, c.page)),
          greaterThanOrEqualTo(4.5),
        );
      });
      test('$name: muted text reads on raised surfaces', () {
        expect(contrast(c.muted, c.raised), greaterThanOrEqualTo(4.5));
      });
      test('$name: the progress fill and the page can be told apart', () {
        expect(contrast(c.cyan, c.raised), greaterThanOrEqualTo(3.0));
      });
    }
  });

  group('the theme', () {
    appTest('follows a dark phone', (h) async {
      final context = h.tester.element(find.byType(Scaffold).first);
      expect(Theme.of(context).brightness, Brightness.dark);
      expect(context.colors.page, AppColors.dark.page);
    });

    appTest('follows a light phone', (h) async {
      final context = h.tester.element(find.byType(Scaffold).first);
      expect(Theme.of(context).brightness, Brightness.light);
      expect(context.colors.page, AppColors.light.page);
    }, look: const Look(Brightness.light, 1.0));

    appTest('uses the bundled fonts', (h) async {
      final context = h.tester.element(find.byType(Scaffold).first);
      expect(Theme.of(context).textTheme.bodyMedium!.fontFamily, 'Inter');
      expect(AppText.songTitle(30).fontFamily, 'Nunito');
      expect(AppText.time(30).fontFamily, 'JetBrainsMono');
      expect(
        AppText.time(30).fontFeatures,
        contains(const FontFeature.tabularFigures()),
      );
    });
  });

  group('licences', () {
    test('the bundled fonts are on the licence page', () async {
      TestWidgetsFlutterBinding.ensureInitialized();
      registerFontLicences();
      final entries = await LicenseRegistry.licenses.toList();
      for (final font in ['Nunito', 'Inter', 'JetBrains Mono']) {
        final entry = entries.where((e) => e.packages.contains('$font font'));
        expect(entry, hasLength(1), reason: font);
        final text = entry.single.paragraphs.map((p) => p.text).join('\n');
        expect(text, contains('SIL OPEN FONT LICENSE Version 1.1'));
      }
    });
  });

  group('settings', () {
    setUp(() => appVersion = () async => '1.2.3 (4)');

    appTest('show the player, the version and the licences', (h) async {
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Settings'), findsWidgets);
      expect(find.text('Connected over Bluetooth'), findsOneWidget);
      expect(find.text('mtrack remote 1.2.3 (4)'), findsOneWidget);
      expect(find.text('Open-source licences'), findsOneWidget);
      await h.tester.tap(find.text('Open-source licences'));
      await h.tester.pump(const Duration(milliseconds: 500));
      await h.tester.pump(const Duration(seconds: 1));
      expect(find.byType(LicensePage), findsOneWidget);
    });

    appTest('forget the player and go back to finding one', (h) async {
      await h.tester.tap(find.byTooltip('Settings'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      await h.tester.tap(find.text('Forget this player'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 400));
      expect(find.text('Find your player'), findsOneWidget);
      expect(h.store.player, isNull);
    });
  });

  group('the tests', () {
    testWidgets('would catch a layout overflow', (tester) async {
      tester.view.physicalSize = const Size(300, 300);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        const Directionality(
          textDirection: TextDirection.ltr,
          child: Row(
            children: [SizedBox(width: 200, height: 10), SizedBox(width: 200)],
          ),
        ),
      );
      final error = tester.takeException();
      expect(error, isA<FlutterError>());
      expect(error.toString(), contains('overflowed'));
    });
  });
}
