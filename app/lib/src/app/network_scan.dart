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
import 'dart:math' as math;

import '../protocol/player_channel.dart';
import '../protocol/player_client.dart';
import 'ports.dart';
import 'wifi_address.dart';

/// How many connection attempts a sweep has in flight at once, at most. It
/// is no longer what sets the pace ([sweepAddressesPerSecond] is); it only
/// has to be big enough that the limiter is: 256 addresses a second, each
/// held up to [sweepAttemptTimeout] (1 s), on up to two ports, is 512.
const int sweepConcurrency = 512;

/// How many new addresses a sweep starts a second outside the phone's own
/// /24 block. The phone's kernel keeps at most 1,024 neighbour (ARP)
/// entries, and an address that never answers holds its entry for about
/// 2.3 s (three asks, 752 ms apart). Measured on a Pixel on a real /16
/// (2026-11-15): sampled during a sweep, the table sat pinned at 1,024,
/// nearly all of it unanswered. Above roughly 450 new addresses a second the
/// table fills and attempts fail silently as timeouts, machines that are
/// there included: swept farthest-first at a faster pace, a player was
/// missed. 256 a second stays well under that.
const int sweepAddressesPerSecond = 256;

/// How long one attempt may take to connect, on any address. With the pace
/// set by [sweepAddressesPerSecond], a longer wait costs no throughput. The
/// kernel asks again for an address about every 750 ms, so a second gives a
/// machine that missed the first ask a second chance.
const Duration sweepAttemptTimeout = Duration(seconds: 1);

/// When the sweep gives up trying addresses, whatever is left. A safety
/// limit: a /16 (65,533 addresses at [sweepAddressesPerSecond]) takes about
/// 4 min 16 s, well inside it.
const Duration sweepMaxDuration = Duration(minutes: 10);

// How often the pace limiter adds to its tokens: eight times a second, so
// the addresses a second are handed out in eighths.
const Duration _paceRefill = Duration(milliseconds: 125);

/// The shortest prefix swept: a /16, 65,536 addresses. A bigger network is
/// swept only in the /16 around the phone.
const int sweepWidestPrefix = 16;

/// The least a progress callback is called, so the screen is not redrawn for
/// every address.
const Duration sweepProgressInterval = Duration(milliseconds: 100);

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
LocalAddress? scannableWifiLocal(Iterable<LocalAddress> addresses) {
  for (final local in addresses) {
    if (isWifiInterface(local.interface) && isScannableAddress(local.address)) {
      return local;
    }
  }
  return null;
}

/// Just the address of [scannableWifiLocal].
String? scannableWifiAddress(Iterable<LocalAddress> addresses) =>
    scannableWifiLocal(addresses)?.address;

/// The addresses to sweep for a phone: its network, as far as it is known,
/// less the network and broadcast addresses and the phone itself.
///
/// The network is the phone's address under its prefix length, a /24 when
/// that is not known, and no wider than a /16 (when it is, [clamped] says so
/// and the /16 around the phone is used). The order is the phone's own /24
/// block first, then the blocks next to it, outward by distance (the one
/// below, the one above, two below, ...), each block upward. [hosts] is made
/// as it is read: a /16 is never held as a list.
class SweepPlan {
  SweepPlan._({
    required this.prefix,
    required this.clamped,
    required this.total,
    required this.ownBlockCount,
    required this._phone,
    required this._low,
    required this._high,
  });

  /// A plan for a phone at [phone] (dotted IPv4) on a network with
  /// [prefixLength], null when not known. Empty for anything that is not an
  /// IPv4 address.
  factory SweepPlan(String phone, int? prefixLength) {
    final o = _octets(phone);
    if (o == null) {
      return SweepPlan._empty();
    }
    final address = (o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3];
    var prefix = prefixLength ?? 24;
    if (prefix < 0 || prefix > 32) {
      prefix = 24;
    }
    var clamped = false;
    if (prefix < sweepWidestPrefix) {
      prefix = sweepWidestPrefix;
      clamped = true;
    }
    final size = 1 << (32 - prefix);
    final base = address & ~(size - 1) & 0xFFFFFFFF;
    // The hosts: all but the network and broadcast addresses. A /31 has two
    // and no such addresses; a /32 has none beyond the phone.
    final int low;
    final int high;
    if (prefix >= 32) {
      low = 1;
      high = 0;
    } else if (prefix == 31) {
      low = base;
      high = base + 1;
    } else {
      low = base + 1;
      high = base + size - 2;
    }
    final phoneIn = address >= low && address <= high;
    final total = high < low ? 0 : high - low + 1 - (phoneIn ? 1 : 0);
    final ownLow = math.max(low, address & ~0xFF & 0xFFFFFFFF);
    final ownHigh = math.min(high, (address | 0xFF) & 0xFFFFFFFF);
    final own = ownHigh < ownLow ? 0 : ownHigh - ownLow + 1 - (phoneIn ? 1 : 0);
    return SweepPlan._(
      prefix: prefix,
      clamped: clamped,
      total: total,
      ownBlockCount: own,
      phone: address,
      low: low,
      high: high,
    );
  }

  SweepPlan._empty()
    : prefix = 32,
      clamped = false,
      total = 0,
      ownBlockCount = 0,
      _phone = 0,
      _low = 1,
      _high = 0;

  /// The prefix length swept: the one given, or 24, or 16 when clamped.
  final int prefix;

  /// Whether the network is bigger than a /16, so only the /16 around the
  /// phone is swept.
  final bool clamped;

  /// How many addresses there are to try.
  final int total;

  /// How many of them come first as the phone's own /24 block: the first
  /// [ownBlockCount] of [hosts].
  final int ownBlockCount;

  final int _phone;
  final int _low;
  final int _high;

  /// The addresses, in order, made as they are read.
  Iterable<String> get hosts sync* {
    if (_high < _low) {
      return;
    }
    final own = _phone >> 8;
    final firstBlock = _low >> 8;
    final lastBlock = _high >> 8;
    for (var d = 0; own - d >= firstBlock || own + d <= lastBlock; d++) {
      for (final block in d == 0 ? [own] : [own - d, own + d]) {
        if (block < firstBlock || block > lastBlock) {
          continue;
        }
        final from = math.max(_low, block << 8);
        final to = math.min(_high, (block << 8) | 0xFF);
        for (var a = from; a <= to; a++) {
          if (a != _phone) {
            yield _dotted(a);
          }
        }
      }
    }
  }
}

String _dotted(int a) =>
    '${(a >> 24) & 255}.${(a >> 16) & 255}.${(a >> 8) & 255}.${a & 255}';

/// The ports to sweep: the standard one, and [remembered] when it differs.
List<int> sweepPorts(int? remembered) => [
  defaultGrpcPort,
  if (remembered != null && remembered != defaultGrpcPort) remembered,
];

/// Why a sweep ended.
enum SweepEnd {
  /// Every address was tried.
  done,

  /// It was stopped: by the person, or because a player was chosen.
  stopped,

  /// [sweepMaxDuration] ran out first.
  timeLimit,
}

/// How a sweep went.
class SweepSummary {
  /// A summary of a sweep that [end]ed after [elapsed], having tried [tried]
  /// of [total] addresses, [accepted] of which (address and port) took a
  /// connection and [confirmed] of which proved to be players.
  const SweepSummary({
    required this.tried,
    required this.total,
    required this.accepted,
    required this.confirmed,
    required this.elapsed,
    required this.end,
  });

  /// Addresses tried to the end on every port.
  final int tried;

  /// Addresses there were to try.
  final int total;

  /// Connections accepted, counted by address and port.
  final int accepted;

  /// Players confirmed.
  final int confirmed;

  /// How long it took.
  final Duration elapsed;

  /// Why it ended.
  final SweepEnd end;
}

/// One sweep of the hosts of a [SweepPlan], and the confirming of what
/// answers.
class NetworkSweep {
  /// A sweep of every host on every port. Each player confirmed is given to
  /// [onFound]; how far it has got to [onProgress], at most once every
  /// [progressInterval].
  NetworkSweep({
    required this.probe,
    required this.wifi,
    required this.plan,
    required this.ports,
    required this.onFound,
    this.onProgress,
    this.concurrency = sweepConcurrency,
    this.addressesPerSecond = sweepAddressesPerSecond,
    this.attemptTimeout = sweepAttemptTimeout,
    this.maxDuration = sweepMaxDuration,
    this.progressInterval = sweepProgressInterval,
    Duration Function()? elapsed,
  }) : _elapsed = elapsed ?? _stopwatch();

  /// How connections are tried.
  final NetworkProbe probe;

  /// How a hit is connected to as a player.
  final WifiLink wifi;

  /// The addresses to try, and in what order.
  final SweepPlan plan;

  /// The ports to try on each.
  final List<int> ports;

  /// Called with each player confirmed: its host name (or its address when
  /// it says none), its address and its port.
  final void Function(FoundPlayer player) onFound;

  /// Called with the addresses tried so far and the total, when that has
  /// changed. Not after the sweep is cancelled.
  final void Function(int tried, int total)? onProgress;

  /// Attempts in flight at once, at most.
  final int concurrency;

  /// New addresses started a second outside the phone's own /24 block.
  final int addressesPerSecond;

  /// How long an attempt may take.
  final Duration attemptTimeout;

  /// When the sweep stops trying new addresses.
  final Duration maxDuration;

  /// The least between two calls of [onProgress].
  final Duration progressInterval;

  final Duration Function() _elapsed;

  // Completes when the sweep is cancelled: nothing more is reported.
  final Completer<void> _cancelled = Completer<void>();
  // Completes when attempts under way are to be abandoned: on the time
  // limit, or on a cancel.
  final Completer<void> _abandon = Completer<void>();
  final Completer<SweepSummary> _finished = Completer<SweepSummary>();
  final List<({String host, int port})> _hits = [];
  final Set<String> _asked = {};
  Timer? _limitTimer;
  Timer? _progressTimer;
  // The pace limiter: tokens to start a new address, added by a periodic
  // timer (no clock is read), and the workers waiting for one.
  Timer? _paceTimer;
  int _tokens = 0;
  late final int _tokensPerRefill = math.max(
    1,
    addressesPerSecond * _paceRefill.inMilliseconds ~/ 1000,
  );
  final List<Completer<void>> _tokenWaiters = [];
  // Attempts are handed out one at a time, in order; see [_take].
  Future<void> _turn = Future<void>.value();
  bool _sweeping = false;
  bool _sweepEnded = false;
  bool _timedOut = false;
  bool _started = false;
  int _confirming = 0;
  int _confirmed = 0;
  int _found = 0;
  int _accepted = 0;
  int _attemptsDone = 0;
  int _reported = -1;

  /// Whether the sweep was cancelled.
  bool get stopped => _cancelled.isCompleted;

  // Addresses whose every port has been tried to the end.
  int get _tried => math.min(plan.total, _attemptsDone ~/ ports.length);

  /// Runs the sweep. Completes with how it went when it is over: everything
  /// tried and confirmed, the time limit ran out, or [cancel] was called.
  /// Runs once.
  Future<SweepSummary> run() {
    if (_started) {
      return _finished.future;
    }
    _started = true;
    if (ports.isEmpty || plan.total == 0) {
      _sweeping = false;
      _report();
      _complete();
      return _finished.future;
    }
    final attempts = _attempts().iterator;
    _sweeping = true;
    _limitTimer = Timer(maxDuration, () {
      _timedOut = true;
      _stopTrying();
    });
    _report();
    _progressTimer = Timer.periodic(progressInterval, (_) => _report());
    if (plan.total > plan.ownBlockCount) {
      _tokens = _tokensPerRefill;
      _paceTimer = Timer.periodic(_paceRefill, (_) => _refill());
    }
    Future<void> worker() async {
      while (!stopped && !_sweepEnded) {
        final started = await _start(attempts);
        if (started == null) {
          break;
        }
        final attempt = started.attempt;
        final open = await started.open;
        if (stopped || _sweepEnded) {
          break;
        }
        _attemptsDone++;
        if (open) {
          _accepted++;
          _hits.add((host: attempt.host, port: attempt.port));
          _confirmNext();
        }
      }
    }

    Future.wait([for (var i = 0; i < concurrency; i++) worker()])
        .whenComplete(() {
          _sweeping = false;
          _limitTimer?.cancel();
          _limitTimer = null;
          _progressTimer?.cancel();
          _progressTimer = null;
          _stopPacing();
          _report();
          _maybeFinish();
        });
    return _finished.future;
  }

  // Every host in turn with all its ports. An attempt is [paced] when it is
  // the first port of a host outside the phone's own block: starting it
  // needs a token. The other ports of the host ride on that one.
  Iterable<({String host, int port, bool paced})> _attempts() sync* {
    var index = 0;
    for (final host in plan.hosts) {
      final own = index++ < plan.ownBlockCount;
      var first = true;
      for (final port in ports) {
        yield (host: host, port: port, paced: !own && first);
        first = false;
      }
    }
  }

  // Starts the next attempt, or gives null when there are none or the sweep
  // is over. The starting is done one at a time, in order, so that a host
  // waiting for its token holds back the attempts after it, its own other
  // ports included.
  Future<({({String host, int port, bool paced}) attempt, Future<bool> open})?>
  _start(Iterator<({String host, int port, bool paced})> attempts) {
    final next = _turn.then((_) async {
      if (stopped || _sweepEnded || !attempts.moveNext()) {
        return null;
      }
      final attempt = attempts.current;
      if (attempt.paced) {
        await _token();
        if (stopped || _sweepEnded) {
          return null;
        }
      }
      final open = probe.canConnect(
        attempt.host,
        attempt.port,
        timeout: attemptTimeout,
        cancelled: _abandon.future,
      );
      return (attempt: attempt, open: open);
    });
    _turn = next.then((_) {});
    return next;
  }

  // Completes when a token is to hand, and takes it. Also completes, with
  // none taken, when pacing is over: the caller looks at whether the sweep
  // ended.
  Future<void> _token() {
    if (_sweepEnded) {
      return Future<void>.value();
    }
    if (_tokens > 0 && _tokenWaiters.isEmpty) {
      _tokens--;
      return Future<void>.value();
    }
    final waiter = Completer<void>();
    _tokenWaiters.add(waiter);
    return waiter.future;
  }

  // One refill of the bucket. Tokens not spent are capped at one refill, so
  // a lull is not followed by a burst.
  void _refill() {
    _tokens = math.min(_tokensPerRefill, _tokens + _tokensPerRefill);
    while (_tokens > 0 && _tokenWaiters.isNotEmpty) {
      _tokens--;
      _tokenWaiters.removeAt(0).complete();
    }
  }

  // Pacing is over: the timer is cancelled and every waiter let go.
  void _stopPacing() {
    _paceTimer?.cancel();
    _paceTimer = null;
    final waiters = List.of(_tokenWaiters);
    _tokenWaiters.clear();
    for (final waiter in waiters) {
      waiter.complete();
    }
  }

  void _report() {
    if (stopped) {
      return;
    }
    final tried = _tried;
    if (tried != _reported) {
      _reported = tried;
      onProgress?.call(tried, plan.total);
    }
  }

  // The time for trying addresses is up: what is in flight is abandoned,
  // what accepted may still be confirmed.
  void _stopTrying() {
    _sweepEnded = true;
    if (!_abandon.isCompleted) {
      _abandon.complete();
    }
    _stopPacing();
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
      _found++;
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
    _limitTimer?.cancel();
    _progressTimer?.cancel();
    _progressTimer = null;
    _stopPacing();
    if (!_finished.isCompleted) {
      final SweepEnd end;
      if (stopped) {
        end = SweepEnd.stopped;
      } else if (_timedOut) {
        end = SweepEnd.timeLimit;
      } else {
        end = SweepEnd.done;
      }
      _finished.complete(
        SweepSummary(
          tried: _tried,
          total: plan.total,
          accepted: _accepted,
          confirmed: _found,
          elapsed: _elapsed(),
          end: end,
        ),
      );
    }
  }
}

Duration Function() _stopwatch() {
  final watch = Stopwatch()..start();
  return () => watch.elapsed;
}
