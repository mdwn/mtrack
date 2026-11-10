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

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/failure.dart';
import 'package:mtrack_remote/src/app/player_link.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import '../support/fake_player.dart';
import '../support/rig.dart';
import '../support/session_harness.dart';

void main() {
  late List<Session> sessions;

  setUp(() => sessions = []);

  /// Runs [body] with a harness, and checks nothing is left running.
  void scenario(
    String name,
    void Function(Harness h, FakeAsync async) body, {
    bool known = true,
  }) {
    test(name, () {
      fakeAsync((async) {
        final h = Harness(async, known: known);
        sessions.add(h.session);
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

  group('connecting', () {
    scenario('runs connect, pair, subscribe, read, fetch in that order', (
      h,
      _,
    ) {
      h.link.paired.clear();
      h.connect();
      expect(h.link.calls, [
        'adapterState',
        'connect:p1',
        'isPaired:p1',
        'pair:p1',
        'subscribe:p1',
        'readStatus',
        'call:Status',
        'call:GetPlaylist',
      ]);
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('does not pair a phone that is already paired', (h, _) {
      h.connect();
      expect(h.link.calls.where((c) => c.startsWith('pair')), isEmpty);
      expect(h.link.calls, contains('subscribe:p1'));
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('shows the pairing phase while pairing', (h, _) {
      h.link.paired.clear();
      h.link.pairingHeld = Completer<void>();
      h.connect();
      expect(h.session.phase, ConnectionPhase.pairing);
      expect(h.link.calls, isNot(contains('subscribe:p1')));
      h.link.pairingHeld!.complete();
      h.settle();
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('a refusal to pair is its own state, and nothing retries it', (
      h,
      _,
    ) {
      h.link.paired.clear();
      h.link.pairingRefused = true;
      h.connect();
      expect(h.session.phase, ConnectionPhase.pairingRefused);
      expect(h.link.calls, isNot(contains('subscribe:p1')));
      final attempts = h.link.calls
          .where((c) => c.startsWith('connect'))
          .length;
      h.elapse(const Duration(minutes: 2));
      expect(
        h.link.calls.where((c) => c.startsWith('connect')).length,
        attempts,
      );
      expect(h.player.currentState, TransportState.disconnected);
    });

    scenario('tries again on request once the player accepts', (h, _) {
      h.link.paired.clear();
      h.link.pairingRefused = true;
      h.connect();
      h.link.pairingRefused = false;
      h.session.retryNow();
      h.settle();
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('remembers the player only once it has connected', (h, _) {
      h.store.player = null;
      h.link.paired.clear();
      h.link.pairingRefused = true;
      h.session.connectTo(
        const DiscoveredPlayer(id: 'p1', name: 'stage', rssi: -50),
      );
      h.settle();
      expect(h.session.phase, ConnectionPhase.pairingRefused);
      expect(h.store.player, isNull);
      h.link.pairingRefused = false;
      h.session.retryNow();
      h.settle();
      expect(h.store.player, remembered);
    }, known: false);

    scenario('a first connection that fails goes back to choosing', (h, _) {
      h.link.connectFailures = 1;
      h.session.connectTo(
        const DiscoveredPlayer(id: 'p1', name: 'stage', rssi: -50),
      );
      h.settle();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.session.connectError, contains('Could not reach'));
      expect(h.store.player, isNull);
      h.elapse(const Duration(minutes: 1));
      expect(
        h.link.calls.where((c) => c.startsWith('connect')).length,
        1,
        reason: 'no retries for a player never connected to',
      );
    }, known: false);

    scenario('does nothing at start when no player is remembered', (h, _) {
      h.connect();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.link.calls, isEmpty);
    }, known: false);

    scenario('stays out of the way of a Bluetooth that is off', (h, _) {
      h.link.adapter = AdapterState.off;
      h.connect();
      expect(h.session.phase, ConnectionPhase.lost);
      expect(h.session.adapter, AdapterState.off);
      expect(h.link.calls, isNot(contains('connect:p1')));
    });

    scenario('forgetting the player disconnects and clears it', (h, _) {
      h.connect();
      var done = false;
      h.session.forget().then((_) => done = true);
      h.settle();
      expect(done, isTrue, reason: 'forget did not finish');
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.store.player, isNull);
      expect(h.session.status, isNull);
      expect(h.player.currentState, TransportState.disconnected);
      expect(h.wake.enabled, isFalse);
      h.elapse(const Duration(minutes: 1));
      expect(h.session.phase, ConnectionPhase.noPlayer);
    });

    scenario('cancelling a connection to a new player goes back to choosing', (
      h,
      _,
    ) {
      h.store.player = null;
      h.link.paired.clear();
      h.link.pairingHeld = Completer<void>();
      h.session.connectTo(
        const DiscoveredPlayer(id: 'p1', name: 'stage', rssi: -50),
      );
      h.settle();
      expect(h.session.phase, ConnectionPhase.pairing);
      h.session.cancelConnecting();
      h.settle();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      h.link.pairingHeld!.complete();
      h.settle();
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.store.player, isNull);
    }, known: false);
  });

  group('reconnecting', () {
    scenario('connects again when a call goes unanswered on a live link', (
      h,
      _,
    ) {
      // The player restarted and the radio kept the link: nothing says the
      // connection is gone, and nothing answers.
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      int connects() =>
          h.link.calls.where((c) => c.startsWith('connect')).length;
      final before = connects();

      h.player.handlers['Next'] = (_) => FakeReply.drop();
      String? said;
      unawaited(h.session.next().then((m) => said = m));
      h.elapse(const Duration(seconds: 5));
      expect(said, isNotNull);
      expect(h.session.phase, ConnectionPhase.lost);

      h.elapse(const Duration(seconds: 1));
      expect(connects() - before, 1);
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('connects again when a write is refused on a live link', (h, _) {
      // The same restart, seen the other way: the phone's write is refused
      // at once, with the link still up.
      h.connect();
      int connects() =>
          h.link.calls.where((c) => c.startsWith('connect')).length;
      final before = connects();

      h.player.refuseWrites = true;
      String? said;
      unawaited(h.session.next().then((m) => said = m));
      h.settle();
      expect(said, noAnswerMessage);
      expect(h.session.phase, ConnectionPhase.lost);

      h.elapse(const Duration(seconds: 1));
      expect(connects() - before, 1);
    });

    scenario('retries after 1 s, 2 s, 5 s, then every 5 s', (h, _) {
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      int connects() =>
          h.link.calls.where((c) => c.startsWith('connect')).length;
      final before = connects();

      h.link.connectFailures = 4;
      h.player.disconnect();
      h.settle();
      expect(h.session.phase, ConnectionPhase.lost);

      h.elapse(const Duration(milliseconds: 999));
      expect(connects() - before, 0);
      h.elapse(const Duration(milliseconds: 1));
      expect(connects() - before, 1);
      h.elapse(const Duration(seconds: 1, milliseconds: 999));
      expect(connects() - before, 1);
      h.elapse(const Duration(milliseconds: 1));
      expect(connects() - before, 2);
      h.elapse(const Duration(seconds: 4, milliseconds: 999));
      expect(connects() - before, 2);
      h.elapse(const Duration(milliseconds: 1));
      expect(connects() - before, 3);
      h.elapse(const Duration(seconds: 5));
      expect(connects() - before, 4);
      expect(h.session.phase, ConnectionPhase.lost);
      h.elapse(const Duration(seconds: 5));
      expect(connects() - before, 5);
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('a launch that cannot connect retries on the same schedule', (
      h,
      _,
    ) {
      h.link.connectFailures = 1;
      h.connect();
      expect(h.session.phase, ConnectionPhase.lost);
      h.elapse(const Duration(seconds: 1));
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('keeps the last state, frozen, while the link is down', (
      h,
      async,
    ) {
      h.rig.playing = true;
      h.rig.elapsed = const Duration(seconds: 10);
      h.rig.publish();
      h.connect();
      h.elapse(const Duration(seconds: 2));
      expect(h.session.clock.position, const Duration(seconds: 12));
      h.link.connectFailures = 100;
      h.player.disconnect();
      h.settle();
      final frozen = h.session.clock.position;
      expect(h.session.status?.songName, 'First');
      h.elapse(const Duration(seconds: 30));
      expect(h.session.clock.position, frozen);
      expect(h.session.controlsDisabled, isTrue);
    });

    scenario('commands fail plainly while the link is down', (h, _) {
      h.connect();
      h.link.connectFailures = 100;
      h.player.disconnect();
      h.settle();
      final sent = h.player.received.length;
      expect(h.run(h.session.play), notConnectedMessage);
      expect(h.player.received.length, sent);
    });

    scenario('a drop while connecting is retried, not left hanging', (h, _) {
      h.link.connectDelay = const Duration(milliseconds: 100);
      h.link.connectFailures = 1;
      h.session.start();
      h.elapse(const Duration(milliseconds: 100));
      expect(h.session.phase, ConnectionPhase.lost);
      h.elapse(const Duration(seconds: 2));
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario('dispose cancels a pending retry', (h, async) {
      h.link.connectFailures = 100;
      h.connect();
      expect(h.session.phase, ConnectionPhase.lost);
      expect(async.pendingTimers, isNotEmpty);
      final attempts = h.link.calls.length;
      h.session.dispose();
      expect(async.pendingTimers, isEmpty);
      h.elapse(const Duration(minutes: 1));
      expect(h.link.calls.length, attempts);
    });

    scenario('dispose releases the player', (h, _) {
      h.connect();
      expect(h.player.hasStatusListener, isTrue);
      h.session.dispose();
      h.settle();
      expect(h.player.hasStatusListener, isFalse);
      expect(h.player.hasResponseListener, isFalse);
      expect(h.player.currentState, TransportState.disconnected);
    });
  });

  group('following the player', () {
    scenario('holds the status and drives the clock from it', (h, _) {
      h.rig.playing = true;
      h.rig.elapsed = const Duration(seconds: 5);
      h.rig.publish();
      h.connect();
      expect(h.session.playing, isTrue);
      expect(h.session.clock.position, const Duration(seconds: 5));
      h.elapse(const Duration(seconds: 3));
      expect(h.session.clock.position, const Duration(seconds: 8));
      h.rig.elapsed = const Duration(seconds: 20);
      h.rig.publish();
      h.settle();
      expect(h.session.clock.position, const Duration(seconds: 20));
    });

    scenario('fetches the song details and playlist after the status', (h, _) {
      h.connect();
      expect(h.session.details?.name, 'First');
      expect(h.session.details!.sections.map((s) => s.name), [
        'Intro',
        'Verse',
      ]);
      expect(h.session.details!.sections.first.hasTimes, isTrue);
      expect(h.session.entries.map((e) => e.songName), [
        'First',
        'Second',
        'Third',
      ]);
      expect(h.session.position, 0);
      expect(h.session.nextEntry?.songName, 'Second');
    });

    scenario('a new song fetches its details once', (h, _) {
      h.connect();
      expect(h.count('Status'), 1);
      h.rig.index = 1;
      h.rig.publish();
      h.settle();
      expect(h.count('Status'), 2);
      expect(h.session.details?.name, 'Second');
      expect(h.session.details!.sections, isEmpty);
    });

    scenario('a status that changes only the time fetches nothing', (h, _) {
      h.connect();
      for (var i = 1; i <= 5; i++) {
        h.rig.elapsed = Duration(seconds: i);
        h.rig.publish();
        h.settle();
      }
      expect(h.count('Status'), 1);
      expect(h.count('GetPlaylist'), 1);
    });

    scenario('a new playlist refetches the playlist and not the song', (h, _) {
      h.rig.playlists['Other'] = [RigSong('Solo')];
      h.connect();
      h.rig.songs = h.rig.playlists['Other']!;
      h.rig.playlist = 'Other';
      h.rig.index = 0;
      h.rig.publish();
      h.settle();
      expect(h.count('GetPlaylist'), 2);
      expect(h.session.entries.map((e) => e.songName), ['Solo']);
      expect(h.session.details?.name, 'Solo');
    });

    scenario('details that fail to load are asked for again', (h, _) {
      var calls = 0;
      final ok = h.player.handlers['Status']!;
      h.player.handlers['Status'] = (r) {
        calls++;
        return calls == 1 ? FakeReply.error(14, 'busy') : ok(r);
      };
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.details, isNull);
      h.elapse(detailsRetryDelay);
      expect(h.session.details?.name, 'First');
    });

    scenario('the loop and the waiting song come from the status', (h, _) {
      h.rig.playing = true;
      h.rig.loop = 'Verse';
      h.rig.autoPlaySong = 'Second';
      h.rig.autoPlayIn = const Duration(seconds: 10);
      h.rig.publish();
      h.connect();
      expect(h.session.loopingSection, 'Verse');
      expect(h.session.autoPlaySong, 'Second');
      expect(h.session.autoPlayRemaining, const Duration(seconds: 10));
      h.elapse(const Duration(seconds: 4));
      expect(h.session.autoPlayRemaining, const Duration(seconds: 6));
      h.elapse(const Duration(seconds: 60));
      expect(h.session.autoPlayRemaining, Duration.zero);
      h.rig.loop = null;
      h.rig.autoPlaySong = null;
      h.rig.publish();
      h.settle();
      expect(h.session.loopingSection, isNull);
      expect(h.session.autoPlayRemaining, isNull);
    });

    scenario('lists the playlists when asked', (h, _) {
      h.rig.playlists['Other'] = [RigSong('Solo')];
      h.connect();
      expect(h.session.choices, isNull);
      h.run(h.session.loadPlaylists);
      expect(h.session.choices?.names, ['Other', 'Set']);
      expect(h.session.choices?.active, 'Set');
    });
  });

  group('actions', () {
    scenario('send the call they are named for', (h, _) {
      h.connect();
      for (final entry in <String, Future<String?> Function()>{
        'Play': h.session.play,
        'Stop': h.session.stop,
        'Next': h.session.next,
        'Previous': h.session.previous,
        'StopSectionLoop': h.session.stopLoop,
        'CancelAutoPlay': h.session.cancelAutoPlay,
      }.entries) {
        final before = h.count(entry.key);
        expect(h.run(entry.value), isNull, reason: entry.key);
        expect(h.count(entry.key), before + 1, reason: entry.key);
      }
    });

    scenario('carry the name they act on', (h, _) {
      h.rig.playlists['Other'] = [RigSong('Solo')];
      h.connect();
      expect(h.run(() => h.session.navigateTo(2, 'Third')), isNull);
      final request = pp.NavigateToSongRequest.fromBuffer(
        h.last('NavigateToSong').payload,
      );
      expect(request.songName, 'Third');
      expect(request.position, 2);
      expect(h.session.position, 2);
      expect(h.run(() => h.session.switchPlaylist('Other')), isNull);
      expect(
        pp.SwitchToPlaylistRequest.fromBuffer(
          h.last('SwitchToPlaylist').payload,
        ).playlistName,
        'Other',
      );
      expect(h.session.entries.map((e) => e.songName), ['Solo']);
      h.rig.playlist = 'Set';
      h.rig.songs = h.rig.playlists['Set']!;
      h.rig.publish();
      h.settle();
      expect(h.run(() => h.session.seekToSection('Verse')), isNull);
      expect(
        pp.SeekToSectionRequest.fromBuffer(h.last('SeekToSection').payload)
            .sectionName,
        'Verse',
      );
      h.rig.playing = true;
      expect(h.run(() => h.session.loopSection('Intro')), isNull);
      expect(
        pp.LoopSectionRequest.fromBuffer(h.last('LoopSection').payload)
            .sectionName,
        'Intro',
      );
      expect(h.session.loopingSection, 'Intro');
    });

    scenario('a second tap while one is out sends nothing', (h, async) {
      h.connect();
      h.player.handlers['Play'] = (_) =>
          FakeReply.ok(pp.PlayResponse(), delay: const Duration(seconds: 1));
      String? first;
      var firstDone = false;
      h.session.play().then((v) {
        first = v;
        firstDone = true;
      });
      h.settle();
      expect(h.session.isBusy(ActionKind.play), isTrue);
      expect(h.run(h.session.play), isNull);
      expect(h.count('Play'), 1);
      h.elapse(const Duration(seconds: 1));
      expect(firstDone, isTrue);
      expect(first, isNull);
      expect(h.session.isBusy(ActionKind.play), isFalse);
      h.run(h.session.play, after: const Duration(seconds: 1));
      expect(h.count('Play'), 2);
    });

    scenario('different actions do not hold each other up', (h, _) {
      h.connect();
      h.player.handlers['Play'] = (_) =>
          FakeReply.ok(pp.PlayResponse(), delay: const Duration(seconds: 1));
      h.session.play();
      h.settle();
      h.run(() => h.session.navigateTo(1, 'Second'));
      expect(h.count('NavigateToSong'), 1);
      h.elapse(const Duration(seconds: 1));
    });

    scenario('a locked player is reported as locked', (h, _) {
      h.connect();
      h.player.handlers['Next'] = (_) => FakeReply.error(7, 'locked');
      expect(
        h.run(h.session.next),
        'The player is locked, or does not allow this from a phone.',
      );
    });

    scenario('a refusal in the player\'s words is passed on', (h, _) {
      h.connect();
      expect(
        h.run(() => h.session.loopSection('Intro')),
        'Cannot loop section: no song is playing',
      );
      h.rig.playing = true;
      expect(
        h.run(() => h.session.switchPlaylist('Set')),
        'Cannot switch playlist while playing',
      );
    });

    scenario('a position past the end is the player\'s message', (h, _) {
      h.connect();
      expect(
        h.run(() => h.session.navigateTo(9, 'Nope')),
        'the playlist has no entry 9',
      );
    });

    scenario('a changed playlist is reported and fetched again', (h, _) {
      h.connect();
      expect(h.count('GetPlaylist'), 1);
      // The player's list changed under the phone: entry 1 is another song.
      h.rig.songs[1] = RigSong('Replacement', lengthSeconds: 50);
      final failure = h.run(() => h.session.navigateTo(1, 'Second'));
      expect(failure, contains('playlist has changed'));
      expect(h.count('GetPlaylist'), 2);
      expect(h.session.entries.map((e) => e.songName), contains('Replacement'));
      expect(h.session.position, 0);
    });

    scenario('going to the entry it is on sends nothing', (h, _) {
      h.connect();
      expect(h.run(() => h.session.navigateTo(0, 'First')), isNull);
      expect(h.count('NavigateToSong'), 0);
    });

    scenario('no answer is "did not answer"', (h, _) {
      h.connect();
      h.player.handlers['Play'] = (_) => FakeReply.drop();
      expect(
        h.run(h.session.play, after: const Duration(seconds: 6)),
        noAnswerMessage,
      );
      h.player.handlers['Stop'] = (_) => FakeReply.error(14, 'gone');
      expect(h.run(h.session.stop), noAnswerMessage);
      h.player.handlers['Next'] = (_) => FakeReply.error(4, 'slow');
      expect(h.run(h.session.next), noAnswerMessage);
    });

    scenario('failures are also sent to the notices', (h, _) {
      h.connect();
      final notices = <String>[];
      h.session.notices.listen(notices.add);
      h.player.handlers['Play'] = (_) => FakeReply.error(7, 'no');
      h.run(h.session.play);
      expect(notices, hasLength(1));
      h.run(h.session.stop);
      expect(notices, hasLength(1));
    });

    scenario('a failure frees the control for another try', (h, _) {
      h.connect();
      h.player.handlers['Play'] = (_) => FakeReply.error(9, 'not now');
      h.run(h.session.play);
      expect(h.session.isBusy(ActionKind.play), isFalse);
      h.run(h.session.play);
      expect(h.count('Play'), 2);
    });
  });

  group('scanning', () {
    scenario('lists players strongest first and stops when one is chosen', (
      h,
      _,
    ) {
      h.session.startScan();
      h.settle();
      expect(h.session.scanning, isTrue);
      h.link.discover(const DiscoveredPlayer(id: 'a', name: 'far', rssi: -90));
      h.link.discover(const DiscoveredPlayer(id: 'b', name: 'near', rssi: -40));
      h.link.discover(const DiscoveredPlayer(id: 'a', name: 'far', rssi: -85));
      h.settle();
      expect(h.session.found.map((p) => p.name), ['near', 'far']);
      expect(h.session.found.last.rssi, -85);
      h.session.connectTo(h.session.found.first);
      h.settle();
      expect(h.link.scanning, isFalse);
      expect(h.session.scanning, isFalse);
    }, known: false);

    scenario('says when the scan ends', (h, _) {
      h.session.startScan();
      h.settle();
      h.link.endScan();
      h.settle();
      expect(h.session.scanning, isFalse);
      expect(h.session.found, isEmpty);
    }, known: false);

    scenario('does not scan with Bluetooth off, and scans once it is on', (
      h,
      _,
    ) {
      h.link.adapter = AdapterState.off;
      h.session.startScan();
      h.settle();
      expect(h.session.adapter, AdapterState.off);
      expect(h.link.calls, isNot(contains('scan')));
      h.link.adapter = AdapterState.ready;
      h.session.startScan();
      h.settle();
      expect(h.link.calls, contains('scan'));
    }, known: false);

    scenario('asks for permission again and scans if it is given', (h, _) {
      h.link.adapter = AdapterState.permissionDenied;
      h.session.startScan();
      h.settle();
      expect(h.session.adapter, AdapterState.permissionDenied);
      h.link.adapter = AdapterState.ready;
      h.session.requestAccess();
      h.settle();
      expect(h.link.calls, contains('requestAccess'));
      expect(h.session.adapter, AdapterState.ready);
      expect(h.session.scanning, isTrue);
    }, known: false);

    scenario('a scan that fails says so', (h, _) {
      h.session.startScan();
      h.settle();
      h.link.failScan(StateError('boom'));
      h.settle();
      expect(h.session.scanning, isFalse);
      expect(h.session.connectError, isNotNull);
    }, known: false);
  });

  group('keeping the screen on', () {
    scenario('only while connected and in the foreground', (h, _) {
      h.connect();
      expect(h.wake.enabled, isTrue);
      h.session.setForeground(false);
      h.settle();
      expect(h.wake.enabled, isFalse);
      h.session.setForeground(true);
      h.settle();
      expect(h.wake.enabled, isTrue);
      h.link.connectFailures = 100;
      h.player.disconnect();
      h.settle();
      expect(h.wake.enabled, isFalse);
    });

    scenario('is released when the session ends', (h, _) {
      h.connect();
      h.session.dispose();
      h.settle();
      expect(h.wake.enabled, isFalse);
    });

    scenario('is not asked for twice in a row', (h, _) {
      h.connect();
      h.session.setForeground(true);
      h.session.setForeground(true);
      h.settle();
      expect(h.wake.history, [false, true]);
    });
  });

  group('failure words', () {
    test('describe a status the player sent', () {
      expect(
        describeFailure(StateError('x')),
        contains('Something went wrong'),
      );
    });

    test('give a reason for each way a first connection can fail', () {
      final e = StateError('x');
      expect(describeConnectFailure(e, AdapterState.off), contains('off'));
      expect(
        describeConnectFailure(e, AdapterState.permissionDenied),
        contains('not allowed'),
      );
      expect(
        describeConnectFailure(e, AdapterState.unsupported),
        contains('Low Energy'),
      );
      expect(
        describeConnectFailure(e, AdapterState.ready),
        contains('Could not reach'),
      );
    });
  });

  test('a status message type is what the session shows', () {
    // Guards the contract the fake rig and the real player share: the app
    // reads `ble.Status`, whose loop field is optional.
    expect(ble.Status().hasLoopSection(), isFalse);
    expect(ble.Status(loopSection: 'x').hasLoopSection(), isTrue);
  });
}
