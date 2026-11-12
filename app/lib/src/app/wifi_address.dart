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
import 'dart:io';

/// The port a player's gRPC controller listens on unless its config says
/// otherwise.
const int defaultGrpcPort = 43234;

/// Where to reach a player over Wi-Fi: a host name or IP and a port.
class WifiAddress {
  /// An address on [host] and [port].
  const WifiAddress(this.host, this.port);

  /// A host name (`mtrack.local`) or an IP address.
  final String host;

  /// The gRPC port.
  final int port;

  @override
  String toString() {
    final shown = host.contains(':') ? '[$host]' : host;
    return port == defaultGrpcPort ? shown : '$shown:$port';
  }

  @override
  bool operator ==(Object other) =>
      other is WifiAddress && other.host == host && other.port == port;

  @override
  int get hashCode => Object.hash(host, port);
}

final RegExp _label = RegExp(r'^[A-Za-z0-9_]([A-Za-z0-9_-]*[A-Za-z0-9_])?$');
final RegExp _digits = RegExp(r'^[0-9]+$');
final RegExp _numericHost = RegExp(r'^[0-9.]+$');

/// Reads what a person typed as the player's address: a host name or an IP,
/// with an optional `:port` after it (43234 when left off), such as
/// `mtrack.local`, `192.168.1.40` or `192.168.1.40:50051`. An IPv6 address is
/// written in brackets when it has a port.
///
/// Throws a [FormatException] whose message says what is wrong, in words for
/// a person.
WifiAddress parseWifiAddress(String text) {
  final typed = text.trim();
  if (typed.isEmpty) {
    throw const FormatException(
      "Type the player's address, such as mtrack.local.",
    );
  }
  if (typed.contains('/') || typed.contains(RegExp(r'\s'))) {
    throw const FormatException(
      'Type just the name or number, such as mtrack.local or 192.168.1.40, '
      'not a web address.',
    );
  }
  String host;
  String? portText;
  if (typed.startsWith('[')) {
    final close = typed.indexOf(']');
    if (close < 0) {
      throw const FormatException('That address is missing a closing ].');
    }
    host = typed.substring(1, close);
    final rest = typed.substring(close + 1);
    if (rest.isNotEmpty) {
      if (!rest.startsWith(':')) {
        throw const FormatException('That address is not one I can read.');
      }
      portText = rest.substring(1);
    }
  } else if (':'.allMatches(typed).length > 1) {
    host = typed;
  } else if (typed.contains(':')) {
    final at = typed.indexOf(':');
    host = typed.substring(0, at);
    portText = typed.substring(at + 1);
  } else {
    host = typed;
  }
  var port = defaultGrpcPort;
  if (portText != null) {
    final value = _digits.hasMatch(portText) ? int.tryParse(portText) : null;
    if (value == null || value < 1 || value > 65535) {
      throw const FormatException(
        'The port is a number from 1 to 65535. Leave it off to use 43234.',
      );
    }
    port = value;
  }
  if (host.isEmpty) {
    throw const FormatException(
      "Type the player's address, such as mtrack.local.",
    );
  }
  if (host.contains(':')) {
    if (InternetAddress.tryParse(host)?.type != InternetAddressType.IPv6) {
      throw const FormatException('That is not a valid IPv6 address.');
    }
    return WifiAddress(host, port);
  }
  if (_numericHost.hasMatch(host)) {
    final parts = host.split('.');
    final valid =
        parts.length == 4 &&
        parts.every((p) {
          final n = _digits.hasMatch(p) ? int.tryParse(p) : null;
          return n != null && n <= 255;
        });
    if (!valid) {
      throw const FormatException(
        'That is not a valid number. It should look like 192.168.1.40.',
      );
    }
    return WifiAddress(host, port);
  }
  final labels = host.split('.');
  if (host.length > 253 ||
      !labels.every((l) => l.length <= 63 && _label.hasMatch(l))) {
    throw const FormatException(
      'A name has only letters, numbers and hyphens, with dots between '
      'its parts.',
    );
  }
  return WifiAddress(host, port);
}
