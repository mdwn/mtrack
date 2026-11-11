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
/// A player the app has paired with and will reconnect to.
class RememberedPlayer {
  /// A player with the platform's [id] and the [name] it advertised.
  const RememberedPlayer({required this.id, required this.name});

  /// The platform's identifier for the peripheral.
  final String id;

  /// The player's name, as it was when last seen.
  final String name;

  @override
  bool operator ==(Object other) =>
      other is RememberedPlayer && other.id == id && other.name == name;

  @override
  int get hashCode => Object.hash(id, name);
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
