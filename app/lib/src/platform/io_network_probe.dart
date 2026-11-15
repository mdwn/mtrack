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
import 'dart:io';

import '../app/ports.dart';
import 'interface_prefixes.dart';

/// Looks at the phone's own interfaces and tries TCP connections with
/// `dart:io`, with the netmasks from the C library (`interface_prefixes`).
/// Needs no permission beyond the network access the app has.
class IoNetworkProbe implements NetworkProbe {
  /// A probe over `dart:io`. [prefixes] reads the netmasks, by
  /// [prefixKey]; it is [interfacePrefixes] unless a test gives another.
  IoNetworkProbe({Map<String, int> Function()? prefixes})
    : _prefixes = prefixes ?? interfacePrefixes;

  final Map<String, int> Function() _prefixes;

  // Failed attempts by kind since the last [takeDiagnostics].
  int _timeouts = 0;
  int _other = 0;
  final Map<int, int> _errnos = {};

  @override
  Future<List<LocalAddress>> localAddresses() async {
    final interfaces = await NetworkInterface.list(
      type: InternetAddressType.IPv4,
      includeLinkLocal: true,
    );
    Map<String, int> prefixes;
    try {
      prefixes = _prefixes();
    } on Object {
      prefixes = const {};
    }
    return [
      for (final interface in interfaces)
        for (final address in interface.addresses)
          LocalAddress(
            interface.name,
            address.address,
            prefixLength: _known(
              prefixes[prefixKey(interface.name, address.address)],
            ),
          ),
    ];
  }

  // A prefix of 0 is a mask that said nothing, not a network of everything.
  int? _known(int? prefix) => prefix != null && prefix > 0 ? prefix : null;

  @override
  String takeDiagnostics() {
    final parts = [
      if (_timeouts > 0) 'timeout=$_timeouts',
      for (final code in _errnos.keys.toList()..sort())
        'errno$code=${_errnos[code]}',
      if (_other > 0) 'other=$_other',
    ];
    _timeouts = 0;
    _other = 0;
    _errnos.clear();
    return parts.join(' ');
  }

  @override
  Future<bool> canConnect(
    String host,
    int port, {
    required Duration timeout,
    required Future<void> cancelled,
  }) async {
    var abandoned = false;
    try {
      final started = await Socket.startConnect(host, port);
      var over = false;
      unawaited(
        cancelled.then((_) {
          if (!over) {
            abandoned = true;
            started.cancel();
          }
        }),
      );
      try {
        final socket = await started.socket.timeout(timeout);
        socket.destroy();
        return true;
      } finally {
        over = true;
        // A connection that timed out is still being made: stop it.
        started.cancel();
      }
    } on TimeoutException {
      if (!abandoned) {
        _timeouts++;
      }
      return false;
    } on SocketException catch (e) {
      if (!abandoned) {
        final code = e.osError?.errorCode;
        if (code == null) {
          _other++;
        } else {
          _errnos[code] = (_errnos[code] ?? 0) + 1;
        }
      }
      return false;
    } on Object {
      if (!abandoned) {
        _other++;
      }
      return false;
    }
  }
}
