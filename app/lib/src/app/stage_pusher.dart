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

import 'package:flutter/foundation.dart';

import 'stage_watch.dart';

/// The first byte of a serialized `StageUpdate` that holds a layout: field 1,
/// length-delimited. The player tells the two apart the same way.
const int stageLayoutTag = 0x0A;

/// The first byte of a serialized `StageUpdate` that holds a frame: field 2,
/// length-delimited.
const int stageFrameTag = 0x12;

/// Something waiting to go to the page that may not be dropped or reordered:
/// a reset, or a message that is not a frame.
class _Kept {
  const _Kept.reset() : bytes = null;
  const _Kept.message(Uint8List this.bytes);

  /// Null for a reset.
  final Uint8List? bytes;

  bool get isReset => bytes == null;
  bool get isLayout => bytes != null && bytes![0] == stageLayoutTag;
}

/// Hands the stage stream to the stage page (`window.mtrackStage`, the host
/// contract in `src/webui/svelte/src/stage-main.ts`), one JavaScript call at
/// a time.
///
/// The stream can outrun the page: a call into a web view takes a while and
/// the session queues nothing for a slow listener (see `StageWatch`). So this
/// takes every event at once, and keeps what the page still has to be given:
///
/// - Nothing is sent before the page has said it is ready. What arrives
///   meanwhile waits here, under the same rules.
/// - A reset and a layout are never dropped and never reordered. A reset
///   empties what waited ahead of it, since `reset()` on the page forgets
///   all of that anyway.
/// - At most one frame waits, the newest, and it goes after everything else:
///   a frame stands alone, so the ones it replaces are no loss. A frame that
///   waits when a layout or a reset arrives is dropped, since it was drawn
///   against the layout before.
/// - While one call is out, the next is not made. All that waits when it
///   returns goes in one call.
///
/// A page that loads again has lost what it was given. [pageLoading] and
/// [pageReady] notice that and ask for the stream to be started over
/// (`restart`), so the page is sent a layout again.
///
/// Listeners are told when [pageIsReady] or [showing] changes.
class StagePusher extends ChangeNotifier {
  /// A pusher that calls into the page with [run], asks for the stream to
  /// be started over with [restart], and reports a call that failed to
  /// [failed].
  StagePusher({
    required this._run,
    required this._restart,
    required this._failed,
  });

  final Future<void> Function(String javaScript) _run;
  final void Function() _restart;
  final void Function(Object error) _failed;

  final List<_Kept> _kept = [];
  Uint8List? _frame;
  String? _theme;
  bool _retry3d = false;

  bool _ready = false;
  bool _busy = false;
  bool _showing = false;
  bool _disposed = false;

  /// True once anything of the stream has been handed to the page that is
  /// loaded now, so a page that replaces it needs the stream from its start.
  bool _given = false;

  /// Moves when the page is replaced, so the return of a call into the page
  /// before is ignored.
  int _page = 0;

  /// True once the page has said it is ready, until it loads again.
  bool get pageIsReady => _ready;

  /// True while the page holds a layout of the stream being listened to: one
  /// has been handed over since the last reset. Until then the page has
  /// nothing of this stream to draw.
  bool get showing => _showing;

  /// True while a call into the page is out.
  bool get isBusy => _busy;

  /// The page is about to load, or load again. Nothing is sent until it is
  /// ready. If the page before had been given any of the stream, the stream
  /// is asked to start over, so that the new page gets its layout.
  void pageLoading() {
    if (_disposed) {
      return;
    }
    _replaced(wasReady: _ready);
    _ready = false;
    notifyListeners();
  }

  /// The page says it is ready. What waited is sent. A page that says so
  /// when it already had is one that loaded again by itself: the stream is
  /// started over for it.
  void pageReady() {
    if (_disposed) {
      return;
    }
    if (_ready) {
      _replaced(wasReady: true);
    }
    _ready = true;
    notifyListeners();
    _pump();
  }

  void _replaced({required bool wasReady}) {
    _page++;
    _busy = false;
    _showing = false;
    _retry3d = false;
    if (wasReady && _given) {
      _given = false;
      _kept.clear();
      _frame = null;
      _restart();
    }
  }

  /// Takes one event of the stream. Never waits: listen with this as a
  /// plain callback.
  void add(StageEvent event) {
    if (_disposed) {
      return;
    }
    switch (event) {
      case StageReset():
        _kept
          ..clear()
          ..add(const _Kept.reset());
        _frame = null;
        if (_showing) {
          _showing = false;
          notifyListeners();
        }
      case StageBytes(:final bytes):
        if (bytes.isEmpty) {
          // A `StageUpdate` with nothing in it.
          return;
        }
        if (bytes[0] == stageFrameTag) {
          _frame = bytes;
        } else {
          // A layout, or something newer than this app: kept, in order.
          if (bytes[0] == stageLayoutTag) {
            _frame = null;
          }
          _kept.add(_Kept.message(bytes));
        }
    }
    _pump();
  }

  /// The page's theme is to be [theme] (`light` or `dark`). Sent when the
  /// page is ready and no call is out, like everything else.
  void setTheme(String theme) {
    if (_disposed) {
      return;
    }
    _theme = theme;
    _pump();
  }

  /// The page is to build its 3D view again, asking again for the files it
  /// could not have (`retry3d()`). Sent when the page is ready and no call
  /// is out, like everything else; a page that loads again has no 3D view
  /// to build again, so it is not sent to one.
  void retry3d() {
    if (_disposed) {
      return;
    }
    _retry3d = true;
    _pump();
  }

  /// The stream was cancelled: what waited is of no use to the page. The
  /// page keeps what it shows until the next stream's reset.
  void clear() {
    _kept.clear();
    _frame = null;
  }

  void _pump() {
    if (!_ready || _busy) {
      return;
    }
    final theme = _theme;
    final retry3d = _retry3d;
    final kept = List.of(_kept);
    final frame = _frame;
    if (theme == null && !retry3d && kept.isEmpty && frame == null) {
      return;
    }
    _theme = null;
    _retry3d = false;
    _kept.clear();
    _frame = null;

    // Base64's alphabet has nothing a quoted string needs escaped.
    final script = StringBuffer('(function(){var s=window.mtrackStage;');
    if (theme != null) {
      script.write("s.setTheme('$theme');");
    }
    if (retry3d) {
      script.write('s.retry3d();');
    }
    for (final item in kept) {
      script.write(
        item.isReset
            ? 's.reset();'
            : "s.update('${base64.encode(item.bytes!)}');",
      );
    }
    if (frame != null) {
      script.write("s.update('${base64.encode(frame)}');");
    }
    script.write('})();');

    if (kept.isNotEmpty || frame != null) {
      _given = true;
    }
    final page = _page;
    _busy = true;
    Future<void>.sync(() => _run(script.toString())).then(
      (_) {
        if (_disposed || page != _page) {
          return;
        }
        _busy = false;
        // A layout went, and no reset has come since to take it back.
        final resetWaits = _kept.isNotEmpty && _kept.first.isReset;
        if (!_showing && !resetWaits && kept.any((item) => item.isLayout)) {
          _showing = true;
          notifyListeners();
        }
        _pump();
      },
      onError: (Object error) {
        if (_disposed || page != _page) {
          return;
        }
        _busy = false;
        _kept.clear();
        _frame = null;
        _failed(error);
      },
    );
  }

  @override
  void dispose() {
    _disposed = true;
    _page++;
    _kept.clear();
    _frame = null;
    super.dispose();
  }
}
