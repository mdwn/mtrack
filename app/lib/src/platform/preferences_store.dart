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

  final SharedPreferencesAsync _preferences;

  @override
  Future<RememberedPlayer?> load() async {
    final id = await _preferences.getString(_idKey);
    if (id == null || id.isEmpty) {
      return null;
    }
    final name = await _preferences.getString(_nameKey);
    return RememberedPlayer(id: id, name: name ?? id);
  }

  @override
  Future<void> save(RememberedPlayer player) async {
    await _preferences.setString(_idKey, player.id);
    await _preferences.setString(_nameKey, player.name);
  }

  @override
  Future<void> clear() async {
    await _preferences.remove(_idKey);
    await _preferences.remove(_nameKey);
  }
}
