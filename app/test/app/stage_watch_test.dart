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

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/stage_watch.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import '../support/fake_player.dart';
import '../support/session_harness.dart';

/// A layout the page would be sent, serialized. Its venue name is [name], so
/// a test can tell one from another.
Uint8List layout(String name) => pp.StageUpdate(
  layout: pp.StageLayout(venue: pp.StageVenue(name: name)),
).writeToBuffer();

/// A frame, serialized.
Uint8List frame() => pp.StageUpdate(frame: pp.StageFrame()).writeToBuffer();

/// What a listener saw: `reset`, `layout:<venue>` or `frame`.
String describe(StageEvent event) => switch (event) {
  StageReset() => 'reset',
  StageBytes(:final bytes) => () {
    final update = pp.StageUpdate.fromBuffer(bytes);
    return update.hasLayout() ? 'layout:${update.layout.venue.name}' : 'frame';
  }(),
};

void main() {
  /// Runs [body] with a harness, and checks nothing is left running.
  void scenario(String name, void Function(Harness h, FakeAsync async) body) {
    test(name, () {
      fakeAsync((async) {
        final h = Harness(async);
        body(h, async);
        h.session.dispose();
        h.settle();
        expect(
          async.pendingTimers,
          isEmpty,
          reason: 'timers left running after dispose',
        );
      });
    });
  }

  /// Listens to the stage and records what it says.
  StreamSubscription<StageEvent> watch(
    Harness h,
    List<String> into, {
    int maxRate = 10,
  }) => h.session
      .watchStage(maxRate: maxRate)
      .listen(
        (event) => into.add(describe(event)),
        onError: (Object e) => into.add('error'),
        onDone: () => into.add('done'),
      );

  List<ble.Request> opens(Harness h) =>
      h.player.received.where((r) => r.method == 'WatchStage').toList();

  int connects(Harness h) =>
      h.link.calls.where((c) => c.startsWith('connect')).length;

  group('watching the stage', () {
    scenario('opens the stream on listening, and passes its messages on', (
      h,
      _,
    ) {
      h.connect();
      expect(opens(h), isEmpty, reason: 'nothing is open until it is watched');
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      final request = opens(h).single;
      expect(pp.WatchStageRequest.fromBuffer(request.payload).maxRate, 10);
      expect(seen, ['reset']);

      h.player
        ..streamMessage(request.id, layout('club'))
        ..streamMessage(request.id, frame())
        ..streamMessage(request.id, frame());
      h.settle();
      expect(seen, ['reset', 'layout:club', 'frame', 'frame']);
      expect(h.session.stageMissingFromPlayer, isFalse);
      expect(h.session.missingFromPlayer, isEmpty);
      unawaited(sub.cancel());
    });

    scenario('hands the bytes over as the player sent them', (h, _) {
      h.connect();
      Uint8List? got;
      final sub = h.session.watchStage().listen((event) {
        if (event is StageBytes) {
          got = event.bytes;
        }
      });
      h.settle();
      final sent = layout('club');
      h.player.streamMessage(opens(h).single.id, sent);
      h.settle();
      expect(got, sent);
      unawaited(sub.cancel());
    });

    scenario('cancelling sends the cancel', (h, _) {
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      final id = opens(h).single.id;
      unawaited(sub.cancel());
      h.settle();
      expect(h.player.cancels, [id]);
      expect(h.player.openStreams, isEmpty);
      // A straggler changes nothing.
      h.player.streamMessage(id, frame());
      h.settle();
      expect(seen, ['reset']);
    });

    scenario('can be listened to and cancelled as a tab comes and goes', (
      h,
      _,
    ) {
      h.connect();
      for (var i = 0; i < 4; i++) {
        final seen = <String>[];
        final sub = watch(h, seen);
        h.settle();
        expect(opens(h).length, i + 1);
        expect(h.player.openStreams, [opens(h).last.id]);
        h.player.streamMessage(opens(h).last.id, layout('show $i'));
        h.settle();
        expect(seen, ['reset', 'layout:show $i']);
        unawaited(sub.cancel());
        h.settle();
        expect(h.player.openStreams, isEmpty);
      }
      expect(h.player.cancels, opens(h).map((r) => r.id));
    });

    scenario('waits for the player to let go of the last stream before '
        'opening the next', (h, _) {
      // A layout can be seconds in the sending, and the player finishes it
      // before it answers a cancel. Shown, hidden and shown again inside
      // that time, the tab must not run into the two streams a phone may
      // have.
      h.connect();
      h.player.holdCancels = true;
      final first = watch(h, []);
      h.settle();
      final id = opens(h).single.id;
      unawaited(first.cancel());
      h.settle();
      for (var i = 0; i < 3; i++) {
        final sub = watch(h, []);
        h.settle();
        unawaited(sub.cancel());
        h.settle();
      }
      final seen = <String>[];
      final last = watch(h, seen);
      h.settle();
      expect(opens(h).length, 1, reason: 'the first has not ended yet');
      expect(seen, isEmpty);

      h.player.endStream(id, code: 1);
      h.settle();
      expect(opens(h).length, 2);
      expect(h.player.openStreams, [opens(h).last.id]);
      expect(seen, ['reset']);
      h.player.holdCancels = false;
      unawaited(last.cancel());
    });

    scenario('nothing is open while not connected, and listening early '
        'opens it once connected', (h, _) {
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      expect(seen, isEmpty);
      h.connect();
      expect(opens(h).length, 1);
      expect(seen, ['reset']);
      unawaited(sub.cancel());
    });
  });

  group('the link', () {
    scenario('a lost link ends the stream quietly, and the watch opens it '
        'again after the reconnect, reset first', (h, _) {
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      final first = opens(h).single.id;
      h.player.streamMessage(first, layout('before'));
      h.settle();

      h.player.disconnect();
      h.settle();
      expect(h.session.phase, ConnectionPhase.lost);
      expect(seen, ['reset', 'layout:before'], reason: 'no error, no end');

      h.elapse(const Duration(seconds: 1));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(opens(h).length, 2);
      final second = opens(h).last.id;
      expect(second, isNot(first), reason: 'a new client, a new id');
      expect(seen, ['reset', 'layout:before', 'reset']);
      // The old stream's id means nothing now.
      h.player
        ..streamMessage(first, layout('stale'))
        ..streamMessage(second, layout('after'))
        ..streamMessage(second, frame());
      h.settle();
      expect(seen, [
        'reset',
        'layout:before',
        'reset',
        'layout:after',
        'frame',
      ]);
      // The link took the old stream with it: there was nothing to cancel.
      expect(h.player.cancels, isEmpty);
      unawaited(sub.cancel());
      h.settle();
      expect(h.player.cancels, [second]);
    });

    scenario('a watch cancelled while the link is down is not opened again', (
      h,
      _,
    ) {
      h.connect();
      final sub = watch(h, []);
      h.settle();
      h.player.disconnect();
      h.settle();
      unawaited(sub.cancel());
      h.elapse(const Duration(seconds: 1));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(opens(h).length, 1);
    });

    scenario('a link that died without saying so is found by asking, while '
        'the stage is watched', (h, _) {
      // The player restarted and the radio kept the link. Nothing is
      // playing, so no status comes, and a still stage sends nothing either:
      // silence is all there is to go on.
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      h.player.streamMessage(opens(h).single.id, layout('before'));
      h.settle();
      final before = connects(h);

      h.player.handlers['ListPlaylists'] = (_) => FakeReply.drop();
      h.player.openStreams.clear();
      // One interval with something heard, one with nothing, then the
      // question and the time it is given.
      h.elapse(stageProbeInterval * 2);
      expect(h.count('ListPlaylists'), 1);
      expect(h.session.phase, ConnectionPhase.ready);
      h.elapse(PlayerClient.defaultTimeout);
      expect(h.session.phase, ConnectionPhase.lost);

      h.player.handlers.remove('ListPlaylists');
      h.elapse(const Duration(seconds: 1));
      expect(connects(h) - before, 1);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(opens(h).length, 2);
      expect(seen, ['reset', 'layout:before', 'reset']);
      unawaited(sub.cancel());
    });

    scenario('a quiet player that answers is left alone', (h, _) {
      h.connect();
      final sub = watch(h, []);
      h.settle();
      h.elapse(const Duration(seconds: 30));
      expect(h.count('ListPlaylists'), greaterThan(0));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(opens(h).length, 1);
      unawaited(sub.cancel());
    });

    scenario('nothing is asked while the player is heard from', (h, _) {
      h.connect();
      final sub = watch(h, []);
      h.settle();
      final id = opens(h).single.id;
      for (var i = 0; i < 30; i++) {
        h.elapse(const Duration(seconds: 1));
        if (i.isEven) {
          h.rig.publish();
        } else {
          h.player.streamMessage(id, frame());
        }
      }
      expect(h.count('ListPlaylists'), 0);
      unawaited(sub.cancel());
    });

    scenario('nothing is asked when the stage is not watched', (h, _) {
      h.connect();
      final sub = watch(h, []);
      h.settle();
      unawaited(sub.cancel());
      h.elapse(const Duration(minutes: 1));
      expect(h.count('ListPlaylists'), 0);
    });

    scenario('a stream the player ends is asked for again after a wait', (
      h,
      _,
    ) {
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      h.player.endStream(opens(h).single.id, code: 13, text: 'sampler gone');
      h.settle();
      expect(seen, ['reset'], reason: 'the listener is not failed');
      h.elapse(stageRetryDelay - const Duration(milliseconds: 1));
      expect(opens(h).length, 1);
      h.elapse(const Duration(milliseconds: 1));
      expect(opens(h).length, 2);
      expect(seen, ['reset', 'reset']);

      // And when it ends it with OK, which it should not.
      h.player.endStream(opens(h).last.id);
      h.elapse(stageRetryDelay);
      expect(opens(h).length, 3);
      unawaited(sub.cancel());
    });

    scenario('forgetting the player closes the stream', (h, _) {
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      unawaited(h.session.forget());
      h.settle();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.player.currentState, TransportState.disconnected);
      h.elapse(const Duration(minutes: 1));
      expect(opens(h).length, 1);
      expect(seen, ['reset']);
      unawaited(sub.cancel());
    });

    scenario('disposing the session ends the listener', (h, _) {
      h.connect();
      final seen = <String>[];
      watch(h, seen);
      h.settle();
      h.session.dispose();
      h.settle();
      expect(seen, ['reset', 'done']);
    });
  });

  group('several listeners', () {
    scenario('share one stream, started over when one joins so that it '
        'gets the layout', (h, _) {
      h.connect();
      final a = <String>[];
      final b = <String>[];
      final first = watch(h, a, maxRate: 5);
      h.settle();
      final one = opens(h).single.id;
      h.player.streamMessage(one, layout('club'));
      h.settle();

      final second = watch(h, b, maxRate: 20);
      h.settle();
      expect(h.player.cancels, [one]);
      expect(opens(h).length, 2);
      final two = opens(h).last.id;
      expect(h.player.openStreams, [two], reason: 'one stream, not two');
      // The stream runs at the most any listener asked for.
      expect(
        pp.WatchStageRequest.fromBuffer(opens(h).last.payload).maxRate,
        20,
      );
      h.player
        ..streamMessage(two, layout('club'))
        ..streamMessage(two, frame());
      h.settle();
      expect(a, ['reset', 'layout:club', 'reset', 'layout:club', 'frame']);
      expect(b, ['reset', 'layout:club', 'frame']);

      // One leaves: the other carries on, on the same stream.
      unawaited(first.cancel());
      h.settle();
      expect(h.player.openStreams, [two]);
      h.player.streamMessage(two, frame());
      h.settle();
      expect(b.last, 'frame');
      expect(b.length, 4);
      expect(a.length, 5);

      // The last leaves: the stream is cancelled.
      unawaited(second.cancel());
      h.settle();
      expect(h.player.openStreams, isEmpty);
    });

    scenario('a listener asking for the player\'s own rate gets it for all', (
      h,
      _,
    ) {
      h.connect();
      final first = watch(h, [], maxRate: 0);
      final second = watch(h, []);
      h.settle();
      expect(pp.WatchStageRequest.fromBuffer(opens(h).last.payload).maxRate, 0);
      unawaited(first.cancel());
      unawaited(second.cancel());
    });
  });

  group('a listener that stalls', () {
    scenario('is passed over, holding nothing for it, and starts over when '
        'it resumes', (h, _) {
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      final first = opens(h).single.id;
      h.player.streamMessage(first, layout('one'));
      h.settle();

      sub.pause();
      // A layout goes by while it is not looking, and a great many frames.
      h.player.streamMessage(first, layout('two'));
      for (var i = 0; i < 5000; i++) {
        h.player.streamMessage(first, frame());
      }
      h.settle();
      expect(seen, ['reset', 'layout:one']);

      sub.resume();
      h.settle();
      // None of what went by arrives: the stream is started over instead,
      // so the layout it missed is sent again.
      expect(seen, ['reset', 'layout:one', 'reset']);
      expect(h.player.cancels, [first]);
      final second = opens(h).last.id;
      expect(second, isNot(first));
      h.player.streamMessage(second, layout('two'));
      h.settle();
      expect(seen, ['reset', 'layout:one', 'reset', 'layout:two']);
      unawaited(sub.cancel());
    });

    scenario('does not hold up one that is keeping up', (h, _) {
      h.connect();
      final slow = <String>[];
      final quick = <String>[];
      final a = watch(h, slow);
      final b = watch(h, quick);
      h.settle();
      final id = opens(h).last.id;
      a.pause();
      h.player
        ..streamMessage(id, layout('club'))
        ..streamMessage(id, frame());
      h.settle();
      expect(slow, ['reset']);
      expect(quick, ['reset', 'layout:club', 'frame']);
      unawaited(a.cancel());
      unawaited(b.cancel());
    });
  });

  group('an older player', () {
    scenario('with no stream characteristic connects and is called as '
        'ever, and is asked for no stream', (h, _) {
      h.player.hasStreams = false;
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.stageMissingFromPlayer, isTrue);
      expect(h.session.missingFromPlayer, ['the light view']);
      expect(h.run(h.session.next), isNull);
      expect(h.count('Next'), 1);

      final writes = h.player.writes.length;
      final seen = <String>[];
      final sub = watch(h, seen);
      h.elapse(const Duration(minutes: 1));
      expect(seen, isEmpty, reason: 'silent: no reset, no error, no end');
      expect(opens(h), isEmpty);
      expect(h.player.writes.length, writes, reason: 'nothing was sent');
      unawaited(sub.cancel());
      h.settle();
      expect(h.player.writes.length, writes, reason: 'and nothing cancelled');
    });

    scenario('that has the characteristic but refuses the method is found '
        'out by asking, once', (h, _) {
      h.connect();
      h.player.onStream = (request) {
        h.player.openStreams.remove(request.id);
        h.player.sendResponse(
          ble.Response(
            id: request.id,
            code: 7,
            message: 'method WatchStage is not allowed',
          ),
        );
      };
      expect(h.session.stageMissingFromPlayer, isFalse);
      var notified = 0;
      h.session.addListener(() => notified++);
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      expect(h.session.stageMissingFromPlayer, isTrue);
      expect(h.session.missingFromPlayer, contains('the light view'));
      expect(notified, greaterThan(0), reason: 'the screens are told');
      expect(seen, ['reset']);
      h.elapse(const Duration(minutes: 1));
      expect(opens(h).length, 1, reason: 'not asked again on this link');
      expect(h.count('ListPlaylists'), 0);
      unawaited(sub.cancel());
    });

    scenario('is asked again once it has been updated', (h, _) {
      h.player.hasStreams = false;
      h.connect();
      final seen = <String>[];
      final sub = watch(h, seen);
      h.settle();
      expect(h.session.stageMissingFromPlayer, isTrue);

      // mtrack is updated and restarts; the phone connects again.
      h.player.disconnect();
      h.settle();
      expect(h.session.stageMissingFromPlayer, isTrue, reason: 'kept, dimmed');
      h.player.hasStreams = true;
      h.elapse(const Duration(seconds: 1));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.stageMissingFromPlayer, isFalse);
      expect(h.session.missingFromPlayer, isEmpty);
      expect(opens(h).length, 1);
      expect(seen, ['reset']);
      unawaited(sub.cancel());
    });
  });
}
