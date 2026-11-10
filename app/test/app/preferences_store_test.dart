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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/ports.dart';
import 'package:mtrack_remote/src/platform/preferences_store.dart';
import 'package:shared_preferences_platform_interface/in_memory_shared_preferences_async.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_async_platform_interface.dart';

void main() {
  setUp(() {
    SharedPreferencesAsyncPlatform.instance =
        InMemorySharedPreferencesAsync.empty();
  });

  test('remembers nothing at first', () async {
    expect(await PreferencesPlayerStore().load(), isNull);
  });

  test('keeps a player across a new store, as across a launch', () async {
    const player = RememberedPlayer(id: 'AA:BB', name: 'stage');
    await PreferencesPlayerStore().save(player);
    expect(await PreferencesPlayerStore().load(), player);
  });

  test('replaces the player, and forgets it', () async {
    final store = PreferencesPlayerStore();
    await store.save(const RememberedPlayer(id: '1', name: 'one'));
    await store.save(const RememberedPlayer(id: '2', name: 'two'));
    expect(await store.load(), const RememberedPlayer(id: '2', name: 'two'));
    await store.clear();
    expect(await store.load(), isNull);
  });
}
