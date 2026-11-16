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
import 'package:mtrack_remote/src/app/levels.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/stage_watch.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';

import '../support/session_harness.dart';

void main() {
  /// Runs [body] with a harness that has every way of reaching a player
  /// (Bluetooth, the network, browsing and scanning), in fake time, and
  /// checks that nothing is left running after the session is disposed of.
  void scenario(
    String name,
    void Function(Harness h, FakeAsync async) body, {
    bool known = false,
  }) {
    test(name, () {
      fakeAsync((async) {
        final h = Harness(
          async,
          known: known,
          withWifi: true,
          withBrowser: true,
          withProbe: true,
        );
        body(h, async);
        h.session.dispose();
        h.settle();
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  /// Everything the session asked of a radio or the network.
  List<String> reached(Harness h) => [
    ...h.link.calls,
    ...h.wifi!.attempts,
    ...h.probe!.attempts,
  ];

  void startDemo(Harness h) {
    h.session.startDemo();
    h.settle();
  }

  group('the demo', () {
    scenario('connects to a simulated player and asks nothing of any radio', (
      h,
      async,
    ) {
      startDemo(h);
      final s = h.session;
      expect(s.phase, ConnectionPhase.ready);
      expect(s.inDemo, isTrue);
      expect(s.carrier, Carrier.demo);
      expect(s.playerName, 'Demo player');
      expect(s.status!.songName, 'Harbor Lights');
      expect(reached(h), isEmpty, reason: 'no Bluetooth, no network');
      expect(h.player.received, isEmpty);
      expect(h.wake.enabled, isTrue, reason: 'the screen stays on');
      expect(s.controlsDisabled, isFalse);
    });

    scenario('is never saved, and the store is never touched', (h, async) {
      startDemo(h);
      h.elapse(const Duration(minutes: 2));
      h.session.leaveDemo();
      h.settle();
      expect(h.store.player, isNull);
      expect(h.store.saves, 0);
      expect(h.store.clears, 0);
    });

    scenario('has what every tab shows, as a current player sends it', (
      h,
      async,
    ) {
      startDemo(h);
      final s = h.session;
      // Now and Sections.
      expect(s.entries.map((e) => e.songName), [
        'Harbor Lights',
        'Copper Wire',
        'Sundial',
        'Long Way Down',
      ]);
      expect(s.position, 0);
      expect(s.details!.name, 'Harbor Lights');
      expect(s.details!.sections, isNotEmpty);
      expect(s.details!.sections.every((span) => span.hasTimes), isTrue);
      expect(s.nextEntry!.songName, 'Copper Wire');
      // Lights: the ambient light and the lock.
      expect(s.ambientMode, 'auto');
      expect(s.ambientState, 'playing');
      expect(s.lockAvailable, isTrue);
      expect(s.locked, isFalse);
      // No note about an older player, anywhere.
      expect(s.missingFromPlayer, isEmpty);
      expect(s.stageMissingFromPlayer, isFalse);
      expect(s.lightingAssets.tooOld, isFalse);
      // Levels.
      s.levels.setVisible(true);
      h.settle();
      expect(s.levels.phase, LevelsPhase.ready);
      expect(s.levels.tracks.map((t) => t.name), contains('click'));
      // Library: the songs, and the playlists for the picker.
      s.songLibrary.load();
      s.loadPlaylists();
      h.settle();
      expect(s.songLibrary.songs, hasLength(8));
      expect(s.songLibrary.failure, isNull);
      expect(s.songLibrary.tooOld, isFalse);
      expect(s.choices!.names, ['Rehearsal list', 'Short set', 'all_songs']);
      expect(s.choices!.active, 'Short set');
    });

    scenario('shows the stage: a layout, then frames, and no models', (
      h,
      async,
    ) {
      startDemo(h);
      final events = <StageEvent>[];
      final sub = h.session.watchStage().listen(events.add);
      h.elapse(const Duration(milliseconds: 500));
      expect(events.first, isA<StageReset>());
      final layout = pp.StageUpdate.fromBuffer((events[1] as StageBytes).bytes);
      expect(layout.hasLayout(), isTrue);
      expect(layout.layout.fixtures, isNotEmpty);
      expect(events.whereType<StageBytes>().length, greaterThanOrEqualTo(2));
      // The 3D view asks for nothing the demo has: a player with no models.
      Object? failure = 'not asked';
      Object? bytes = 'not answered';
      h.session.lightingAssets.get('rigs/par.glb').then<void>((v) {
        bytes = v;
      }, onError: (Object e) => failure = e);
      h.settle();
      expect(bytes, isNull);
      expect(failure, 'not asked');
      expect(h.session.lightingAssets.tooOld, isFalse);
      sub.cancel();
    });

    scenario('plays, with a clock that runs, and ends the song', (h, async) {
      startDemo(h);
      final s = h.session;
      expect(h.run(s.play), isNull);
      h.elapse(const Duration(seconds: 3));
      expect(s.playing, isTrue);
      expect(s.clock.position, const Duration(seconds: 3));
      expect(h.run(() => s.seekToSection('Verse')), isNull);
      expect(h.run(s.stop), isNull);
      expect(s.playing, isFalse);
      // A play from near the end plays the song out and moves on.
      expect(h.run(s.play), isNull);
      expect(h.run(() => s.seekToSection('Outro')), isNull);
      h.elapse(const Duration(seconds: 20));
      expect(s.status!.songName, 'Copper Wire');
      expect(s.playing, isFalse);
      expect(s.autoPlaySong, 'Copper Wire');
      expect(s.autoPlayRemaining, isNotNull);
      expect(h.run(s.cancelAutoPlay), isNull);
      expect(s.autoPlaySong, isNull);
    });

    scenario('refuses what the real player refuses, in the real words', (
      h,
      async,
    ) {
      startDemo(h);
      final s = h.session;
      expect(h.run(s.stop), contains('not playing'));
      expect(h.run(s.play), isNull);
      expect(h.run(s.play), contains('already playing'));
      expect(
        h.run(() => s.switchPlaylist('Rehearsal list')),
        contains('while playing'),
      );
    });

    scenario('edits in memory, and the lock stops the edits', (h, async) {
      startDemo(h);
      final s = h.session;
      expect(s.startNewPlaylist('Encore'), isNull);
      s.editing!.addSongs(['Sundial', 'Night Shift']);
      s.editing!.save();
      h.settle();
      expect(s.editing, isNull, reason: 'saved');
      s.loadPlaylists();
      h.settle();
      expect(s.choices!.names, contains('Encore'));
      // Locked, a save is refused as the real player refuses it.
      expect(h.run(() => s.setLock(true)), isNull);
      h.settle();
      expect(s.locked, isTrue);
      expect(s.startEditing(), isNotNull);
      expect(h.run(() => s.setLock(false)), isNull);
      // Deleting a playlist.
      expect(
        h.run(() => s.deletePlaylist('Encore', confirm: () async => true)),
        isNull,
      );
      expect(s.choices!.names, isNot(contains('Encore')));
      // Levels and ambient light.
      expect(h.run(() => s.setAmbientMode('off')), isNull);
      expect(s.ambientMode, 'off');
      s.levels.setVisible(true);
      h.settle();
      expect(h.run(() => s.levels.toggleMute('click')), isNull);
      h.settle();
      expect(
        s.levels.tracks.firstWhere((t) => t.name == 'click').muted,
        isTrue,
      );
    });

    scenario('is gone on leaving, with everything it changed', (h, async) {
      startDemo(h);
      final s = h.session;
      s.startNewPlaylist('Encore');
      s.editing!.addSongs(['Sundial']);
      s.editing!.save();
      h.run(() => s.setLock(true));
      h.run(() => s.setAmbientMode('off'));
      h.run(s.play);
      s.levels.setVisible(true);
      h.settle();
      s.leaveDemo();
      h.settle();
      expect(s.inDemo, isFalse);
      expect(s.phase, ConnectionPhase.noPlayer);
      expect(s.status, isNull);
      expect(s.playerName, isNull);
      expect(s.carrier, isNull);
      expect(s.entries, isEmpty);
      expect(s.details, isNull);
      expect(s.editing, isNull);
      expect(s.songLibrary.songs, isNull);
      expect(h.wake.enabled, isFalse);
      // A new visit starts afresh.
      startDemo(h);
      expect(s.status!.playing, isFalse);
      expect(s.locked, isFalse);
      expect(s.ambientMode, 'auto');
      s.loadPlaylists();
      h.settle();
      expect(s.choices!.names, ['Rehearsal list', 'Short set', 'all_songs']);
    });

    scenario('is left by forgetting too, and forgets no player', (h, async) {
      startDemo(h);
      h.session.forget();
      h.settle();
      expect(h.session.inDemo, isFalse);
      expect(h.store.clears, 0);
    });

    scenario('is started and left only once', (h, async) {
      h.session.leaveDemo();
      h.settle();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      startDemo(h);
      final status = h.session.status;
      startDemo(h);
      expect(h.session.inDemo, isTrue);
      expect(h.session.status, same(status), reason: 'not started over');
    });

    scenario('does not go looking for a player, or for the network', (
      h,
      async,
    ) {
      startDemo(h);
      h.elapse(const Duration(minutes: 5));
      expect(reached(h), isEmpty);
      expect(h.session.phase, ConnectionPhase.ready);
    });

    test(
      'goes back to the connect screen if the simulation will not start',
      () {
        fakeAsync((async) {
          final h = Harness(async, known: false);
          final session = Session(
            link: h.rig.link,
            store: h.store,
            demoChannel: () => throw StateError('no simulation'),
            monotonic: () => async.elapsed,
          );
          session.startDemo();
          async.flushMicrotasks();
          expect(session.inDemo, isFalse);
          expect(session.phase, ConnectionPhase.noPlayer);
          session.dispose();
          h.session.dispose();
          async.flushMicrotasks();
        });
      },
    );
  });

  group('with a player remembered', () {
    scenario('leaves it alone, and returns to it on leaving', (h, async) {
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.bluetooth);
      final remembered = h.store.player;
      final saves = h.store.saves;
      final connects = h.count('Status');

      startDemo(h);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.demo);
      expect(h.session.playerName, 'Demo player');
      expect(h.store.player, same(remembered));
      expect(h.store.saves, saves);
      expect(h.store.clears, 0);
      // Nothing of the demo reached the real player.
      h.session.play();
      h.settle();
      expect(h.count('Play'), 0);
      expect(h.count('Status'), connects);

      h.session.leaveDemo();
      h.settle();
      expect(h.session.inDemo, isFalse);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.session.playerName, 'stage');
      expect(h.store.player, same(remembered));
      expect(h.store.clears, 0);
    }, known: true);

    scenario('does not retry the one it cannot reach while in the demo', (
      h,
      async,
    ) {
      h.link.connectFailures = 1000;
      h.connect();
      h.elapse(const Duration(seconds: 30));
      expect(h.session.phase, ConnectionPhase.lost);
      final tries = h.link.calls.where((c) => c.startsWith('connect')).length;
      expect(tries, greaterThan(1));

      startDemo(h);
      expect(h.session.phase, ConnectionPhase.ready);
      h.elapse(const Duration(minutes: 2));
      expect(
        h.link.calls.where((c) => c.startsWith('connect')),
        hasLength(tries),
        reason: 'no backoff while the demo is on',
      );
      expect(h.store.player, isNotNull);

      // Leaving looks for it again, as the app does at start-up.
      h.session.leaveDemo();
      h.settle();
      expect(h.session.inDemo, isFalse);
      expect(
        h.session.phase,
        anyOf(ConnectionPhase.connecting, ConnectionPhase.lost),
      );
      expect(h.session.playerName, 'stage');
      h.elapse(const Duration(seconds: 10));
      expect(
        h.link.calls.where((c) => c.startsWith('connect')).length,
        greaterThan(tries),
      );
      expect(h.store.clears, 0);
    }, known: true);

    scenario('never starts in the demo: starting the app connects to it', (
      h,
      async,
    ) {
      h.connect();
      expect(h.session.inDemo, isFalse);
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.session.playerName, 'stage');
    }, known: true);

    scenario('lets the demo begin while the app is still starting', (h, async) {
      h.session.start();
      h.session.startDemo();
      h.settle();
      h.elapse(const Duration(seconds: 5));
      expect(h.session.inDemo, isTrue);
      expect(h.session.carrier, Carrier.demo);
      expect(
        h.link.calls.where((c) => c.startsWith('connect')),
        isEmpty,
        reason: 'the remembered player is not connected to behind the demo',
      );
      h.session.leaveDemo();
      h.settle();
      expect(h.session.carrier, Carrier.bluetooth);
    }, known: true);
  });

  scenario('never starts in the demo with no player remembered', (h, async) {
    h.session.start();
    h.settle();
    h.elapse(const Duration(seconds: 5));
    expect(h.session.inDemo, isFalse);
    expect(h.session.phase, ConnectionPhase.noPlayer);
    expect(h.session.carrier, isNull);
  });
}
