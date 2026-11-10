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
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/failure.dart';
import 'package:mtrack_remote/src/app/levels.dart';
import 'package:mtrack_remote/src/app/session.dart';

import '../support/fake_player.dart';
import '../support/rig.dart';
import '../support/session_harness.dart';

void main() {
  /// Runs [body] with a connected session whose Levels tab is on show.
  void scenario(
    String name,
    void Function(Harness h, Levels levels, List<String> notices) body, {
    bool visible = true,
    void Function(Rig rig)? setup,
  }) {
    test(name, () {
      fakeAsync((async) {
        final rig = Rig();
        setup?.call(rig);
        rig.publish();
        final h = Harness(async, rig: rig);
        final notices = <String>[];
        h.session.notices.listen(notices.add);
        h.connect();
        if (visible) {
          h.session.levels.setVisible(true);
          h.settle();
        }
        body(h, h.session.levels, notices);
        h.session.dispose();
        h.settle();
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  List<double> sentGains(Harness h) => h.player.received
      .where((r) => r.method == 'SetTrackGain')
      .map((r) => pp.SetTrackGainRequest.fromBuffer(r.payload).gainDb)
      .toList();

  double gainOf(Levels levels, String name) =>
      levels.tracks.firstWhere((t) => t.name == name).gainDb;

  const ms = Duration(milliseconds: 1);

  group('fetching', () {
    scenario('waits for the tab to be shown', (h, levels, _) {
      expect(h.count('GetTrackGains'), 0);
      levels.setVisible(true);
      h.settle();
      expect(h.count('GetTrackGains'), 1);
      expect(levels.phase, LevelsPhase.ready);
      expect([for (final t in levels.tracks) t.name], ['Click', 'Cue', 'Keys']);
      expect(gainOf(levels, 'Cue'), -3);
    }, visible: false);

    scenario('follows a change of revision while the tab is shown', (
      h,
      levels,
      _,
    ) {
      expect(h.count('GetTrackGains'), 1);
      h.rig.setGainElsewhere('Cue', -9);
      h.settle();
      expect(h.count('GetTrackGains'), 2);
      expect(gainOf(levels, 'Cue'), -9);
    });

    scenario('does not fetch for a status that changes nothing', (
      h,
      levels,
      _,
    ) {
      h.rig.publish();
      h.rig.publish();
      h.settle();
      expect(h.count('GetTrackGains'), 1);
    });

    scenario('misses a change while hidden and fetches when shown again', (
      h,
      levels,
      _,
    ) {
      levels.setVisible(false);
      h.rig.setGainElsewhere('Cue', -9);
      h.settle();
      expect(h.count('GetTrackGains'), 1);
      expect(gainOf(levels, 'Cue'), -3);
      levels.setVisible(true);
      h.settle();
      expect(h.count('GetTrackGains'), 2);
      expect(gainOf(levels, 'Cue'), -9);
    });

    scenario('does not fetch again on showing when nothing changed', (
      h,
      levels,
      _,
    ) {
      levels.setVisible(false);
      levels.setVisible(true);
      h.settle();
      expect(h.count('GetTrackGains'), 1);
    });

    scenario('fetches again after the link comes back', (h, levels, _) {
      h.player.disconnect();
      h.settle();
      h.elapse(const Duration(seconds: 2));
      expect(h.session.isReady, isTrue);
      expect(h.count('GetTrackGains'), 2);
    });

    scenario('an answer that is late does not overwrite a fader that moved', (
      h,
      levels,
      _,
    ) {
      final inner = h.player.handlers['GetTrackGains']!;
      h.player.handlers['GetTrackGains'] = (r) {
        h.player.handlers['GetTrackGains'] = inner;
        return FakeReply.ok(
          pp.GetTrackGainsResponse(
            gains: [pp.TrackGain(track: 'Click', gainDb: 5)],
          ),
          delay: const Duration(milliseconds: 500),
        );
      };
      h.rig.setGainElsewhere('Click', 5);
      h.settle();
      expect(h.count('GetTrackGains'), 2);
      levels.beginDrag('Click');
      levels.dragTo('Click', -30);
      levels.endDrag('Click', -30);
      h.elapse(const Duration(seconds: 1));
      expect(gainOf(levels, 'Click'), -30);
    });

    scenario('waits for a drag to end before fetching', (h, levels, _) {
      levels.beginDrag('Click');
      levels.dragTo('Click', -20);
      h.rig.setGainElsewhere('Cue', -9);
      h.elapse(const Duration(milliseconds: 500));
      expect(h.count('GetTrackGains'), 1);
      expect(gainOf(levels, 'Click'), -20);
      levels.endDrag('Click', -20);
      h.elapse(const Duration(seconds: 1));
      expect(h.count('GetTrackGains'), 2);
      expect(gainOf(levels, 'Click'), -20);
      expect(gainOf(levels, 'Cue'), -9);
    });
  });

  group('the faders', () {
    scenario('send at most one write per track every 100 ms, and the last', (
      h,
      levels,
      _,
    ) {
      final times = <Duration>[];
      final inner = h.player.handlers['SetTrackGain']!;
      h.player.handlers['SetTrackGain'] = (r) {
        times.add(h.async.elapsed);
        return inner(r);
      };
      levels.beginDrag('Click');
      for (var i = 1; i <= 10; i++) {
        levels.dragTo('Click', -i.toDouble());
        h.elapse(ms * 5);
      }
      expect(sentGains(h), [-1], reason: 'the rest wait for the interval');
      levels.endDrag('Click', -20);
      h.elapse(const Duration(milliseconds: 200));
      expect(sentGains(h), [-1, -20]);
      expect(times[1] - times[0], const Duration(milliseconds: 100));
    });

    scenario('keep to the interval through a long drag', (h, levels, _) {
      final times = <Duration>[];
      final inner = h.player.handlers['SetTrackGain']!;
      h.player.handlers['SetTrackGain'] = (r) {
        times.add(h.async.elapsed);
        return inner(r);
      };
      levels.beginDrag('Click');
      for (var i = 1; i <= 100; i++) {
        levels.dragTo('Click', -i / 2);
        h.elapse(ms * 10);
      }
      levels.endDrag('Click', -50);
      h.elapse(const Duration(milliseconds: 300));
      expect(times.length, lessThanOrEqualTo(11));
      for (var i = 1; i < times.length; i++) {
        expect(
          times[i] - times[i - 1],
          greaterThanOrEqualTo(const Duration(milliseconds: 100)),
        );
      }
      expect(sentGains(h).last, -50);
    });

    scenario('are throttled one track at a time', (h, levels, _) {
      levels.dragTo('Click', -10);
      levels.dragTo('Cue', -10);
      levels.dragTo('Click', -11);
      h.settle();
      expect(sentGains(h), [-10, -10]);
    });

    scenario('keep only the newest value behind a slow link', (h, levels, _) {
      h.player.writeDelay = const Duration(milliseconds: 300);
      levels.beginDrag('Click');
      for (var i = 1; i <= 10; i++) {
        levels.dragTo('Click', -i.toDouble());
        h.elapse(ms * 20);
      }
      levels.endDrag('Click', -42);
      h.elapse(const Duration(milliseconds: 250));
      expect(sentGains(h), [-1], reason: 'the first write has just landed');
      h.elapse(const Duration(seconds: 2));
      expect(sentGains(h), [-1, -42], reason: 'nothing in between was queued');
      expect(gainOf(levels, 'Click'), -42);
    });

    scenario('land on 0 dB when let go within a dB of it', (h, levels, _) {
      levels.beginDrag('Click');
      levels.endDrag('Click', 0.8);
      h.elapse(const Duration(milliseconds: 200));
      levels.endDrag('Click', -1.0);
      levels.endDrag('Cue', -0.4);
      h.elapse(const Duration(milliseconds: 200));
      expect(gainOf(levels, 'Click'), 0);
      expect(gainOf(levels, 'Cue'), 0);
      expect(h.rig.tracks[0].gainDb, 0);
      expect(h.rig.tracks[1].gainDb, 0);
      levels.endDrag('Click', 1.5);
      levels.endDrag('Keys', -1.5);
      h.elapse(const Duration(milliseconds: 200));
      expect(gainOf(levels, 'Click'), 1.5);
      expect(gainOf(levels, 'Keys'), -1.5);
    });

    scenario('do not send a value the player already has', (h, levels, _) {
      levels.beginDrag('Click');
      levels.endDrag('Click', 0.3);
      h.elapse(const Duration(milliseconds: 200));
      expect(sentGains(h), isEmpty);
    });

    scenario('show the gain the player applied on release', (h, levels, _) {
      h.player.handlers['SetTrackGain'] = (_) =>
          FakeReply.ok(pp.SetTrackGainResponse(appliedGainDb: -5));
      levels.beginDrag('Click');
      levels.dragTo('Click', -8);
      levels.endDrag('Click', -8);
      h.elapse(const Duration(milliseconds: 200));
      expect(gainOf(levels, 'Click'), -5);
    });

    scenario('keep the range', (h, levels, _) {
      levels.endDrag('Click', 40);
      h.elapse(const Duration(milliseconds: 200));
      expect(gainOf(levels, 'Click'), maxGainDb);
      levels.endDrag('Click', -400);
      h.elapse(const Duration(milliseconds: 200));
      expect(gainOf(levels, 'Click'), minGainDb);
    });

    scenario('say so once when a write fails, and stop sending that drag', (
      h,
      levels,
      notices,
    ) {
      h.player.handlers['SetTrackGain'] = (_) =>
          FakeReply.error(13, 'the mixer is gone');
      levels.beginDrag('Click');
      for (var i = 1; i <= 10; i++) {
        levels.dragTo('Click', -i.toDouble());
        h.elapse(ms * 50);
      }
      expect(notices, ['the mixer is gone']);
      expect(sentGains(h).length, 1);
    });
  });

  group('the mutes', () {
    scenario('round trip through the player', (h, levels, _) {
      expect(h.run(() => levels.toggleMute('Cue')), isNull);
      expect(levels.tracks[1].muted, isTrue);
      expect(h.rig.tracks[1].muted, isTrue);
      expect(
        pp.SetTrackMuteRequest.fromBuffer(h.last('SetTrackMute').payload).muted,
        isTrue,
      );
      expect(h.run(() => levels.toggleMute('Cue')), isNull);
      expect(levels.tracks[1].muted, isFalse);
      expect(h.rig.tracks[1].muted, isFalse);
    });

    scenario('show the answer, not the request', (h, levels, _) {
      h.player.handlers['SetTrackMute'] = (_) =>
          FakeReply.ok(pp.SetTrackMuteResponse(muted: false));
      h.run(() => levels.toggleMute('Cue'));
      expect(levels.tracks[1].muted, isFalse);
    });

    scenario(
      'are busy while the call is out, and a second tap is not a second call',
      (h, levels, _) {
        h.player.handlers['SetTrackMute'] = (_) => FakeReply.ok(
          pp.SetTrackMuteResponse(muted: true),
          delay: const Duration(milliseconds: 300),
        );
        levels.toggleMute('Cue');
        levels.toggleMute('Cue');
        h.elapse(const Duration(milliseconds: 100));
        expect(levels.tracks[1].busy, isTrue);
        expect(h.count('SetTrackMute'), 1);
        h.elapse(const Duration(milliseconds: 500));
        expect(levels.tracks[1].busy, isFalse);
        expect(levels.tracks[1].muted, isTrue);
      },
    );

    scenario('say so when they fail', (h, levels, notices) {
      h.player.handlers['SetTrackMute'] = (_) =>
          FakeReply.error(5, "Track 'Cue' not found");
      expect(h.run(() => levels.toggleMute('Cue')), "Track 'Cue' not found");
      expect(notices, ["Track 'Cue' not found"]);
      expect(levels.tracks[1].muted, isFalse);
    });

    scenario('are refused without a link', (h, levels, notices) {
      h.link.connectFailures = 100;
      h.player.disconnect();
      h.settle();
      expect(h.run(() => levels.toggleMute('Cue')), notConnectedMessage);
    });
  });

  group('the states', () {
    scenario('an empty list', (h, levels, _) {
      expect(levels.phase, LevelsPhase.ready);
      expect(levels.tracks, isEmpty);
    }, setup: (rig) => rig.tracks = []);

    scenario('a player that refuses the call as not allowed is too old', (
      h,
      levels,
      notices,
    ) {
      expect(levels.phase, LevelsPhase.tooOld);
      expect(notices, isEmpty);
      expect(h.count('GetTrackGains'), 1, reason: 'and is not asked again');
      h.elapse(const Duration(seconds: 10));
      expect(h.count('GetTrackGains'), 1);
    }, setup: (rig) => rig.levelsAllowed = false);

    scenario('a failed first fetch says so, waits, and can be retried', (
      h,
      levels,
      notices,
    ) {
      final inner = h.player.handlers['GetTrackGains']!;
      h.player.handlers['GetTrackGains'] = (_) =>
          FakeReply.error(13, 'the mixer is gone');
      levels.setVisible(true);
      h.settle();
      expect(levels.phase, LevelsPhase.failed);
      expect(notices, ['the mixer is gone']);
      h.rig.publish();
      h.elapse(const Duration(seconds: 10));
      expect(h.count('GetTrackGains'), 1, reason: 'not asked again by itself');
      h.player.handlers['GetTrackGains'] = inner;
      levels.retry();
      h.settle();
      expect(levels.phase, LevelsPhase.ready);
      expect(levels.tracks, hasLength(3));
    }, visible: false);

    scenario('a failed refetch keeps the rows it has', (h, levels, notices) {
      h.player.handlers['GetTrackGains'] = (_) =>
          FakeReply.error(13, 'the mixer is gone');
      h.rig.setGainElsewhere('Cue', -9);
      h.settle();
      expect(levels.phase, LevelsPhase.ready);
      expect(levels.tracks, hasLength(3));
      expect(gainOf(levels, 'Cue'), -3);
      expect(notices, ['the mixer is gone']);
    });
  });

  group('the ambient light', () {
    scenario('is hidden when the status has no mode', (h, _, _) {
      expect(h.session.ambientMode, isNull);
      expect(h.session.ambientState, isNull);
    }, setup: (rig) => rig.ambientMode = null);

    scenario('follows the status', (h, _, _) {
      expect(h.session.ambientMode, 'auto');
      expect(h.session.ambientState, 'playing');
      h.rig.ambientState = 'pending';
      h.rig.publish();
      h.settle();
      expect(h.session.ambientState, 'pending');
    }, setup: (rig) => rig.ambientState = 'playing');

    scenario('takes the answer to a change at once', (h, _, _) {
      // The status is not sent, so the answer is all there is.
      h.player.handlers['SetAmbientMode'] = (_) => FakeReply.ok(
        pp.SetAmbientModeResponse(
          ambient: pp.AmbientStatus(mode: 'on', state: 'playing'),
        ),
      );
      expect(h.run(() => h.session.setAmbientMode('on')), isNull);
      expect(h.session.ambientMode, 'on');
      expect(h.session.ambientState, 'playing');
      expect(
        pp.SetAmbientModeRequest.fromBuffer(h.last('SetAmbientMode').payload)
            .mode,
        'on',
      );
    });

    scenario('is busy while the call is out', (h, _, _) {
      h.player.handlers['SetAmbientMode'] = (_) => FakeReply.ok(
        pp.SetAmbientModeResponse(
          ambient: pp.AmbientStatus(mode: 'off', state: 'off'),
        ),
        delay: const Duration(milliseconds: 300),
      );
      h.session.setAmbientMode('off');
      h.elapse(const Duration(milliseconds: 100));
      expect(h.session.isBusy(ActionKind.setAmbient), isTrue);
      h.elapse(const Duration(milliseconds: 500));
      expect(h.session.isBusy(ActionKind.setAmbient), isFalse);
      expect(h.session.ambientMode, 'off');
    });

    scenario('can be set when there is no show, and says so', (h, _, _) {
      expect(h.run(() => h.session.setAmbientMode('on')), isNull);
      expect(h.session.ambientMode, 'on');
      expect(h.session.ambientState, 'unavailable');
    }, setup: (rig) => rig.ambientShow = false);

    scenario('a refusal leaves the mode and says why', (h, _, notices) {
      h.player.handlers['SetAmbientMode'] = (_) =>
          FakeReply.error(7, 'method SetAmbientMode is not allowed');
      expect(h.run(() => h.session.setAmbientMode('on')), isNotNull);
      expect(h.session.ambientMode, 'auto');
      expect(notices, hasLength(1));
    });
  });
}
