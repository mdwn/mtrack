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
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:mtrack_remote/src/app/stage_pusher.dart';
import 'package:mtrack_remote/src/ui/stage_surface.dart';

/// A serialized `StageUpdate` holding a layout, as far as the app can tell:
/// the layout's first byte, then [n] to tell one from another.
List<int> stageLayout(int n) => [stageLayoutTag, n];

/// A serialized `StageUpdate` holding a frame, numbered [n].
List<int> stageFrame(int n) => [stageFrameTag, n];

/// What a script the pusher ran does to the page, in order: `reset`,
/// `theme:dark`, `retry3d`, and `L3` or `F7` for an update made by
/// [stageLayout] or [stageFrame].
List<String> stageCalls(String script) {
  final calls = <String>[];
  final call = RegExp(r"s\.(reset|update|setTheme|retry3d)\((?:'([^']*)')?\);");
  for (final match in call.allMatches(script)) {
    switch (match.group(1)) {
      case 'reset':
        calls.add('reset');
      case 'retry3d':
        calls.add('retry3d');
      case 'setTheme':
        calls.add('theme:${match.group(2)}');
      default:
        final bytes = base64.decode(match.group(2)!);
        calls.add('${bytes[0] == stageLayoutTag ? 'L' : 'F'}${bytes[1]}');
    }
  }
  return calls;
}

/// A [StageHost] with no server and no web view: it records what the Lights
/// tab asks of it and hands out [FakeStageSurface]s.
class FakeStageHost implements StageHost {
  /// Where the page is said to be. A test changes it to play a server that
  /// had to start again.
  Uri page = Uri.parse('http://127.0.0.1:4000/prefix/stage.html');

  /// When set, [serve] fails with it.
  Object? serveError;

  /// How many times the page was asked for, and serving stopped.
  int serves = 0;
  int stops = 0;

  /// Every surface made, oldest first.
  final List<FakeStageSurface> surfaces = [];

  /// The newest surface.
  FakeStageSurface get surface => surfaces.last;

  @override
  Future<Uri> serve() async {
    serves++;
    final error = serveError;
    if (error != null) {
      throw error;
    }
    return page;
  }

  @override
  Future<void> stop() async => stops++;

  /// The view the host remembers, as [rememberView] last set it.
  String? view;

  /// Every view the tab asked to remember, in order.
  final List<String> remembered = [];

  @override
  Future<String?> rememberedView() async => view;

  @override
  Future<void> rememberView(String view) async {
    this.view = view;
    remembered.add(view);
  }

  @override
  StageSurface createSurface() {
    final surface = FakeStageSurface();
    surfaces.add(surface);
    return surface;
  }
}

/// A [StageSurface] that shows nothing and records everything.
class FakeStageSurface implements StageSurface {
  /// What was loaded, in order.
  final List<Uri> loads = [];

  /// The scripts run, in order.
  final List<String> scripts = [];

  /// The backgrounds set, in order.
  final List<Color> backgrounds = [];

  /// When set, a script does not return until the test completes it.
  Completer<void>? held;

  /// True once the tab let go of it.
  bool disposed = false;

  /// How many touches reached the page.
  int touches = 0;

  final StreamController<String> _messages = StreamController.broadcast(
    sync: true,
  );
  final StreamController<String> _errors = StreamController.broadcast(
    sync: true,
  );

  /// Everything the scripts did to the page, in order (see [stageCalls]).
  List<String> get calls => scripts.expand(stageCalls).toList();

  /// The page posts [raw] to its host.
  void say(String raw) => _messages.add(raw);

  /// The page says it is ready.
  void ready() => say(jsonEncode({'type': 'ready'}));

  /// The page reports an error.
  void pageError(String message) =>
      say(jsonEncode({'type': 'error', 'message': message}));

  /// The page itself could not be loaded.
  void loadError(String description) => _errors.add(description);

  @override
  Future<void> load(Uri page) async => loads.add(page);

  @override
  Future<void> runJavaScript(String javaScript) async {
    scripts.add(javaScript);
    await held?.future;
  }

  @override
  Future<void> setBackground(Color color) async => backgrounds.add(color);

  @override
  Stream<String> get messages => _messages.stream;

  @override
  Stream<String> get errors => _errors.stream;

  @override
  Widget get widget => Listener(
    behavior: HitTestBehavior.opaque,
    onPointerDown: (_) => touches++,
    child: const SizedBox.expand(key: Key('stage-surface')),
  );

  @override
  void dispose() {
    disposed = true;
    unawaited(_messages.close());
    unawaited(_errors.close());
  }
}
