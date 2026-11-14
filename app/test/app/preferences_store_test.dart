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
import 'package:mtrack_remote/src/app/wifi_address.dart';
import 'package:mtrack_remote/src/platform/preferences_store.dart';
import 'package:shared_preferences/shared_preferences.dart';
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

  test(
    'keeps a player\'s Wi-Fi addresses, hostname and port with it',
    () async {
      const player = RememberedPlayer(
        id: 'AA:BB',
        name: 'stage',
        hosts: ['mtrack.local', '192.168.1.40'],
        port: 50051,
        hostname: 'stage',
      );
      await PreferencesPlayerStore().save(player);
      final loaded = await PreferencesPlayerStore().load();
      expect(loaded, player);
      expect(loaded!.bluetooth, isTrue);
    },
  );

  test('keeps a player known only by its address as Wi-Fi only', () async {
    final player = RememberedPlayer.byAddress(
      const WifiAddress('mtrack.local', 43234),
    );
    await PreferencesPlayerStore().save(player);
    final loaded = await PreferencesPlayerStore().load();
    expect(loaded, player);
    expect(loaded!.bluetooth, isFalse);
  });

  test('reads a player kept before Wi-Fi as a Bluetooth one', () async {
    final preferences = SharedPreferencesAsync();
    await preferences.setString('player.id', 'AA:BB');
    await preferences.setString('player.name', 'stage');
    final loaded = await PreferencesPlayerStore().load();
    expect(loaded, const RememberedPlayer(id: 'AA:BB', name: 'stage'));
    expect(loaded!.hasWifi, isFalse);
  });

  test('keeps the name a player was found under on the network', () async {
    final player = RememberedPlayer.byService(
      const FoundPlayer(
        name: 'stage',
        hosts: ['fe80::1', '192.168.1.40'],
        port: 43234,
        version: '0.18.3',
      ),
    );
    expect(player.hosts, ['192.168.1.40', 'fe80::1']);
    await PreferencesPlayerStore().save(player);
    final loaded = await PreferencesPlayerStore().load();
    expect(loaded, player);
    expect(loaded!.mdnsName, 'stage');
    expect(loaded.id, 'mdns:stage');
    expect(loaded.bluetooth, isFalse);
  });

  test('reads a Wi-Fi player kept before names were kept, with none', () async {
    final preferences = SharedPreferencesAsync();
    await preferences.setString('player.id', 'wifi:10.0.0.2:43234');
    await preferences.setString('player.name', 'stage');
    await preferences.setStringList('player.wifi.hosts', ['10.0.0.2']);
    await preferences.setInt('player.wifi.port', 43234);
    await preferences.setString('player.wifi.hostname', 'stage');
    await preferences.setBool('player.bluetooth', false);
    final loaded = await PreferencesPlayerStore().load();
    expect(loaded!.mdnsName, isNull);
    expect(loaded.hosts, ['10.0.0.2']);
    expect(loaded.bluetooth, isFalse);
  });

  test('a player saved without a name does not keep the last one', () async {
    final store = PreferencesPlayerStore();
    await store.save(
      const RememberedPlayer(id: 'mdns:a', name: 'a', mdnsName: 'a'),
    );
    await store.save(const RememberedPlayer(id: 'p', name: 'b'));
    expect((await store.load())!.mdnsName, isNull);
  });

  test('updating what a found player is reachable at keeps its name', () async {
    final store = PreferencesPlayerStore();
    final first = RememberedPlayer.byService(
      const FoundPlayer(name: 'stage', hosts: ['10.0.0.2'], port: 43234),
    );
    await store.save(first);
    await store.save(first.copyWith(hosts: ['10.0.0.9'], port: 50051));
    final loaded = (await store.load())!;
    expect(loaded.hosts, ['10.0.0.9']);
    expect(loaded.port, 50051);
    expect(loaded.mdnsName, 'stage');
  });

  test('forgetting a player forgets its addresses too', () async {
    final store = PreferencesPlayerStore();
    await store.save(
      const RememberedPlayer(
        id: 'AA',
        name: 'stage',
        hosts: ['10.0.0.2'],
        port: 43234,
        hostname: 'stage',
        mdnsName: 'stage',
      ),
    );
    await store.clear();
    expect(await store.load(), isNull);
    // A player saved after has none of the old one's.
    await store.save(const RememberedPlayer(id: 'BB', name: 'other'));
    expect(await store.load(), const RememberedPlayer(id: 'BB', name: 'other'));
    expect((await store.load())!.mdnsName, isNull);
  });

  test('replaces the player, and forgets it', () async {
    final store = PreferencesPlayerStore();
    await store.save(const RememberedPlayer(id: '1', name: 'one'));
    await store.save(const RememberedPlayer(id: '2', name: 'two'));
    expect(await store.load(), const RememberedPlayer(id: '2', name: 'two'));
    await store.clear();
    expect(await store.load(), isNull);
  });

  test('keeps a view choice across a new store, as across a launch', () async {
    expect(await PreferencesViewStore().read('library.view'), isNull);
    await PreferencesViewStore().write('library.view', 'songs');
    expect(await PreferencesViewStore().read('library.view'), 'songs');
  });
}
