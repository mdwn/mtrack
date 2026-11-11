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

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/ui/app.dart';

import 'fake_link.dart';
import 'fake_stage.dart';
import 'rig.dart';

/// The bundled fonts, loaded so text measures as it does on a phone.
Future<void> loadAppFonts() async {
  const fonts = {
    'Nunito': 'assets/fonts/nunito/Nunito.ttf',
    'Inter': 'assets/fonts/inter/Inter.ttf',
    'JetBrainsMono': 'assets/fonts/jetbrains_mono/JetBrainsMono.ttf',
  };
  for (final entry in fonts.entries) {
    final loader = FontLoader(entry.key)..addFont(rootBundle.load(entry.value));
    await loader.load();
  }
}

/// How a test looks at the app: theme, text size and screen size.
class Look {
  /// A look.
  const Look(this.brightness, this.textScale);

  /// Light or dark.
  final Brightness brightness;

  /// The phone's text size setting.
  final double textScale;

  @override
  String toString() =>
      '${brightness == Brightness.dark ? 'dark' : 'light'} at ${textScale}x';

  /// Every theme at normal and large text.
  static const List<Look> all = [
    Look(Brightness.dark, 1.0),
    Look(Brightness.dark, 1.3),
    Look(Brightness.light, 1.0),
    Look(Brightness.light, 1.3),
  ];
}

/// The app over a fake rig, on a small phone.
class AppHarness {
  AppHarness._(
    this.tester,
    this.rig,
    this.store,
    this.wake,
    this.stage,
    this.views,
    this.session,
  );

  final WidgetTester tester;
  final Rig rig;
  final MemoryStore store;
  final FakeWake wake;

  /// What the Lights tab shows its stage page in.
  final FakeStageHost stage;

  /// The view choices kept across launches.
  final MemoryViewStore views;
  final Session session;

  /// The time the session's clock reads. Advances with [advance].
  Duration now = Duration.zero;

  /// Builds the app. With [connected], the session starts and connects to the
  /// rig's player before the first frame is checked.
  static Future<AppHarness> pump(
    WidgetTester tester, {
    Rig? rig,
    bool known = true,
    Look look = const Look(Brightness.dark, 1.0),
    Size size = const Size(360, 640),
    String? stageView,
    Map<String, String>? views,
  }) async {
    rig ??= Rig();
    tester.view.physicalSize = size * 3;
    tester.view.devicePixelRatio = 3;
    tester.platformDispatcher.textScaleFactorTestValue = look.textScale;
    tester.platformDispatcher.platformBrightnessTestValue = look.brightness;
    final store = MemoryStore(
      known ? const RememberedPlayer(id: 'p1', name: 'stage') : null,
    );
    final wake = FakeWake();
    late final AppHarness harness;
    final session = Session(
      link: rig.link,
      store: store,
      wake: wake,
      monotonic: () => harness.now,
    );
    final stage = FakeStageHost()..view = stageView;
    final kept = MemoryViewStore(views);
    harness = AppHarness._(tester, rig, store, wake, stage, kept, session);
    rig.link.paired.add('p1');
    await tester.pumpWidget(
      MtrackApp(session: session, stage: stage, views: kept),
    );
    unawaited(session.start());
    await harness.settle();
    return harness;
  }

  /// Lets queued work finish.
  Future<void> settle() async {
    for (var i = 0; i < 4; i++) {
      await tester.pump(const Duration(milliseconds: 1));
    }
  }

  /// Moves time on by [time], on the session's clock and the frame clock.
  Future<void> advance(Duration time) async {
    now += time;
    await tester.pump(time);
  }

  /// Taps the tab called [label].
  Future<void> openTab(String label) async {
    await tester.tap(find.text(label).last);
    await settle();
    await tester.pump(const Duration(milliseconds: 50));
  }

  /// Puts the app in the background or the foreground, as the system does:
  /// through the states between, which a text field's listener insists on.
  Future<void> lifecycle(AppLifecycleState state) async {
    _walkTo(state);
    await settle();
  }

  void _walkTo(AppLifecycleState state) {
    const order = [
      AppLifecycleState.resumed,
      AppLifecycleState.inactive,
      AppLifecycleState.hidden,
      AppLifecycleState.paused,
    ];
    final from = order.indexOf(
      tester.binding.lifecycleState ?? AppLifecycleState.resumed,
    );
    final to = order.indexOf(state);
    if (from < 0 || to < 0) {
      tester.binding.handleAppLifecycleStateChanged(state);
      return;
    }
    final step = to > from ? 1 : -1;
    for (var i = from; i != to; i += step) {
      tester.binding.handleAppLifecycleStateChanged(order[i + step]);
    }
  }

  /// Ends the session and the app, so nothing is left running.
  Future<void> close() async {
    _walkTo(AppLifecycleState.resumed);
    session.dispose();
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 1));
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
    tester.platformDispatcher.clearAllTestValues();
  }
}

/// A widget test over an [AppHarness], closed afterwards. [setup] shapes the
/// rig before the app connects to it. [stageView] is the view of the Lights
/// tab remembered from the last launch; [views] the view choices kept.
void appTest(
  String name,
  Future<void> Function(AppHarness h) body, {
  void Function(Rig rig)? setup,
  Look look = const Look(Brightness.dark, 1.0),
  Size size = const Size(360, 640),
  bool known = true,
  String? stageView,
  Map<String, String>? views,
}) {
  testWidgets(name, (tester) async {
    await loadAppFonts();
    final rig = Rig();
    setup?.call(rig);
    rig.publish();
    final harness = await AppHarness.pump(
      tester,
      rig: rig,
      look: look,
      size: size,
      known: known,
      stageView: stageView,
      views: views,
    );
    try {
      await body(harness);
    } finally {
      await harness.close();
    }
  });
}
