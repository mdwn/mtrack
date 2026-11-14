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
import 'package:shared_preferences/shared_preferences.dart';

import '../app/ports.dart';

/// Remembers the player in the app's preferences.
class PreferencesPlayerStore implements PlayerStore {
  /// A store over [SharedPreferencesAsync].
  PreferencesPlayerStore([SharedPreferencesAsync? preferences])
    : _preferences = preferences ?? SharedPreferencesAsync();

  static const String _idKey = 'player.id';
  static const String _nameKey = 'player.name';
  static const String _hostsKey = 'player.wifi.hosts';
  static const String _portKey = 'player.wifi.port';
  static const String _hostnameKey = 'player.wifi.hostname';
  static const String _bluetoothKey = 'player.bluetooth';
  static const String _mdnsKey = 'player.wifi.mdns';

  final SharedPreferencesAsync _preferences;

  @override
  Future<RememberedPlayer?> load() async {
    final id = await _preferences.getString(_idKey);
    if (id == null || id.isEmpty) {
      return null;
    }
    final name = await _preferences.getString(_nameKey);
    final hostname = await _preferences.getString(_hostnameKey);
    final mdnsName = await _preferences.getString(_mdnsKey);
    return RememberedPlayer(
      id: id,
      name: name ?? id,
      hosts: await _preferences.getStringList(_hostsKey) ?? const [],
      port: await _preferences.getInt(_portKey),
      hostname: hostname == null || hostname.isEmpty ? null : hostname,
      // A player kept before Wi-Fi was Bluetooth's.
      bluetooth: await _preferences.getBool(_bluetoothKey) ?? true,
      // A player kept before it could be found on the network has none.
      mdnsName: mdnsName == null || mdnsName.isEmpty ? null : mdnsName,
    );
  }

  @override
  Future<void> save(RememberedPlayer player) async {
    await _preferences.setString(_idKey, player.id);
    await _preferences.setString(_nameKey, player.name);
    await _preferences.setStringList(_hostsKey, player.hosts);
    final port = player.port;
    if (port == null) {
      await _preferences.remove(_portKey);
    } else {
      await _preferences.setInt(_portKey, port);
    }
    final hostname = player.hostname;
    if (hostname == null) {
      await _preferences.remove(_hostnameKey);
    } else {
      await _preferences.setString(_hostnameKey, hostname);
    }
    final mdnsName = player.mdnsName;
    if (mdnsName == null) {
      await _preferences.remove(_mdnsKey);
    } else {
      await _preferences.setString(_mdnsKey, mdnsName);
    }
    await _preferences.setBool(_bluetoothKey, player.bluetooth);
  }

  @override
  Future<void> clear() async {
    for (final key in [
      _idKey,
      _nameKey,
      _hostsKey,
      _portKey,
      _hostnameKey,
      _bluetoothKey,
      _mdnsKey,
    ]) {
      await _preferences.remove(key);
    }
  }
}

/// Keeps the app's view choices in its preferences.
class PreferencesViewStore implements ViewStore {
  /// A store over [SharedPreferencesAsync].
  PreferencesViewStore([SharedPreferencesAsync? preferences])
    : _preferences = preferences ?? SharedPreferencesAsync();

  final SharedPreferencesAsync _preferences;

  @override
  Future<String?> read(String key) async {
    try {
      return await _preferences.getString(key);
    } on Object {
      return null;
    }
  }

  @override
  Future<void> write(String key, String value) async {
    try {
      await _preferences.setString(key, value);
    } on Object {
      // Not kept: the next launch starts as a first launch does.
    }
  }
}
