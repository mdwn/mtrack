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
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/lighting_assets.dart';
import 'package:mtrack_remote/src/ui/lights_screen.dart';
import 'package:mtrack_remote/src/ui/settings_sheet.dart';
import 'package:mtrack_remote/src/ui/theme.dart';

import '../support/app_harness.dart';
import '../support/fake_stage.dart';

int calls(AppHarness h, String method) =>
    h.rig.player.received.where((r) => r.method == method).length;

Future<void> openSettings(AppHarness h) async {
  appVersion = () async => '0.1.0 (1)';
  await h.tester.tap(find.byTooltip('Settings'));
  await h.settle();
  await h.tester.pump(const Duration(milliseconds: 400));
}

/// True while the player has a stage stream open for the phone.
bool watching(AppHarness h) => h.rig.player.openStreams.isNotEmpty;

/// The player sends [update] on the stage stream that is open.
Future<void> stream(AppHarness h, List<int> update) async {
  h.rig.player.streamMessage(h.rig.player.openStreams.last, update);
  await h.settle();
}

/// The Lights tab, open, with its page ready and a layout drawn.
Future<void> showLights(AppHarness h) async {
  await h.openTab('Lights');
  h.stage.surface.ready();
  await stream(h, stageLayout(1));
}

void main() {
  group('the tab bar', () {
    appTest('has five tabs, in order, with Lights last', (h) async {
      final centres = <double>[];
      for (final label in ['Now', 'Library', 'Sections', 'Levels', 'Lights']) {
        final bar = find
            .text(label)
            .evaluate()
            .map((e) => h.tester.getCenter(find.byWidget(e.widget)))
            .where((c) => c.dy > 560);
        expect(bar, hasLength(1), reason: label);
        centres.add(bar.single.dx);
      }
      expect([...centres]..sort(), centres);
      expect(centres.toSet(), hasLength(5));
    });

    for (final (size, scale) in [
      (const Size(320, 480), 1.3),
      (const Size(320, 480), 1.0),
      (const Size(360, 640), 1.3),
    ]) {
      appTest(
        'fits every label on one line at ${size.width.toInt()} wide and '
        '${scale}x text',
        (h) async {
          final tab = size.width / 5;
          final sizes = <double>{};
          for (final label in [
            'Now',
            'Library',
            'Sections',
            'Levels',
            'Lights',
          ]) {
            final text = find.text(label).last;
            final box = h.tester.getSize(text);
            expect(box.width, lessThanOrEqualTo(tab), reason: label);
            final paragraph = h.tester.renderObject<RenderParagraph>(text);
            expect(paragraph.didExceedMaxLines, isFalse, reason: label);
            expect(
              paragraph.getMaxIntrinsicWidth(double.infinity),
              lessThanOrEqualTo(tab),
              reason: '$label is not cut off',
            );
            sizes.add(h.tester.widget<Text>(text).style!.fontSize!);
            // The whole tab is the target, and it is tall enough to hit.
            final target = h.tester.getSize(
              find.ancestor(of: text, matching: find.byType(InkWell)).first,
            );
            expect(target.height, greaterThanOrEqualTo(48), reason: label);
            expect(target.width, greaterThanOrEqualTo(48), reason: label);
          }
          expect(sizes, hasLength(1), reason: 'one size for every label');
        },
        size: size,
        look: Look(Brightness.dark, scale),
      );
    }

    appTest('keeps its labels at their own size when there is room', (h) async {
      final text = h.tester.widget<Text>(find.text('Sections').last);
      expect(text.style!.fontSize, 12.5);
    });
  });

  group('the ambient light', () {
    appTest('is at the top of the Lights tab, with what it is doing', (
      h,
    ) async {
      await h.openTab('Lights');
      expect(find.text('Ambient light'), findsOneWidget);
      expect(find.text('playing'), findsOneWidget);
      final auto = find.bySemanticsLabel('Ambient light Auto');
      expect(auto, findsOneWidget);
      expect(
        h.tester.getTopLeft(find.text('Ambient light')).dy,
        lessThan(
          h.tester.getTopLeft(find.byKey(const Key('stage-surface'))).dy,
        ),
      );
    }, setup: (rig) => rig.ambientState = 'playing');

    appTest('is no longer in the settings', (h) async {
      await openSettings(h);
      expect(find.text('Forget this player'), findsOneWidget);
      expect(find.text('Player lock'), findsOneWidget);
      expect(find.text('Ambient light'), findsNothing);
      expect(find.bySemanticsLabel('Ambient light Auto'), findsNothing);
    });

    appTest('says waiting for a pending show', (h) async {
      await h.openTab('Lights');
      expect(find.text('waiting'), findsOneWidget);
    }, setup: (rig) => rig.ambientState = 'pending');

    appTest('is hidden when the player has none', (h) async {
      await h.openTab('Lights');
      expect(find.text('Ambient light'), findsNothing);
      expect(find.byKey(const Key('stage-surface')), findsOneWidget);
    }, setup: (rig) => rig.ambientMode = null);

    appTest('changes the mode when a segment is tapped', (h) async {
      await h.openTab('Lights');
      await h.tester.tap(find.text('On'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      final request = pp.SetAmbientModeRequest.fromBuffer(
        h.rig.player.received
            .lastWhere((r) => r.method == 'SetAmbientMode')
            .payload,
      );
      expect(request.mode, 'on');
      expect(h.rig.ambientMode, 'on');
      expect(find.text('playing'), findsOneWidget);
    });

    appTest('says there is no ambient show when there is none', (h) async {
      await h.openTab('Lights');
      await h.tester.tap(find.text('On'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(find.text('no ambient show'), findsOneWidget);
      await h.tester.tap(find.text('Off'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 50));
      expect(find.text('no ambient show'), findsNothing);
      expect(find.text('off'), findsOneWidget);
    }, setup: (rig) => rig.ambientShow = false);

    appTest('cannot be changed without the player', (h) async {
      await h.openTab('Lights');
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
      await h.tester.tap(find.text('Off'));
      await h.settle();
      expect(calls(h, 'SetAmbientMode'), 0);
      await h.advance(const Duration(seconds: 30));
    });

    appTest('works against a player too old for the stage', (h) async {
      await h.openTab('Lights');
      expect(find.text(lightsTooOldMessage), findsOneWidget);
      await h.tester.tap(find.text('On'));
      await h.settle();
      expect(h.rig.ambientMode, 'on');
    }, setup: (rig) => rig.player.hasStreams = false);
  });

  group('the stage stream', () {
    appTest('is not opened until the tab is', (h) async {
      expect(calls(h, 'WatchStage'), 0);
      expect(h.stage.surfaces, isEmpty);
      expect(h.stage.serves, 0);
      await h.openTab('Levels');
      expect(calls(h, 'WatchStage'), 0);
      expect(h.stage.surfaces, isEmpty);

      await h.openTab('Lights');
      expect(calls(h, 'WatchStage'), 1);
      expect(watching(h), isTrue);
      final request = h.rig.player.received.lastWhere(
        (r) => r.method == 'WatchStage',
      );
      expect(pp.WatchStageRequest.fromBuffer(request.payload).maxRate, 10);
      expect(h.stage.surfaces, hasLength(1));
    });

    appTest('is cancelled on leaving the tab, and opened again on coming '
        'back, with a reset first', (h) async {
      await showLights(h);
      await stream(h, stageFrame(1));
      expect(h.stage.surface.calls, ['reset', 'L1', 'F1']);

      await h.openTab('Now');
      expect(watching(h), isFalse);
      expect(h.rig.player.cancels, hasLength(1));

      await h.openTab('Lights');
      expect(watching(h), isTrue);
      expect(calls(h, 'WatchStage'), 2);
      expect(h.stage.surfaces, hasLength(1), reason: 'the page is kept');
      expect(h.stage.surface.loads, hasLength(1));
      await stream(h, stageLayout(2));
      expect(h.stage.surface.calls, ['reset', 'L1', 'F1', 'reset', 'L2']);
    });

    appTest('is cancelled while the app is in the background', (h) async {
      await showLights(h);
      await h.lifecycle(AppLifecycleState.inactive);
      expect(watching(h), isTrue, reason: 'still on screen');
      await h.lifecycle(AppLifecycleState.hidden);
      expect(watching(h), isFalse);
      await h.lifecycle(AppLifecycleState.paused);
      expect(watching(h), isFalse);
      expect(calls(h, 'WatchStage'), 1);

      await h.lifecycle(AppLifecycleState.hidden);
      await h.lifecycle(AppLifecycleState.inactive);
      await h.lifecycle(AppLifecycleState.resumed);
      expect(watching(h), isTrue);
      expect(calls(h, 'WatchStage'), 2);
    });

    appTest('stays closed in the background whichever tab is showing', (
      h,
    ) async {
      await h.lifecycle(AppLifecycleState.paused);
      await h.openTab('Lights');
      expect(calls(h, 'WatchStage'), 0);
      await h.lifecycle(AppLifecycleState.resumed);
      expect(calls(h, 'WatchStage'), 1);
      await h.openTab('Now');
      await h.lifecycle(AppLifecycleState.paused);
      await h.lifecycle(AppLifecycleState.resumed);
      expect(calls(h, 'WatchStage'), 1);
      expect(watching(h), isFalse);
    });

    appTest('is cancelled while a playlist is being edited', (h) async {
      await showLights(h);
      h.session.startNewPlaylist('New');
      await h.settle();
      expect(watching(h), isFalse);
    });

    appTest('is opened once when the link comes back', (h) async {
      await showLights(h);
      h.rig.player.disconnect();
      await h.settle();
      await h.advance(const Duration(seconds: 5));
      await h.settle();
      expect(h.session.isReady, isTrue);
      expect(calls(h, 'WatchStage'), 2);
      expect(h.rig.player.openStreams, hasLength(1));
      await stream(h, stageLayout(2));
      expect(h.stage.surface.calls, ['reset', 'L1', 'reset', 'L2']);
    });
  });

  group('the page', () {
    appTest('is loaded in the theme, on the page\'s own colour', (h) async {
      await h.openTab('Lights');
      final surface = h.stage.surface;
      expect(surface.loads.single.toString(), '${h.stage.page}?theme=dark');
      expect(surface.backgrounds.single, AppColors.dark.surface);
    });

    appTest('is loaded light in the light theme', (h) async {
      await h.openTab('Lights');
      final surface = h.stage.surface;
      expect(surface.loads.single.queryParameters, {'theme': 'light'});
      expect(surface.backgrounds.single, AppColors.light.surface);
    }, look: const Look(Brightness.light, 1.0));

    appTest('is told when the theme changes', (h) async {
      await showLights(h);
      h.tester.platformDispatcher.platformBrightnessTestValue =
          Brightness.light;
      await h.settle();
      // The theme fades from one to the other.
      await h.tester.pump(const Duration(milliseconds: 400));
      await h.settle();
      final surface = h.stage.surface;
      expect(surface.calls, ['reset', 'L1', 'theme:light']);
      expect(surface.backgrounds.last, AppColors.light.surface);
      expect(surface.loads, hasLength(1), reason: 'not loaded again');
    });

    appTest('is sent nothing until it says it is ready', (h) async {
      await h.openTab('Lights');
      await stream(h, stageLayout(1));
      await stream(h, stageFrame(1));
      await stream(h, stageFrame(2));
      final surface = h.stage.surface;
      expect(surface.scripts, isEmpty);
      expect(find.text(lightsWaitingMessage), findsOneWidget);

      surface.ready();
      await h.settle();
      expect(surface.calls, ['reset', 'L1', 'F2']);
      expect(surface.scripts, hasLength(1));
      expect(find.text(lightsWaitingMessage), findsNothing);
    });

    appTest('waits, saying so, until it has a layout', (h) async {
      await h.openTab('Lights');
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      h.stage.surface.ready();
      await h.settle();
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      await stream(h, stageLayout(1));
      expect(find.text(lightsWaitingMessage), findsNothing);
      expect(find.byType(CircularProgressIndicator), findsNothing);
    });

    appTest('gets one call at a time, and the newest frame', (h) async {
      await showLights(h);
      final surface = h.stage.surface;
      surface.held = Completer<void>();
      await stream(h, stageFrame(1));
      await stream(h, stageFrame(2));
      await stream(h, stageFrame(3));
      expect(surface.calls, ['reset', 'L1', 'F1']);
      surface.held!.complete();
      surface.held = null;
      await h.settle();
      expect(surface.calls, ['reset', 'L1', 'F1', 'F3']);
    });

    appTest('takes touches, and is named to a screen reader', (h) async {
      await h.openTab('Lights');
      final surface = h.stage.surface;
      final view = find.byKey(const Key('stage-surface'));
      // Under "Waiting for the lights…", the cover takes them.
      await h.tester.tapAt(h.tester.getCenter(view));
      expect(surface.touches, 0);

      surface.ready();
      await stream(h, stageLayout(1));
      // The page's own: its Plot | 3D switch, and 3D turning under a finger.
      await h.tester.tapAt(h.tester.getCenter(view));
      await h.tester.dragFrom(h.tester.getCenter(view), const Offset(80, -40));
      expect(surface.touches, 2);
      expect(find.bySemanticsLabel('Stage lights'), findsOneWidget);
      // Edge to edge, down to the tab bar.
      final box = h.tester.getRect(view);
      expect(box.left, 0);
      expect(box.right, 360);
      final bar = h.tester.getRect(
        find.ancestor(
          of: find.text('Lights').last,
          matching: find.byType(InkWell),
        ),
      );
      expect(box.bottom, closeTo(bar.top, 1.5));
    });

    appTest('that loads again by itself has the stream started over', (
      h,
    ) async {
      await showLights(h);
      expect(calls(h, 'WatchStage'), 1);
      h.stage.surface.ready();
      await h.settle();
      expect(calls(h, 'WatchStage'), 2);
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      await stream(h, stageLayout(1));
      expect(h.stage.surface.calls, ['reset', 'L1', 'reset', 'L1']);
      expect(find.text(lightsWaitingMessage), findsNothing);
    });

    appTest('is loaded again, and the stream started over, when it has '
        'moved while the app was away', (h) async {
      await showLights(h);
      await h.lifecycle(AppLifecycleState.paused);
      await h.lifecycle(AppLifecycleState.resumed);
      final surface = h.stage.surface;
      expect(surface.loads, hasLength(1), reason: 'still where it was');

      await h.lifecycle(AppLifecycleState.paused);
      h.stage.page = Uri.parse('http://127.0.0.1:4001/other/stage.html');
      await h.lifecycle(AppLifecycleState.resumed);
      expect(surface.loads, hasLength(2));
      expect(surface.loads.last.toString(), '${h.stage.page}?theme=dark');
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      final before = surface.scripts.length;
      await stream(h, stageLayout(2));
      expect(surface.scripts, hasLength(before), reason: 'not ready yet');
      surface.ready();
      await h.settle();
      expect(surface.calls.sublist(3), ['reset', 'L2']);
      expect(find.text(lightsWaitingMessage), findsNothing);
    });

    appTest('is checked on coming back to the tab too', (h) async {
      await showLights(h);
      await h.openTab('Now');
      h.stage.page = Uri.parse('http://127.0.0.1:4001/other/stage.html');
      await h.openTab('Lights');
      expect(h.stage.surface.loads, hasLength(2));
    });

    appTest('is let go, and no longer served, with the player', (h) async {
      await showLights(h);
      final surface = h.stage.surface;
      await h.session.forget();
      await h.settle();
      expect(find.text('Find your player'), findsOneWidget);
      expect(surface.disposed, isTrue);
      expect(h.stage.stops, 1);
      expect(watching(h), isFalse);
    });
  });

  group('a page that fails', () {
    Future<void> expectFailed(AppHarness h) async {
      expect(find.text(lightsFailedMessage), findsOneWidget);
      expect(find.text('Try again'), findsOneWidget);
      expect(find.text(lightsWaitingMessage), findsNothing);
      expect(watching(h), isFalse, reason: 'nothing to feed');
    }

    Future<void> tryAgain(AppHarness h) async {
      await h.tester.tap(find.text('Try again'));
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 200));
    }

    appTest('by reporting an error before it is ready says so, and can be '
        'tried again', (h) async {
      await h.openTab('Lights');
      final surface = h.stage.surface;
      surface.pageError("could not load the player's rules module: 404");
      await h.settle();
      await expectFailed(h);

      await tryAgain(h);
      expect(find.text(lightsFailedMessage), findsNothing);
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      expect(surface.loads, hasLength(2));
      expect(h.stage.surfaces, hasLength(1));
      expect(watching(h), isTrue);
      surface.ready();
      await stream(h, stageLayout(1));
      expect(surface.calls, ['reset', 'L1']);
      expect(find.text(lightsWaitingMessage), findsNothing);
    });

    appTest('by not loading says so', (h) async {
      await h.openTab('Lights');
      h.stage.surface.loadError('net::ERR_CONNECTION_REFUSED');
      await h.settle();
      await expectFailed(h);
    });

    appTest('because it could not be served says so, and can be tried '
        'again', (h) async {
      h.stage.serveError = StateError('no socket');
      await h.openTab('Lights');
      await expectFailed(h);
      expect(h.stage.surface.loads, isEmpty);

      h.stage.serveError = null;
      await tryAgain(h);
      expect(h.stage.surface.loads, hasLength(1));
      expect(find.text(lightsWaitingMessage), findsOneWidget);
    });

    appTest('by never saying it is ready says so in the end', (h) async {
      await h.openTab('Lights');
      await h.advance(stagePageTimeout - const Duration(seconds: 1));
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      await h.advance(const Duration(seconds: 2));
      await h.settle();
      await expectFailed(h);
    });

    appTest('is not timed out once it is ready', (h) async {
      await showLights(h);
      await h.advance(stagePageTimeout * 2);
      expect(find.text(lightsFailedMessage), findsNothing);
    });

    appTest('because a call into it failed says so', (h) async {
      await showLights(h);
      h.stage.surface.held = Completer<void>();
      await stream(h, stageFrame(1));
      h.stage.surface.held!.completeError(StateError('no page'));
      await h.settle();
      await expectFailed(h);
    });

    appTest('with one error while it runs carries on, as the page does', (
      h,
    ) async {
      await showLights(h);
      h.stage.surface.pageError('stage update: bad bytes');
      await h.settle();
      expect(find.text(lightsFailedMessage), findsNothing);
      expect(watching(h), isTrue);
      // Errors far apart are each one by itself.
      for (var i = 0; i < 4; i++) {
        await h.advance(stagePageErrorWindow + const Duration(seconds: 1));
        h.stage.surface.pageError('stage update: bad bytes');
        await h.settle();
      }
      expect(find.text(lightsFailedMessage), findsNothing);
    });

    appTest('with errors one after another while it runs says so', (h) async {
      await showLights(h);
      for (var i = 0; i < stagePageErrorLimit; i++) {
        h.stage.surface.pageError('uncaught error: broken');
        await h.settle();
      }
      await expectFailed(h);
      await tryAgain(h);
      expect(h.stage.surface.loads, hasLength(2));
    });

    appTest('says nothing it does not understand is an error', (h) async {
      await showLights(h);
      h.stage.surface
        ..say('not json')
        ..say('{"type":"later"}')
        ..say('[1]');
      await h.settle();
      expect(find.text(lightsFailedMessage), findsNothing);
    });
  });

  group('an older player', () {
    appTest('with no streams is told so in place of the stage, and no page '
        'is loaded', (h) async {
      await h.openTab('Lights');
      expect(find.text(lightsTooOldMessage), findsOneWidget);
      expect(find.text('Ambient light'), findsOneWidget);
      expect(find.text(lightsWaitingMessage), findsNothing);
      expect(h.stage.surfaces, isEmpty);
      expect(h.stage.serves, 0);
      expect(calls(h, 'WatchStage'), 0);
    }, setup: (rig) => rig.player.hasStreams = false);

    appTest('that refuses the stream is told so once it has', (h) async {
      h.rig.player.onStream = (request) => h.rig.player.endStream(
        request.id,
        code: 7,
        text: 'method WatchStage is not allowed',
      );
      await h.openTab('Lights');
      await h.settle();
      expect(find.text(lightsTooOldMessage), findsOneWidget);
      expect(find.byKey(const Key('stage-surface')), findsNothing);
      expect(h.stage.surface.disposed, isTrue);
    });

    appTest('is named in the settings as lacking the light view', (h) async {
      await openSettings(h);
      expect(
        find.text(
          'This player runs an older mtrack, so the light view is not shown. '
          'Update mtrack on the player to use it.',
        ),
        findsOneWidget,
      );
    }, setup: (rig) => rig.player.hasStreams = false);
  });

  group('in 3D', () {
    const rig = 'abc/rig-def-v2.json';

    /// The page says it shows [view].
    Future<void> view(AppHarness h, String view) async {
      h.stage.surface.say(jsonEncode({'type': 'view', 'view': view}));
      await h.settle();
    }

    /// The page asks for the asset at [path], as its 3D view does.
    Uint8List? Function() ask(AppHarness h, String path) {
      Uint8List? got;
      Object? error;
      var done = false;
      h.session.lightingAssets
          .get(path)
          .then(
            (bytes) {
              got = bytes;
              done = true;
            },
            onError: (Object e) {
              error = e;
              done = true;
            },
          );
      return () {
        expect(done, isTrue, reason: 'the asset came');
        if (error != null) {
          throw error!;
        }
        return got;
      };
    }

    int assetStream(AppHarness h) => h.rig.player.openStreams.last;

    appTest('starts in the view the phone showed last', (h) async {
      await h.openTab('Lights');
      expect(h.stage.surface.loads.single.queryParameters, {
        'theme': 'dark',
        'view': '3d',
      });
    }, stageView: '3d');

    appTest('starts in the plot when nothing is remembered', (h) async {
      await h.openTab('Lights');
      expect(h.stage.surface.loads.single.queryParameters, {'theme': 'dark'});
    });

    appTest('remembers the view the page shows, and keeps it when the page '
        'loads again', (h) async {
      await showLights(h);
      await view(h, 'plot');
      await view(h, '3d');
      expect(h.stage.remembered, ['plot', '3d']);
      await h.lifecycle(AppLifecycleState.paused);
      h.stage.page = Uri.parse('http://127.0.0.1:4001/other/stage.html');
      await h.lifecycle(AppLifecycleState.resumed);
      expect(h.stage.surface.loads.last.queryParameters['view'], '3d');
    });

    appTest('a view the page cannot be in is not remembered', (h) async {
      await showLights(h);
      await view(h, 'sideways');
      expect(h.stage.remembered, isEmpty);
    });

    appTest('shows the models coming, over the 3D view', (h) async {
      await showLights(h);
      await view(h, '3d');
      final one = ask(h, rig);
      ask(h, 'abc/models/Base.glb');
      await h.settle();
      expect(find.byKey(const Key('models-loading')), findsOneWidget);
      expect(find.text('$lightsModelsLoadingMessage 1 of 2'), findsOneWidget);
      h.rig.player.sendAsset(
        assetStream(h),
        List.filled(100, 1),
        chunk: 50,
        end: false,
      );
      await h.settle();
      final bar = h.tester.widget<LinearProgressIndicator>(
        find.byType(LinearProgressIndicator),
      );
      expect(bar.value, 0.5);
      h.rig.player.endStream(assetStream(h));
      await h.settle();
      expect(one(), hasLength(100));
      expect(find.text('$lightsModelsLoadingMessage 2 of 2'), findsOneWidget);
      h.rig.player.sendAsset(assetStream(h), List.filled(10, 1));
      await h.settle();
      expect(find.byKey(const Key('models-loading')), findsNothing);
      // The stage page's own touches are not covered but where the note was.
      expect(find.text(lightsWaitingMessage), findsNothing);
    });

    appTest('says nothing of models over the plot', (h) async {
      await showLights(h);
      await view(h, 'plot');
      ask(h, rig);
      await h.settle();
      expect(find.byKey(const Key('models-loading')), findsNothing);
    });

    appTest('a model the player does not have is said, and can be asked for '
        'again', (h) async {
      h.rig.player.assets = {};
      await showLights(h);
      await view(h, '3d');
      final got = ask(h, rig);
      await h.settle();
      expect(got(), isNull);
      expect(find.text(lightsModelsMissingMessage), findsOneWidget);
      // The plot still runs under it.
      await stream(h, stageFrame(1));
      expect(h.stage.surface.calls.last, 'F1');

      await h.tester.tap(find.text('Try again'));
      await h.settle();
      expect(h.stage.surface.calls, contains('retry3d'));
      expect(find.text(lightsModelsMissingMessage), findsNothing);
    });

    appTest('a model the link lost is said, and a new stream starts afresh', (
      h,
    ) async {
      await showLights(h);
      await view(h, '3d');
      final got = ask(h, rig);
      await h.settle();
      h.rig.player.sendAsset(
        assetStream(h),
        List.filled(100, 1),
        total: 1000,
        end: false,
      );
      h.rig.player.disconnect();
      await h.settle();
      expect(got, throwsA(isA<AssetUnavailable>()));
      await h.advance(const Duration(seconds: 5));
      await h.settle();
      // The page is fed the new stream from its start, and builds 3D anew.
      await stream(h, stageLayout(2));
      expect(find.text(lightsModelsFailedMessage), findsNothing);
      ask(h, rig);
      await h.settle();
      h.rig.player.sendAsset(assetStream(h), List.filled(10, 1));
      await h.settle();
      expect(find.text(lightsModelsFailedMessage), findsNothing);
    });

    appTest('a failure while the link is up offers Try again', (h) async {
      await showLights(h);
      await view(h, '3d');
      final got = ask(h, rig);
      await h.settle();
      h.rig.player.endStream(assetStream(h), code: 13, text: 'disk error');
      await h.settle();
      expect(got, throwsA(isA<AssetUnavailable>()));
      expect(find.text(lightsModelsFailedMessage), findsOneWidget);
      expect(find.text('Try again'), findsOneWidget);
    });

    appTest('an older player is told so, and named in the settings', (h) async {
      h.rig.player.streamMethods.remove('GetLightingAsset');
      await showLights(h);
      await view(h, '3d');
      ask(h, rig);
      await h.settle();
      expect(find.text(lightsModelsTooOldMessage), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
      await openSettings(h);
      expect(
        find.text(
          'This player runs an older mtrack, so the 3D light view is not '
          'shown. Update mtrack on the player to use it.',
        ),
        findsOneWidget,
      );
    });
  });

  group('without the link', () {
    Future<void> lose(AppHarness h) async {
      h.rig.link.connectFailures = 100;
      h.rig.player.disconnect();
      await h.settle();
      await h.tester.pump(const Duration(milliseconds: 100));
    }

    appTest('the lights that were showing are dimmed under the '
        'reconnecting banner, and the stream is let go', (h) async {
      await showLights(h);
      expect(find.byKey(const Key('stage-dimmed')), findsNothing);
      await lose(h);
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(find.byKey(const Key('stage-dimmed')), findsOneWidget);
      expect(find.text(lightsWaitingMessage), findsNothing);
      expect(find.text(lightsFailedMessage), findsNothing);
      final scripts = h.stage.surface.scripts.length;
      await h.advance(const Duration(seconds: 30));
      expect(h.stage.surface.scripts, hasLength(scripts));
    });

    appTest('a stage that had nothing yet shows nothing', (h) async {
      await h.openTab('Lights');
      await lose(h);
      expect(find.text('Reconnecting…'), findsOneWidget);
      expect(find.byKey(const Key('stage-blank')), findsOneWidget);
      expect(find.text(lightsWaitingMessage), findsNothing);
      await h.advance(const Duration(seconds: 30));
    });

    appTest('coming back waits for the new stream, then shows it', (h) async {
      await showLights(h);
      h.rig.player.disconnect();
      await h.settle();
      await h.advance(const Duration(seconds: 5));
      await h.settle();
      expect(find.text('Reconnecting…'), findsNothing);
      expect(find.text(lightsWaitingMessage), findsOneWidget);
      await stream(h, stageLayout(1));
      expect(find.text(lightsWaitingMessage), findsNothing);
      expect(find.byKey(const Key('stage-dimmed')), findsNothing);
    });
  });
}
