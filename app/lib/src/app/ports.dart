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

import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/player_channel.dart';
import 'wifi_address.dart';

/// A player seen on the network by its DNS-SD advert (`_mtrack._tcp`).
class FoundPlayer {
  /// A player advertised as [name], reachable at [hosts] on [port].
  const FoundPlayer({
    required this.name,
    required this.hosts,
    required this.port,
    this.version,
  });

  /// The service's instance name: the player's host name. This is what to
  /// show. (The service's own host label is an implementation detail of the
  /// advert and is never kept.)
  final String name;

  /// The addresses it answers on, as strings, in the order the platform gave
  /// them: IPv4 and IPv6 mixed. See [orderedHosts].
  final List<String> hosts;

  /// The gRPC port.
  final int port;

  /// The mtrack version it reports (the `v` TXT record), or null.
  final String? version;

  /// [hosts] in the order to try them: IPv4 first, then IPv6, with the
  /// link-local IPv6 addresses (which cannot be reached without knowing the
  /// interface) last. The order inside each group is kept.
  List<String> get orderedHosts {
    int rank(String host) {
      final parsed = InternetAddress.tryParse(host);
      if (parsed == null) {
        return 1;
      }
      if (parsed.type == InternetAddressType.IPv4) {
        return 0;
      }
      return parsed.isLinkLocal ? 3 : 2;
    }

    return [
      for (var group = 0; group <= 3; group++)
        for (final host in hosts)
          if (rank(host) == group) host,
    ];
  }

  @override
  bool operator ==(Object other) =>
      other is FoundPlayer &&
      other.name == name &&
      other.port == port &&
      other.version == version &&
      other.hosts.length == hosts.length &&
      [for (var i = 0; i < hosts.length; i++) other.hosts[i] == hosts[i]]
          .every((same) => same);

  @override
  int get hashCode => Object.hash(name, port, version, Object.hashAll(hosts));
}

/// A player the app will reconnect to: one it has paired with over
/// Bluetooth, or one it was given an address for, or both.
class RememberedPlayer {
  /// A player with the platform's [id] and the [name] it advertised.
  ///
  /// [hosts] and [port] are where it was last reachable over Wi-Fi, and
  /// [hostname] the name it gave itself there (what tells it from another
  /// device that comes to have the same address). [bluetooth] is false for a
  /// player known only by its address, which is reached over Wi-Fi alone.
  /// [mdnsName] is the DNS-SD instance name it was found under, if it was
  /// found that way: what to look for again when its addresses stop working.
  const RememberedPlayer({
    required this.id,
    required this.name,
    this.hosts = const [],
    this.port,
    this.hostname,
    this.bluetooth = true,
    this.mdnsName,
  });

  /// A player found on the network as [found], reached over Wi-Fi alone and
  /// remembered under its instance name.
  RememberedPlayer.byService(FoundPlayer found)
    : id = 'mdns:${found.name}',
      name = found.name,
      hosts = found.orderedHosts.take(maxHosts).toList(),
      port = found.port,
      hostname = null,
      bluetooth = false,
      mdnsName = found.name;

  /// A player known only by [address]; [name] is what to call it.
  RememberedPlayer.byAddress(WifiAddress address, {String? name})
    : id = 'wifi:${address.host}:${address.port}',
      name = name ?? address.host,
      hosts = [address.host],
      port = address.port,
      hostname = null,
      bluetooth = false,
      mdnsName = null;

  /// The platform's identifier for the peripheral (a made-up one that starts
  /// `wifi:` for a player with no Bluetooth).
  final String id;

  /// The player's name, as it was when last seen.
  final String name;

  /// The addresses to try over Wi-Fi, in order: host names and IPs.
  final List<String> hosts;

  /// The player's gRPC port, or null when it has none to tell.
  final int? port;

  /// The host name the player reported over Wi-Fi or Bluetooth, if it ever
  /// did.
  final String? hostname;

  /// Whether the player can be reached over Bluetooth.
  final bool bluetooth;

  /// The DNS-SD instance name the player was found under on the network, or
  /// null when it was not found that way (a typed address, or Bluetooth).
  final String? mdnsName;

  /// Whether there is anywhere to try over Wi-Fi.
  bool get hasWifi => hosts.isNotEmpty && port != null;

  /// This player with the given parts changed.
  RememberedPlayer copyWith({
    String? name,
    List<String>? hosts,
    int? port,
    bool clearPort = false,
    String? hostname,
  }) => RememberedPlayer(
    id: id,
    name: name ?? this.name,
    hosts: hosts ?? this.hosts,
    port: clearPort ? null : (port ?? this.port),
    hostname: hostname ?? this.hostname,
    bluetooth: bluetooth,
    mdnsName: mdnsName,
  );

  /// The most addresses kept for a player.
  static const int maxHosts = 6;

  /// This player as the player's own account of where it is on the network
  /// ([info]) has it. [connectedHost] is the address that was just reached
  /// over Wi-Fi, if one was: it goes first. Addresses the person typed as
  /// names stay, since a name outlives the address it resolves to; the
  /// numbers the player reported replace the old ones.
  ///
  /// A player that reports no gRPC port has nothing to try over Wi-Fi.
  RememberedPlayer learned(
    player.GetNetworkInfoResponse info,
    String? connectedHost,
  ) {
    final reported = info.hasGrpcPort() ? info.grpcPort : null;
    final newPort = reported ?? (connectedHost != null ? port : null);
    final newHostname = info.hostname.isNotEmpty ? info.hostname : hostname;
    final newName = !bluetooth && info.hostname.isNotEmpty
        ? info.hostname
        : name;
    if (newPort == null) {
      return RememberedPlayer(
        id: id,
        name: newName,
        hostname: newHostname,
        bluetooth: bluetooth,
        mdnsName: mdnsName,
      );
    }
    final newHosts = <String>{
      ?connectedHost,
      ...info.addresses,
      ...hosts.where((h) => InternetAddress.tryParse(h) == null),
    };
    return RememberedPlayer(
      id: id,
      name: newName,
      hosts: newHosts.take(maxHosts).toList(),
      port: newPort,
      hostname: newHostname,
      bluetooth: bluetooth,
      mdnsName: mdnsName,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is RememberedPlayer &&
      other.id == id &&
      other.name == name &&
      other.port == port &&
      other.hostname == hostname &&
      other.bluetooth == bluetooth &&
      other.mdnsName == mdnsName &&
      other.hosts.length == hosts.length &&
      [for (var i = 0; i < hosts.length; i++) other.hosts[i] == hosts[i]]
          .every((same) => same);

  @override
  int get hashCode => Object.hash(
    id,
    name,
    port,
    hostname,
    bluetooth,
    mdnsName,
    Object.hashAll(hosts),
  );
}

/// Opens connections to a player over Wi-Fi. The real one is in
/// `lib/src/platform/`; tests use a fake.
abstract class WifiLink {
  /// Connects to the player at [host] and [port] and waits for it to answer.
  /// The channel that comes back is ready. Throws a `PlayerException` when
  /// nothing answers within [timeout], or when the player answers with an
  /// error (`UNIMPLEMENTED` for a player from before Wi-Fi).
  Future<PlayerChannel> connect(String host, int port, {Duration timeout});
}

/// Looks for players on the network by the DNS-SD service they advertise.
/// The real one is in `lib/src/platform/`; tests use a fake.
abstract class PlayerBrowser {
  /// Browses for players: it starts when the stream is listened to and stops
  /// when the subscription is cancelled. Each event is the whole set of
  /// players found so far and resolved to an address and a port, so a player
  /// that goes away (its service is lost) is a set without it. Only players
  /// with at least one address are in a set. A failure (the platform cannot
  /// browse, or refused) is an error on the stream, after which it is over.
  Stream<List<FoundPlayer>> browse();
}

/// Where the app keeps the player it last used, across launches.
abstract class PlayerStore {
  /// The remembered player, or null when there is none.
  Future<RememberedPlayer?> load();

  /// Remembers [player] in place of any other.
  Future<void> save(RememberedPlayer player);

  /// Forgets the remembered player.
  Future<void> clear();
}

/// Small choices about how the app looks that it keeps across launches,
/// such as which side of a tab was showing, each under a key.
abstract class ViewStore {
  /// The choice kept under [key], or null when there is none (or it could
  /// not be read).
  Future<String?> read(String key);

  /// Keeps [value] under [key]. A failure is not reported: the choice is
  /// only not kept.
  Future<void> write(String key, String value);
}

/// A [ViewStore] that keeps its choices for as long as it lives.
class MemoryViewStore implements ViewStore {
  /// A store holding [initial].
  MemoryViewStore([Map<String, String>? initial]) : values = {...?initial};

  /// What it holds.
  final Map<String, String> values;

  @override
  Future<String?> read(String key) async => values[key];

  @override
  Future<void> write(String key, String value) async => values[key] = value;
}

/// Keeps the screen on, for as long as the app asks.
abstract class ScreenWake {
  /// Turns keeping the screen on on or off.
  Future<void> setEnabled(bool enabled);
}
