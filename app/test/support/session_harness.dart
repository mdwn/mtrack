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
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/app/session.dart';

import 'fake_browser.dart';
import 'fake_link.dart';
import 'fake_player.dart';
import 'fake_wifi.dart';
import 'rig.dart';

const RememberedPlayer remembered = RememberedPlayer(id: 'p1', name: 'stage');

/// A session over a fake rig, driven by fake time.
class Harness {
  Harness(
    this.async, {
    bool known = true,
    Rig? rig,
    bool withWifi = false,
    bool withBrowser = false,
    RememberedPlayer? player,
  }) : rig = rig ?? Rig() {
    store = MemoryStore(known ? (player ?? remembered) : null);
    wifi = withWifi ? FakeWifi(this.rig) : null;
    browser = withBrowser ? FakeBrowser() : null;
    session = Session(
      link: this.rig.link,
      store: store,
      wake: wake,
      wifi: wifi,
      browser: browser,
      monotonic: () => async.elapsed,
    );
    this.rig.link.paired.add('p1');
  }

  final FakeAsync async;
  final Rig rig;
  late final MemoryStore store;
  final FakeWake wake = FakeWake();
  late final Session session;

  /// The network, when the harness was made with one.
  late final FakeWifi? wifi;

  /// The browser for players on the network, when the harness was made with
  /// one.
  late final FakeBrowser? browser;

  FakeLink get link => rig.link;
  FakePlayer get player => rig.player;

  void settle() => async.flushMicrotasks();

  void elapse(Duration d) {
    async.elapse(d);
    async.flushMicrotasks();
  }

  /// Starts the session and lets the connection finish.
  void connect() {
    session.start();
    settle();
  }

  List<String> methods() => player.received.map((r) => r.method).toList();

  ble.Request last(String method) =>
      player.received.lastWhere((r) => r.method == method);

  int count(String method) => methods().where((m) => m == method).length;

  /// Runs [action] and returns what it came to, once it has.
  String? run(Future<String?> Function() action, {Duration? after}) {
    String? result;
    var done = false;
    action().then((value) {
      result = value;
      done = true;
    });
    if (after != null) {
      elapse(after);
    } else {
      settle();
    }
    expect(done, isTrue, reason: 'the action did not finish');
    return result;
  }
}
