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

import 'package:bonsoir/bonsoir.dart';

import '../app/ports.dart';

/// Browses for players with the platform's own DNS-SD browser (`NsdManager`
/// on Android, Bonjour on iOS) through the `bonsoir` plugin. This is the only
/// file that knows the plugin, so it can be replaced by rewriting it.
///
/// It does nothing but translate the plugin's events into sets of
/// [FoundPlayer], so it cannot be tested without a phone.
class BonsoirPlayerBrowser implements PlayerBrowser {
  /// A browser for the players' service.
  const BonsoirPlayerBrowser();

  /// The DNS-SD service type a player advertises on its gRPC port.
  static const String serviceType = '_mtrack._tcp';

  @override
  Stream<List<FoundPlayer>> browse() {
    late final StreamController<List<FoundPlayer>> out;
    final services = <String, BonsoirService>{};
    BonsoirDiscovery? discovery;
    StreamSubscription<BonsoirDiscoveryEvent>? events;
    var cancelled = false;

    void emit() {
      if (cancelled || out.isClosed) {
        return;
      }
      out.add([
        for (final service in services.values)
          if (service.hostAddresses.isNotEmpty && service.port > 0)
            FoundPlayer(
              name: service.name,
              hosts: service.hostAddresses.toSet().toList(),
              port: service.port,
              version: service.attributes['v'],
            ),
      ]);
    }

    void fail(Object error) {
      if (!cancelled && !out.isClosed) {
        out.addError(error);
      }
    }

    void handle(BonsoirDiscoveryEvent event) {
      switch (event) {
        case BonsoirDiscoveryServiceFoundEvent(:final service):
          services.putIfAbsent(service.name, () => service);
          final d = discovery;
          if (d != null) {
            // Found is only the name; the addresses and port come with the
            // resolved event.
            service.resolve(d.serviceResolver).catchError(fail);
          }
        case BonsoirDiscoveryServiceResolvedEvent(:final service):
        case BonsoirDiscoveryServiceUpdatedEvent(:final service):
          final before = services[service.name];
          // An update (the TXT record arriving) may not repeat the addresses.
          services[service.name] =
              service.hostAddresses.isEmpty && before != null
              ? service.copyWith(
                  hostAddresses: before.hostAddresses,
                  port: service.port > 0 ? service.port : before.port,
                )
              : service;
          emit();
        case BonsoirDiscoveryServiceLostEvent(:final service):
          services.remove(service.name);
          emit();
        default:
          break;
      }
    }

    Future<void> stopDiscovery(BonsoirDiscovery? d) async {
      try {
        await d?.stop();
      } on Object {
        // Already stopped, or never started: nothing more to stop.
      }
    }

    Future<void> start() async {
      try {
        final d = BonsoirDiscovery(type: serviceType);
        discovery = d;
        await d.initialize();
        if (cancelled) {
          await stopDiscovery(d);
          return;
        }
        events = d.eventStream!.listen(handle, onError: fail);
        await d.start();
        if (cancelled) {
          await stopDiscovery(d);
        }
      } on Object catch (e) {
        fail(e);
      }
    }

    out = StreamController<List<FoundPlayer>>(
      onListen: () => unawaited(start()),
      onCancel: () async {
        cancelled = true;
        await events?.cancel();
        await stopDiscovery(discovery);
        discovery = null;
      },
    );
    return out.stream;
  }
}
