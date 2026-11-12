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
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';
import 'package:mtrack_remote/src/app/stage_watch.dart';
import 'package:mtrack_remote/src/app/wifi_address.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

import '../support/fake_player.dart';
import '../support/session_harness.dart';

const String ip = '192.168.1.40';

/// A player paired by Bluetooth that has been told where it is on Wi-Fi.
const RememberedPlayer known = RememberedPlayer(
  id: 'p1',
  name: 'stage',
  hosts: [ip],
  port: 43234,
  hostname: 'stage',
);

pp.GetNetworkInfoResponse info({
  String hostname = 'stage',
  List<String> addresses = const [ip],
  int? port = 43234,
}) => pp.GetNetworkInfoResponse(
  hostname: hostname,
  addresses: addresses,
  grpcPort: port,
);

/// Starts the session and lets it give Wi-Fi its time before Bluetooth.
void connectAfterWifi(Harness h) {
  h.connect();
  h.elapse(wifiConnectTimeout);
}

void main() {
  /// Runs [body] with a harness that has a network, in fake time, and checks
  /// nothing is left running after the session is disposed.
  void scenario(
    String name,
    void Function(Harness h, FakeAsync async) body, {
    bool paired = true,
    RememberedPlayer? player = known,
    bool answers = true,
  }) {
    test(name, () {
      fakeAsync((async) {
        final h = Harness(
          async,
          withWifi: true,
          player: player,
          known: player != null,
        );
        h.rig.networkInfo = info();
        if (answers) {
          h.wifi!.reachable.add(ip);
        }
        body(h, async);
        h.session.dispose();
        h.settle();
        expect(async.pendingTimers, isEmpty);
      });
    });
  }

  group('choosing the radio', () {
    scenario('a player that answers on Wi-Fi is used over Wi-Fi, '
        'and Bluetooth is not touched', (h, _) {
      h.connect();
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.wifi);
      expect(h.wifi!.attempts, ['$ip:43234']);
      expect(h.link.calls, isEmpty);
      // It asked who it was talking to, then fetched what the status leaves
      // out, all over the network.
      expect(
        h.wifi!.live!.calls,
        containsAllInOrder(['GetNetworkInfo', 'Status']),
      );
      expect(h.session.status?.songName, 'First');
    });

    scenario('a player that does not answer on Wi-Fi is used over Bluetooth, '
        'after the Wi-Fi timeout and without an error', (h, async) {
      h.session.start();
      h.settle();
      expect(h.session.phase, ConnectionPhase.connecting);
      expect(h.link.calls, isEmpty, reason: 'Wi-Fi is given its two seconds');
      h.elapse(const Duration(seconds: 2));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.session.connectError, isNull);
      expect(h.link.calls, contains('subscribe:p1'));
      expect(h.wifi!.channels, isEmpty);
    }, answers: false);

    scenario('a player never told its addresses goes straight to Bluetooth', (
      h,
      _,
    ) {
      h.connect();
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.wifi!.attempts, isEmpty);
      // And is told them now, for next time.
      expect(h.store.player!.hosts, [ip]);
      expect(h.store.player!.port, 43234);
      expect(h.store.player!.hostname, 'stage');
    }, player: const RememberedPlayer(id: 'p1', name: 'stage'));

    scenario('each Bluetooth connection refreshes the addresses it remembers', (
      h,
      _,
    ) {
      h.rig.networkInfo = info(addresses: ['10.0.0.9', ip], port: 50051);
      connectAfterWifi(h);
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.store.player!.hosts, ['10.0.0.9', ip]);
      expect(h.store.player!.port, 50051);
      expect(h.store.player!.id, 'p1');
      expect(h.store.player!.bluetooth, isTrue);
    }, answers: false);

    scenario(
      'a player with no gRPC controller leaves nothing to try on Wi-Fi',
      (h, _) {
        h.rig.networkInfo = info(port: null);
        connectAfterWifi(h);
        expect(h.store.player!.hasWifi, isFalse);
      },
      answers: false,
    );

    scenario('the status over Wi-Fi comes from the one watch, and polls '
        'nothing', (h, _) {
      h.connect();
      final before = h.count('Status');
      h.rig.playing = true;
      h.rig.elapsed = const Duration(seconds: 12);
      h.rig.publish();
      h.settle();
      expect(h.session.playing, isTrue);
      h.elapse(const Duration(seconds: 30));
      expect(h.count('Status'), before);
    });

    scenario('a call goes to the player over the network', (h, _) {
      h.connect();
      expect(h.run(h.session.play), isNull);
      expect(h.wifi!.live!.calls, contains('Play'));
      expect(h.link.calls, isEmpty);
    });

    scenario('a refusal over Wi-Fi reads as it does over Bluetooth', (h, _) {
      h.connect();
      h.rig.locked = true;
      h.rig.publish();
      h.settle();
      h.rig.player.handlers['Play'] = (_) =>
          FakeReply.error(GrpcCode.permissionDenied, 'locked');
      expect(
        h.run(h.session.play),
        'The player is locked, or does not allow this from a phone.',
      );
    });
  });

  group('the wrong player', () {
    scenario(
      'another device at the remembered address is not used; Bluetooth is',
      (h, _) {
        h.wifi!.networkInfo = info(hostname: 'somebody-elses-printer');
        h.connect();
        h.elapse(Duration.zero);
        expect(h.session.carrier, Carrier.bluetooth);
        expect(h.session.connectError, isNull);
        expect(h.wifi!.attempts, ['$ip:43234']);
        expect(
          h.wifi!.channels.single.currentState,
          TransportState.disconnected,
          reason: 'the connection to the stranger is closed',
        );
        // What is remembered is still the player.
        expect(h.store.player!.hostname, 'stage');
      },
    );

    scenario('a player that will not say who it is is not trusted once '
        'it has been told', (h, _) {
      h.rig.player.handlers['GetNetworkInfo'] = (_) {
        return FakeReply.error(GrpcCode.unimplemented, '');
      };
      h.wifi!.networkInfo = null;
      h.connect();
      expect(h.session.carrier, Carrier.bluetooth);
    });

    scenario('the background retry does not move over to a stranger', (h, _) {
      h.wifi!.networkInfo = info(hostname: 'stranger');
      h.connect();
      h.elapse(const Duration(minutes: 2));
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.session.phase, ConnectionPhase.ready);
    });

    scenario(
      'a player known only by an address learns its name on first contact, '
      'then holds to it',
      (h, _) {
        h.session.connectToAddress(const WifiAddress(ip, 43234));
        h.settle();
        expect(h.store.player!.hostname, 'stage');
        // The lease moves: the same address answers as another device.
        h.wifi!.networkInfo = info(hostname: 'a-printer');
        h.wifi!.live!.drop();
        h.settle();
        h.elapse(const Duration(minutes: 1));
        expect(h.session.phase, ConnectionPhase.lost);
        expect(h.wifi!.live, isNull, reason: 'no connection is kept');
        expect(h.store.player!.hostname, 'stage');
      },
      player: null,
    );
  });

  group('older players', () {
    scenario(
      'a player without WatchStatus stays on Bluetooth and says nothing',
      (h, _) {
        h.wifi!.refusal = const PlayerException(
          GrpcCode.unimplemented,
          'unknown',
        );
        h.connect();
        expect(h.session.carrier, Carrier.bluetooth);
        expect(h.session.connectError, isNull);
        final notices = <String>[];
        h.session.notices.listen(notices.add);
        // And is not asked again, minute after minute.
        h.elapse(const Duration(minutes: 3));
        expect(h.wifi!.attempts, hasLength(1));
        expect(notices, isEmpty);
        expect(h.session.carrier, Carrier.bluetooth);
      },
    );

    scenario('a player without GetNetworkInfo is used as before', (h, _) {
      h.rig.networkInfo = null;
      h.connect();
      expect(h.session.carrier, Carrier.bluetooth);
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.store.player, const RememberedPlayer(id: 'p1', name: 'stage'));
      h.elapse(const Duration(minutes: 1));
      expect(h.wifi!.attempts, isEmpty);
    }, player: const RememberedPlayer(id: 'p1', name: 'stage'));
  });

  group('moving over', () {
    scenario(
      'a player that starts answering on Wi-Fi is moved to without the person '
      'doing anything',
      (h, async) {
        h.wifi!.reachable.clear();
        h.connect();
        h.elapse(const Duration(seconds: 2));
        expect(h.session.carrier, Carrier.bluetooth);
        h.wifi!.reachable.add(ip);
        h.elapse(wifiRetryInterval);
        expect(h.session.phase, ConnectionPhase.ready);
        expect(h.session.carrier, Carrier.wifi);
        expect(
          h.rig.player.currentState,
          TransportState.disconnected,
          reason: 'Bluetooth is let go',
        );
        // The status is now the watch.
        h.rig.playing = true;
        h.rig.publish();
        h.settle();
        expect(h.session.playing, isTrue);
        // And it does not go on looking.
        final attempts = h.wifi!.attempts.length;
        h.elapse(const Duration(minutes: 2));
        expect(h.wifi!.attempts, hasLength(attempts));
      },
    );

    scenario('it looks again at a modest interval while the player is silent', (
      h,
      _,
    ) {
      h.wifi!.reachable.clear();
      h.connect();
      h.elapse(const Duration(seconds: 2));
      final first = h.wifi!.attempts.length;
      h.elapse(wifiRetryInterval + const Duration(seconds: 2));
      expect(h.wifi!.attempts.length, first + 1);
      h.elapse(wifiRetryInterval);
      expect(h.wifi!.attempts.length, first + 2);
      expect(h.session.carrier, Carrier.bluetooth);
    });

    scenario('it waits for a call in flight before it moves', (h, async) {
      h.wifi!.reachable.clear();
      h.connect();
      h.elapse(const Duration(seconds: 2));
      h.wifi!.reachable.add(ip);
      h.rig.player.handlers['Play'] = (_) =>
          FakeReply.ok(pp.PlayResponse(), delay: const Duration(seconds: 3));
      // The call goes out just before the interval is up.
      h.elapse(wifiRetryInterval - const Duration(seconds: 1));
      String? result;
      var done = false;
      h.session.play().then((value) {
        result = value;
        done = true;
      });
      h.elapse(const Duration(seconds: 2));
      expect(done, isFalse);
      expect(h.session.carrier, Carrier.bluetooth, reason: 'call in flight');
      expect(h.session.phase, ConnectionPhase.ready);
      h.elapse(const Duration(seconds: 6));
      expect(done, isTrue);
      expect(result, isNull, reason: 'the call was not interrupted');
      h.elapse(const Duration(seconds: 1));
      expect(h.session.carrier, Carrier.wifi);
    });

    scenario('it gives up on a move that calls keep from happening, and '
        'moves at the next interval', (h, _) {
      h.wifi!.reachable.clear();
      h.connect();
      h.elapse(wifiConnectTimeout);
      h.wifi!.reachable.add(ip);
      const slow = Duration(milliseconds: 4500);
      final handlers = h.rig.player.handlers;
      handlers['Play'] = (_) => FakeReply.ok(pp.PlayResponse(), delay: slow);
      handlers['Stop'] = (_) => FakeReply.ok(pp.StopResponse(), delay: slow);
      handlers['Next'] = (_) => FakeReply.ok(pp.NextResponse(), delay: slow);
      handlers['Previous'] = (_) =>
          FakeReply.ok(pp.PreviousResponse(), delay: slow);
      // Calls that overlap, one after another, across the interval and the
      // patience that follows it.
      h.elapse(wifiRetryInterval - const Duration(seconds: 1));
      final calls = [
        h.session.play,
        h.session.stop,
        h.session.next,
        h.session.previous,
      ];
      for (final call in calls) {
        call();
        h.elapse(const Duration(seconds: 4));
      }
      expect(h.session.carrier, Carrier.bluetooth);
      expect(
        h.wifi!.channels.first.currentState,
        TransportState.disconnected,
        reason: 'the connection that was never used is closed',
      );
      expect(h.session.phase, ConnectionPhase.ready);
      h.elapse(wifiRetryInterval);
      expect(h.session.carrier, Carrier.wifi);
    });

    scenario('a lost Wi-Fi link is found again over whichever answers', (h, _) {
      h.connect();
      expect(h.session.carrier, Carrier.wifi);
      h.wifi!.reachable.clear();
      h.wifi!.live!.drop();
      h.settle();
      expect(h.session.phase, ConnectionPhase.lost);
      h.elapse(const Duration(seconds: 12));
      expect(h.session.phase, ConnectionPhase.ready);
      expect(h.session.carrier, Carrier.bluetooth);
    });
  });

  group('a player known only by its address', () {
    scenario(
      'connects over Wi-Fi alone, remembers its address and name, and never '
      'touches Bluetooth',
      (h, _) {
        h.session.connectToAddress(const WifiAddress(ip, 43234));
        h.settle();
        expect(h.session.phase, ConnectionPhase.ready);
        expect(h.session.carrier, Carrier.wifi);
        expect(h.link.calls, isEmpty);
        final saved = h.store.player!;
        expect(saved.bluetooth, isFalse);
        expect(saved.name, 'stage', reason: 'the name the player gave');
        expect(saved.hosts, [ip]);
        expect(saved.port, 43234);
        expect(h.session.playerName, 'stage');
      },
      player: null,
    );

    scenario('remembers a name the person typed, ahead of the numbers', (h, _) {
      h.wifi!.reachable.add('mtrack.local');
      h.session.connectToAddress(const WifiAddress('mtrack.local', 43234));
      h.settle();
      expect(h.store.player!.hosts, ['mtrack.local', ip]);
    }, player: null);

    scenario('says plainly when nothing answers, and remembers nothing', (
      h,
      _,
    ) {
      h.session.connectToAddress(const WifiAddress('10.9.9.9', 43234));
      h.elapse(const Duration(seconds: 3));
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(
        h.session.connectError,
        startsWith('Nothing answered at 10.9.9.9.'),
      );
      expect(h.store.player, isNull);
      expect(h.link.calls, isEmpty);
    }, player: null);

    scenario('says when the player is too old for Wi-Fi', (h, _) {
      h.wifi!.refusal = const PlayerException(
        GrpcCode.unimplemented,
        'unknown',
      );
      h.session.connectToAddress(const WifiAddress(ip, 43234));
      h.settle();
      expect(h.session.connectError, contains('too old'));
      expect(h.store.player, isNull);
    }, player: null);

    test('a restart finds it, and an outage is waited out over Wi-Fi only', () {
      fakeAsync((async) {
        const saved = RememberedPlayer(
          id: 'wifi:$ip:43234',
          name: 'stage',
          hosts: [ip],
          port: 43234,
          hostname: 'stage',
          bluetooth: false,
        );
        final h = Harness(async, withWifi: true, player: saved);
        h.rig.networkInfo = info();
        h.wifi!.reachable.add(ip);
        h.connect();
        expect(h.session.carrier, Carrier.wifi);
        h.wifi!.reachable.clear();
        h.wifi!.live!.drop();
        h.settle();
        h.elapse(const Duration(minutes: 1));
        expect(h.session.phase, ConnectionPhase.lost);
        expect(h.link.calls, isEmpty, reason: 'no Bluetooth to fall back to');
        h.wifi!.reachable.add(ip);
        h.elapse(const Duration(seconds: 10));
        expect(h.session.phase, ConnectionPhase.ready);
        h.session.dispose();
        h.settle();
      });
    });

    scenario('forgetting it works like forgetting a Bluetooth player', (h, _) {
      h.session.connectToAddress(const WifiAddress(ip, 43234));
      h.settle();
      h.session.forget();
      h.settle();
      expect(h.store.player, isNull);
      expect(h.session.phase, ConnectionPhase.noPlayer);
      expect(h.session.target, isNull);
      expect(h.wifi!.channels.single.currentState, TransportState.disconnected);
    }, player: null);
  });

  group('the stage', () {
    scenario('asks for the player\'s own rate over Wi-Fi', (h, _) {
      h.connect();
      final sub = h.session.watchStage().listen((_) {});
      h.settle();
      final stream = h.wifi!.live!.opened.single;
      expect(stream.method, 'WatchStage');
      expect(pp.WatchStageRequest.fromBuffer(stream.request).maxRate, 0);
      sub.cancel();
      h.settle();
    });

    scenario('asks for $stageRateBluetooth a second over Bluetooth', (h, _) {
      connectAfterWifi(h);
      final sub = h.session.watchStage().listen((_) {});
      h.settle();
      final asked = pp.WatchStageRequest.fromBuffer(
        h.last('WatchStage').payload,
      );
      expect(asked.maxRate, stageRateBluetooth);
      sub.cancel();
      h.settle();
    }, answers: false);
  });

  group('the carrier', () {
    scenario('is null until connected', (h, _) {
      expect(h.session.carrier, isNull);
      h.connect();
      expect(h.session.carrier, Carrier.wifi);
      h.wifi!.live!.drop();
      h.settle();
      expect(h.session.carrier, isNull);
    });
  });
}
