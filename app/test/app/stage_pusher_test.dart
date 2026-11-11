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
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/stage_pusher.dart';
import 'package:mtrack_remote/src/app/stage_watch.dart';

import '../support/fake_stage.dart';

/// A pusher over a page that takes each call when the test says so.
class Page {
  Page() {
    pusher = StagePusher(
      run: (script) {
        scripts.add(script);
        inFlight++;
        most = inFlight > most ? inFlight : most;
        final call = Completer<void>();
        _calls.add(call);
        return call.future.whenComplete(() => inFlight--);
      },
      restart: () => restarts++,
      failed: failures.add,
    );
  }

  late final StagePusher pusher;
  final List<String> scripts = [];
  final List<Completer<void>> _calls = [];
  final List<Object> failures = [];
  int restarts = 0;
  int inFlight = 0;

  /// The most calls that were ever out at once.
  int most = 0;

  /// What each call did, in order.
  List<List<String>> get calls => scripts.map(stageCalls).toList();

  /// Everything done to the page, in order.
  List<String> get all => scripts.expand(stageCalls).toList();

  void reset() => pusher.add(const StageReset());
  void layout(int n) =>
      pusher.add(StageBytes(Uint8List.fromList(stageLayout(n))));
  void frame(int n) =>
      pusher.add(StageBytes(Uint8List.fromList(stageFrame(n))));

  /// The call that is out returns.
  Future<void> returns() async {
    _calls.removeAt(0).complete();
    await pumpEventQueue();
  }

  /// The call that is out fails.
  Future<void> throws(Object error) async {
    _calls.removeAt(0).completeError(error);
    await pumpEventQueue();
  }
}

void main() {
  group('before the page is ready', () {
    test('nothing is sent', () async {
      final page = Page();
      page.pusher.pageLoading();
      page
        ..reset()
        ..layout(1)
        ..frame(1);
      await pumpEventQueue();
      expect(page.scripts, isEmpty);
      expect(page.pusher.pageIsReady, isFalse);
      expect(page.pusher.showing, isFalse);
    });

    test('the reset, the layout and the newest frame go once it is', () async {
      final page = Page();
      page.pusher.pageLoading();
      page
        ..reset()
        ..layout(1)
        ..frame(1)
        ..frame(2)
        ..frame(3);
      page.pusher.pageReady();
      expect(page.calls, [
        ['reset', 'L1', 'F3'],
      ]);
      expect(page.restarts, 0);
      await page.returns();
      expect(page.pusher.showing, isTrue);
    });

    test('a theme set meanwhile goes with them, first', () async {
      final page = Page();
      page.pusher.pageLoading();
      page.pusher.setTheme('light');
      page
        ..reset()
        ..layout(1);
      expect(page.scripts, isEmpty);
      page.pusher.pageReady();
      expect(page.all, ['theme:light', 'reset', 'L1']);
    });
  });

  group('once the page is ready', () {
    Page ready() {
      final page = Page();
      page.pusher
        ..pageLoading()
        ..pageReady();
      return page;
    }

    test('an update is sent as base64, to window.mtrackStage', () {
      final page = ready()..layout(7);
      expect(
        page.scripts.single,
        "(function(){var s=window.mtrackStage;s.update('Cgc=');})();",
      );
    });

    test('a reset goes before the layout that follows it', () async {
      final page = ready()
        ..reset()
        ..layout(1);
      await page.returns();
      await page.returns();
      expect(page.all, ['reset', 'L1']);
    });

    test('one call is out at a time', () async {
      final page = ready()..reset();
      for (var n = 1; n <= 5; n++) {
        page
          ..layout(n)
          ..frame(n);
      }
      expect(page.scripts, hasLength(1));
      while (page.inFlight > 0) {
        await page.returns();
      }
      expect(page.most, 1);
    });

    test('while a call is out, the newest frame wins', () async {
      final page = ready()..layout(1);
      page
        ..frame(1)
        ..frame(2)
        ..frame(3);
      expect(page.calls, [
        ['L1'],
      ]);
      await page.returns();
      expect(page.calls, [
        ['L1'],
        ['F3'],
      ]);
      await page.returns();
      expect(page.inFlight, 0);
    });

    test('a layout is never dropped, and layouts keep their order', () async {
      final page = ready()..layout(1);
      page
        ..layout(2)
        ..frame(1)
        ..layout(3)
        ..layout(4)
        ..frame(2);
      await page.returns();
      expect(page.calls, [
        ['L1'],
        ['L2', 'L3', 'L4', 'F2'],
      ]);
    });

    test(
      'a frame that waits is dropped when a layout arrives after it',
      () async {
        final page = ready()..layout(1);
        page
          ..frame(1)
          ..layout(2);
        await page.returns();
        expect(page.calls, [
          ['L1'],
          ['L2'],
        ]);
      },
    );

    test(
      'a frame that waits is dropped when a reset arrives after it',
      () async {
        final page = ready()..layout(1);
        page
          ..frame(1)
          ..reset();
        await page.returns();
        expect(page.calls, [
          ['L1'],
          ['reset'],
        ]);
      },
    );

    test('a reset takes the place of what waited ahead of it', () async {
      final page = ready()..layout(1);
      page
        ..layout(2)
        ..frame(1)
        ..reset()
        ..layout(3)
        ..frame(2);
      await page.returns();
      expect(page.calls, [
        ['L1'],
        ['reset', 'L3', 'F2'],
      ]);
    });

    test('a reset is never dropped', () async {
      final page = ready()..layout(1);
      page
        ..reset()
        ..reset();
      await page.returns();
      expect(page.calls.last, ['reset']);
    });

    test('an update with nothing in it is not sent', () {
      final page = ready();
      page.pusher.add(StageBytes(Uint8List(0)));
      expect(page.scripts, isEmpty);
    });

    test(
      'an update it does not know is kept in order, like a layout',
      () async {
        final page = ready()..layout(1);
        page.pusher.add(StageBytes(Uint8List.fromList([0x1A, 9])));
        page.frame(1);
        await page.returns();
        expect(
          page.scripts.last,
          contains("s.update('Ggk=');s.update('EgE=');"),
        );
      },
    );

    test('showing is true once a layout has been handed over', () async {
      final page = ready();
      var told = 0;
      page.pusher.addListener(() => told++);
      page
        ..reset()
        ..layout(1);
      expect(page.pusher.showing, isFalse);
      await page.returns();
      expect(page.pusher.showing, isFalse, reason: 'only the reset went');
      await page.returns();
      expect(page.pusher.showing, isTrue);
      expect(told, 1);
      page.reset();
      expect(page.pusher.showing, isFalse);
      expect(told, 2);
    });

    test(
      'showing stays false when a reset arrived behind the layout',
      () async {
        final page = ready()..layout(1);
        page.reset();
        await page.returns();
        expect(page.pusher.showing, isFalse);
      },
    );

    test('a theme is sent by itself, and waits its turn', () async {
      final page = ready()..layout(1);
      page.pusher.setTheme('dark');
      expect(page.scripts, hasLength(1));
      await page.returns();
      expect(page.calls.last, ['theme:dark']);
    });

    test('a 3D retry is sent by itself, once, and waits its turn', () async {
      final page = ready()..layout(1);
      page.pusher
        ..retry3d()
        ..retry3d();
      expect(page.scripts, hasLength(1));
      await page.returns();
      expect(page.calls.last, ['retry3d']);
      await page.returns();
      expect(page.scripts, hasLength(2));
    });

    test('a 3D retry is not sent to a page that loaded again', () async {
      final page = ready()..layout(1);
      page.pusher.retry3d();
      page.pusher.pageLoading();
      page.pusher.pageReady();
      await pumpEventQueue();
      expect(page.all, isNot(contains('retry3d')));
    });

    test('a call that fails is reported, and what waited is let go', () async {
      final page = ready()..layout(1);
      page.frame(1);
      await page.throws(StateError('no page'));
      expect(page.failures.single, isStateError);
      expect(page.scripts, hasLength(1));
      expect(page.pusher.isBusy, isFalse);
    });

    test('clearing forgets what waited', () async {
      final page = ready()..layout(1);
      page
        ..layout(2)
        ..frame(1);
      page.pusher.clear();
      await page.returns();
      expect(page.calls, [
        ['L1'],
      ]);
    });
  });

  group('a page that loads again', () {
    test('has the stream started over, and is sent nothing old', () async {
      final page = Page();
      page.pusher
        ..pageLoading()
        ..pageReady();
      page
        ..reset()
        ..layout(1);
      await page.returns();
      await page.returns();
      expect(page.pusher.showing, isTrue);

      page.frame(1);
      page.frame(2);
      page.pusher.pageLoading();
      expect(page.restarts, 1);
      expect(page.pusher.pageIsReady, isFalse);
      expect(page.pusher.showing, isFalse);
      // The call into the page before returns, and means nothing now.
      await page.returns();
      expect(page.scripts, hasLength(3));

      // The stream starts over, and waits for the new page.
      page
        ..reset()
        ..layout(1)
        ..frame(3);
      expect(page.scripts, hasLength(3));
      page.pusher.pageReady();
      expect(page.calls.last, ['reset', 'L1', 'F3']);
      expect(page.restarts, 1);
    });

    test('by itself is noticed by its second ready', () async {
      final page = Page();
      page.pusher
        ..pageLoading()
        ..pageReady();
      page
        ..reset()
        ..layout(1);
      await page.returns();
      page.frame(1);
      page.pusher.pageReady();
      expect(page.restarts, 1);
      expect(page.pusher.showing, isFalse);
      // The call into the page before returns, and means nothing now;
      // nothing more goes until the stream has started over.
      await page.returns();
      expect(page.calls, [
        ['reset'],
        ['L1'],
      ]);
      page
        ..reset()
        ..layout(1);
      await page.returns();
      expect(page.calls.sublist(2), [
        ['reset'],
        ['L1'],
      ]);
    });

    test('before it was given anything keeps what waits', () {
      final page = Page();
      page.pusher.pageLoading();
      page
        ..reset()
        ..layout(1);
      page.pusher.pageLoading();
      expect(page.restarts, 0);
      page.pusher.pageReady();
      expect(page.all, ['reset', 'L1']);
    });

    test('ready but given nothing yet keeps the stream as it is', () {
      final page = Page();
      page.pusher
        ..pageLoading()
        ..pageReady()
        ..pageLoading();
      expect(page.restarts, 0);
    });
  });

  test('a disposed pusher does nothing', () async {
    final page = Page();
    page.pusher
      ..pageLoading()
      ..pageReady();
    page.layout(1);
    page.pusher.dispose();
    await page.returns();
    expect(page.scripts, hasLength(1));
  });
}
