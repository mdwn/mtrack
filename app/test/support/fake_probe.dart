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

import 'package:mtrack_remote/src/app/ports.dart';

/// A [NetworkProbe] over a made-up network: the phone has [addresses], and
/// the `host:port` pairs in [open] accept a connection. Every other attempt
/// is silent until its timeout, as a quiet address is.
class FakeProbe implements NetworkProbe {
  /// What the phone's interfaces are.
  List<LocalAddress> addresses = const [LocalAddress('wlan0', '192.168.1.20')];

  /// When set, listing the interfaces fails with this.
  Object? throwOnList;

  /// The `host:port` pairs that accept a connection.
  final Set<String> open = {};

  /// How long an attempt on an open pair takes.
  Duration openAfter = const Duration(milliseconds: 5);

  /// How many times the interfaces were listed.
  int listings = 0;

  /// Every attempt started, as `host:port`, in order.
  final List<String> attempts = [];

  /// The timeout each attempt was given.
  final List<Duration> timeouts = [];

  /// Attempts under way now, and the most there ever were at once.
  int inFlight = 0;
  int maxInFlight = 0;

  /// How many attempts were abandoned before they ended.
  int abandoned = 0;

  /// How many attempts ended in any way.
  int finished = 0;

  @override
  Future<List<LocalAddress>> localAddresses() async {
    listings++;
    final thrown = throwOnList;
    if (thrown != null) {
      throw thrown;
    }
    return addresses;
  }

  @override
  Future<bool> canConnect(
    String host,
    int port, {
    required Duration timeout,
    required Future<void> cancelled,
  }) {
    attempts.add('$host:$port');
    timeouts.add(timeout);
    inFlight++;
    if (inFlight > maxInFlight) {
      maxInFlight = inFlight;
    }
    final result = Completer<bool>();
    final isOpen = open.contains('$host:$port');
    final timer = Timer(isOpen ? openAfter : timeout, () {
      if (!result.isCompleted) {
        result.complete(isOpen && openAfter <= timeout);
      }
    });
    cancelled.then((_) {
      if (!result.isCompleted) {
        abandoned++;
        timer.cancel();
        result.complete(false);
      }
    });
    return result.future.whenComplete(() {
      inFlight--;
      finished++;
    });
  }
}
