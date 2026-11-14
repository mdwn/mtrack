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

import '../protocol/player_channel.dart';
import '../protocol/player_client.dart';
import 'ports.dart';
import 'wifi_address.dart';

/// How many connection attempts a sweep has in flight at once.
const int sweepConcurrency = 32;

/// How long one attempt may take to connect.
const Duration sweepAttemptTimeout = Duration(milliseconds: 400);

/// When the sweep stops trying addresses, whatever is left.
const Duration sweepDeadline = Duration(seconds: 8);

/// When the whole scan ends, confirmations included.
const Duration scanDeadline = Duration(seconds: 12);

/// How many addresses that accepted a connection are asked who they are;
/// the rest are not looked at. (A network where everything accepts is not
/// one with players on every address.)
const int maxConfirmations = 16;

/// How many are asked at once.
const int confirmConcurrency = 4;

/// How long a hit has to connect as a player, and then to say its name.
const Duration confirmTimeout = Duration(milliseconds: 1500);

final RegExp _wifiInterface = RegExp(r'^(wlan[0-9]+|en0)$');

/// Whether the interface called [name] is the phone's Wi-Fi: `wlan0`,
/// `wlan1` and so on on Android, `en0` on iOS. An allowlist: mobile data
/// (`rmnet*`, `pdp_ip*`, `ccmni*`), VPNs (`tun*`, `utun*`, `ipsec*`, `wg*`,
/// `tailscale*`), loopback and anything not known are not it.
bool isWifiInterface(String name) => _wifiInterface.hasMatch(name);

/// The four numbers of a dotted IPv4 address, or null when [text] is not
/// one.
List<int>? _octets(String text) {
  final parts = text.split('.');
  if (parts.length != 4) {
    return null;
  }
  final numbers = <int>[];
  for (final part in parts) {
    if (!RegExp(r'^[0-9]{1,3}$').hasMatch(part)) {
      return null;
    }
    final n = int.parse(part);
    if (n > 255) {
      return null;
    }
    numbers.add(n);
  }
  return numbers;
}

/// Whether [address] is an IPv4 address on a private network: 10/8,
/// 172.16/12 or 192.168/16, or link-local (169.254/16). Public addresses,
/// carrier-grade NAT (100.64/10), loopback and anything that is not an IPv4
/// address are not.
bool isScannableAddress(String address) {
  final o = _octets(address);
  if (o == null) {
    return false;
  }
  return o[0] == 10 ||
      (o[0] == 172 && o[1] >= 16 && o[1] <= 31) ||
      (o[0] == 192 && o[1] == 168) ||
      (o[0] == 169 && o[1] == 254);
}

/// The phone's own address to sweep around: the first of [addresses] that is
/// on the Wi-Fi interface ([isWifiInterface]) and private
/// ([isScannableAddress]). Null when there is none, which is to say the phone
/// is not on a network it may scan.
String? scannableWifiAddress(Iterable<LocalAddress> addresses) {
  for (final local in addresses) {
    if (isWifiInterface(local.interface) && isScannableAddress(local.address)) {
      return local.address;
    }
  }
  return null;
}

/// The addresses to sweep for a phone at [phone]: the 254 hosts of
/// `a.b.c.0/24` (`.1` to `.254`) less the phone's own, whatever the real
/// netmask is. Empty when [phone] is not an IPv4 address.
List<String> sweepHosts(String phone) {
  final o = _octets(phone);
  if (o == null) {
    return const [];
  }
  return [
    for (var host = 1; host <= 254; host++)
      if (host != o[3]) '${o[0]}.${o[1]}.${o[2]}.$host',
  ];
}

/// The ports to sweep: the standard one, and [remembered] when it differs.
List<int> sweepPorts(int? remembered) => [
  defaultGrpcPort,
  if (remembered != null && remembered != defaultGrpcPort) remembered,
];

/// One sweep of the hosts of a subnet, and the confirming of what answers.
class NetworkSweep {
  /// A sweep of every host on every port. Each player confirmed is given to
  /// [onFound].
  NetworkSweep({
    required this.probe,
    required this.wifi,
    required this.hosts,
    required this.ports,
    required this.onFound,
    this.concurrency = sweepConcurrency,
    this.attemptTimeout = sweepAttemptTimeout,
    this.deadline = sweepDeadline,
    this.overall = scanDeadline,
  });

  /// How connections are tried.
  final NetworkProbe probe;

  /// How a hit is connected to as a player.
  final WifiLink wifi;

  /// The addresses to try.
  final List<String> hosts;

  /// The ports to try on each.
  final List<int> ports;

  /// Called with each player confirmed: its host name (or its address when
  /// it says none), its address and its port.
  final void Function(FoundPlayer player) onFound;

  /// Attempts in flight at once, at most.
  final int concurrency;

  /// How long an attempt may take.
  final Duration attemptTimeout;

  /// When the sweep stops trying new addresses.
  final Duration deadline;

  /// When everything ends.
  final Duration overall;

  // Completes when the sweep is cancelled: nothing more is reported.
  final Completer<void> _cancelled = Completer<void>();
  // Completes when attempts under way are to be abandoned: on the sweep's
  // deadline, or on a cancel.
  final Completer<void> _abandon = Completer<void>();
  final Completer<void> _finished = Completer<void>();
  final List<({String host, int port})> _hits = [];
  final Set<String> _asked = {};
  Timer? _sweepTimer;
  Timer? _overallTimer;
  bool _sweeping = false;
  bool _sweepEnded = false;
  bool _started = false;
  int _confirming = 0;
  int _confirmed = 0;

  /// Whether the sweep was cancelled or ran out of time.
  bool get stopped => _cancelled.isCompleted;

  /// Runs the sweep. Completes when it is over: everything tried and
  /// confirmed, the time ran out, or [cancel] was called. Runs once.
  Future<void> run() {
    if (_started) {
      return _finished.future;
    }
    _started = true;
    final attempts = [
      for (final host in hosts)
        for (final port in ports) (host: host, port: port),
    ];
    _sweeping = true;
    _sweepTimer = Timer(deadline, _stopTrying);
    _overallTimer = Timer(overall, cancel);
    var next = 0;
    Future<void> worker() async {
      while (!stopped && !_sweepEnded && next < attempts.length) {
        final attempt = attempts[next++];
        final open = await probe.canConnect(
          attempt.host,
          attempt.port,
          timeout: attemptTimeout,
          cancelled: _abandon.future,
        );
        if (open && !stopped && !_sweepEnded) {
          _hits.add(attempt);
          _confirmNext();
        }
      }
    }

    Future.wait([for (var i = 0; i < concurrency; i++) worker()])
        .whenComplete(() {
          _sweeping = false;
          _sweepTimer?.cancel();
          _sweepTimer = null;
          _maybeFinish();
        });
    return _finished.future;
  }

  // The time for trying addresses is up: what is in flight is abandoned,
  // what accepted may still be confirmed.
  void _stopTrying() {
    _sweepEnded = true;
    if (!_abandon.isCompleted) {
      _abandon.complete();
    }
  }

  /// Ends the sweep now: attempts are abandoned with their sockets closed,
  /// and nothing more is confirmed or reported.
  void cancel() {
    if (!_cancelled.isCompleted) {
      _cancelled.complete();
    }
    _stopTrying();
    _hits.clear();
    _complete();
  }

  void _confirmNext() {
    while (!stopped &&
        _confirming < confirmConcurrency &&
        _hits.isNotEmpty &&
        _confirmed < maxConfirmations) {
      final hit = _hits.removeAt(0);
      if (!_asked.add('${hit.host}:${hit.port}')) {
        continue;
      }
      _confirmed++;
      _confirming++;
      _confirm(hit.host, hit.port).whenComplete(() {
        _confirming--;
        _confirmNext();
        _maybeFinish();
      });
    }
  }

  // Connects to the address as a player and asks its name. Anything that is
  // not a player is dropped without a word.
  Future<void> _confirm(String host, int port) async {
    PlayerChannel? channel;
    try {
      channel = await wifi.connect(host, port, timeout: confirmTimeout);
      if (stopped) {
        return;
      }
      final info = await PlayerClient(channel)
          .getNetworkInfo(timeout: confirmTimeout);
      if (stopped) {
        return;
      }
      onFound(
        FoundPlayer(
          name: info.hostname.isNotEmpty ? info.hostname : host,
          hosts: [host],
          port: port,
        ),
      );
    } on Object {
      // Not a player, or one too old, or one that will not say.
    } finally {
      try {
        await channel?.dispose();
      } on Object {
        // Nothing more to do with it.
      }
    }
  }

  void _maybeFinish() {
    if (!_sweeping &&
        _confirming == 0 &&
        (_hits.isEmpty || _confirmed >= maxConfirmations)) {
      _complete();
    }
  }

  void _complete() {
    _sweepTimer?.cancel();
    _overallTimer?.cancel();
    if (!_finished.isCompleted) {
      _finished.complete();
    }
  }
}
