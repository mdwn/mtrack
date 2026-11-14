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

import 'package:mtrack_remote/src/app/ports.dart';

/// A [PlayerBrowser] whose network is whatever the test puts on it.
class FakeBrowser implements PlayerBrowser {
  /// The players on the network now, by name.
  final Map<String, FoundPlayer> present = {};

  /// How long after browsing starts the players already [present] are
  /// reported. Zero reports them as soon as the stream is listened to.
  Duration answerAfter = Duration.zero;

  /// When set, [browse] throws this, as a plugin that cannot start does.
  Object? throwOnBrowse;

  /// When set, a browse fails with this as soon as it starts.
  Object? failOnStart;

  /// How many times browsing was started.
  int starts = 0;

  /// How many times a browse was cancelled.
  int cancels = 0;

  final List<_Browse> _active = [];

  /// Whether anyone is browsing now.
  bool get browsing => _active.isNotEmpty;

  /// Puts [player] on the network and tells whoever is browsing.
  void appear(FoundPlayer player) {
    present[player.name] = player;
    _tell();
  }

  /// Takes the player [name] off the network (its service is lost).
  void vanish(String name) {
    present.remove(name);
    _tell();
  }

  /// Fails every browse in progress with [error].
  void fail(Object error) {
    for (final browse in List.of(_active)) {
      browse.controller.addError(error);
    }
  }

  void _tell() {
    for (final browse in _active) {
      if (browse.reporting) {
        browse.controller.add(present.values.toList());
      }
    }
  }

  @override
  Stream<List<FoundPlayer>> browse() {
    final thrown = throwOnBrowse;
    if (thrown != null) {
      throw thrown;
    }
    final browse = _Browse();
    browse.controller = StreamController<List<FoundPlayer>>(
      sync: true,
      onListen: () {
        starts++;
        _active.add(browse);
        final failure = failOnStart;
        if (failure != null) {
          browse.controller.addError(failure);
          return;
        }
        void report() {
          browse.reporting = true;
          if (present.isNotEmpty) {
            browse.controller.add(present.values.toList());
          }
        }

        if (answerAfter == Duration.zero) {
          report();
        } else {
          browse.timer = Timer(answerAfter, report);
        }
      },
      onCancel: () {
        cancels++;
        browse.timer?.cancel();
        _active.remove(browse);
      },
    );
    return browse.controller.stream;
  }
}

class _Browse {
  late final StreamController<List<FoundPlayer>> controller;
  Timer? timer;
  bool reporting = false;
}
