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

/// Looks at the phone's own interfaces and tries TCP connections with
/// `dart:io`. Needs no permission beyond the network access the app has.
class IoNetworkProbe implements NetworkProbe {
  /// A probe over `dart:io`.
  const IoNetworkProbe();

  @override
  Future<List<LocalAddress>> localAddresses() async {
    final interfaces = await NetworkInterface.list(
      type: InternetAddressType.IPv4,
      includeLinkLocal: true,
    );
    return [
      for (final interface in interfaces)
        for (final address in interface.addresses)
          LocalAddress(interface.name, address.address),
    ];
  }

  @override
  Future<bool> canConnect(
    String host,
    int port, {
    required Duration timeout,
    required Future<void> cancelled,
  }) async {
    ConnectionTask<Socket>? task;
    try {
      task = await Socket.startConnect(host, port);
      final started = task;
      var over = false;
      unawaited(
        cancelled.then((_) {
          if (!over) {
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
    } on Object {
      return false;
    }
  }
}
