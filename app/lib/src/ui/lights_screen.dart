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

import 'package:flutter/material.dart';

import '../app/session.dart';
import '../app/stage_pusher.dart';
import '../app/stage_watch.dart';
import 'ambient_card.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'stage_surface.dart';
import 'theme.dart';
import 'widgets.dart';

/// What the Lights tab says in place of the stage against a player from
/// before the stage stream, in the words of the settings sheet's note.
const String lightsTooOldMessage =
    'This player runs an older mtrack, so its lights are not shown. '
    'Update mtrack on the player to see them.';

/// What the Lights tab says while it has no layout to draw yet.
const String lightsWaitingMessage = 'Waiting for the lights…';

/// What the Lights tab says when the stage page could not be shown.
const String lightsFailedMessage = 'Could not show the lights.';

/// How long the page may take to say it is ready before the tab gives up
/// on it.
const Duration stagePageTimeout = Duration(seconds: 20);

/// How many errors the page may report within [stagePageErrorWindow], once
/// it is running, before the tab gives up on it. One by itself is logged:
/// by the page's contract an update that does not decode changes nothing on
/// screen.
const int stagePageErrorLimit = 3;

/// The time [stagePageErrorLimit] is counted over.
const Duration stagePageErrorWindow = Duration(seconds: 10);

/// The Lights tab: the ambient light's control, and under it the player's
/// stage plot, live. The plot is the web UI's own page in a web view
/// ([StageSurface]), fed the player's stage stream.
class LightsScreen extends StatefulWidget {
  /// The Lights tab. [active] is true while the tab is the one on show.
  const LightsScreen({required this.active, required this.host, super.key});

  /// Whether this tab is showing.
  final bool active;

  /// Where the stage page and the surface to show it in come from.
  final StageHost host;

  @override
  State<LightsScreen> createState() => _LightsScreenState();
}

class _LightsScreenState extends State<LightsScreen>
    with WidgetsBindingObserver {
  late final StagePusher _pusher = StagePusher(
    run: (javaScript) => _surface!.runJavaScript(javaScript),
    restart: _relisten,
    failed: (error) => _fail('a call into the page failed', error),
  );

  StageSurface? _surface;
  StreamSubscription<String>? _messages;
  StreamSubscription<String>? _errors;
  StreamSubscription<StageEvent>? _stage;

  /// False while the app is not on screen.
  bool _foreground = true;

  /// True when the page could not be shown, until "Try again".
  bool _failed = false;

  /// True from listening to the stream until its reset arrives: what the
  /// page shows meanwhile is the stream before's.
  bool _awaitingReset = false;

  /// True when the page's address is to be checked the next time the tab is
  /// on show: the app was away, and its server may not have survived.
  bool _checkPage = false;

  /// The address the surface was last told to load.
  Uri? _page;

  /// Moves with every load, so a load that was overtaken does nothing more.
  int _loads = 0;

  Timer? _readyTimer;
  Timer? _errorWindow;
  int _pageErrors = 0;
  Brightness? _brightness;
  bool _syncQueued = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _foreground = _onScreen(WidgetsBinding.instance.lifecycleState);
  }

  /// Whether the app is on screen in [state]. `inactive` still is: the
  /// notification shade or a system dialog is over it, and starting the
  /// stream over for that would blank the stage for nothing.
  static bool _onScreen(AppLifecycleState? state) =>
      state == null ||
      state == AppLifecycleState.resumed ||
      state == AppLifecycleState.inactive;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // Depends on the session, so this runs whenever it changes.
    SessionScope.of(context);
    final brightness = Theme.of(context).brightness;
    if (brightness != _brightness) {
      final first = _brightness == null;
      _brightness = brightness;
      final surface = _surface;
      if (!first && surface != null) {
        _pusher.setTheme(_themeName);
        unawaited(_paint(surface));
      }
    }
    _queueSync();
  }

  @override
  void didUpdateWidget(LightsScreen old) {
    super.didUpdateWidget(old);
    if (widget.active != old.active) {
      if (widget.active) {
        _checkPage = true;
      }
      _queueSync();
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final foreground = _onScreen(state);
    if (foreground == _foreground) {
      return;
    }
    _foreground = foreground;
    if (foreground) {
      _checkPage = true;
    }
    _sync();
  }

  String get _themeName => _brightness == Brightness.light ? 'light' : 'dark';

  /// The page's own background in the theme, so there is no seam around it
  /// and no flash before it paints.
  Future<void> _paint(StageSurface surface) async {
    try {
      await surface.setBackground(context.colors.surface);
    } on Object catch (e) {
      debugPrint('Lights: could not set the web view background: $e');
    }
  }

  /// Runs [_sync] once the frame is built: it can start the stream, which
  /// changes what is on screen.
  void _queueSync() {
    if (_syncQueued) {
      return;
    }
    _syncQueued = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _syncQueued = false;
      _sync();
    });
  }

  /// Makes what is running match what is on show: the surface exists once
  /// the tab has been looked at, and the stream is listened to only while
  /// the tab is the one showing, the app is on screen and the link is up.
  void _sync() {
    if (!mounted) {
      return;
    }
    final session = SessionScope.read(context);
    final visible = widget.active && _foreground;
    final tooOld = session.stageMissingFromPlayer;
    if (tooOld && _surface != null) {
      _closeSurface();
      setState(() => _failed = false);
    }
    if (visible && !tooOld && !_failed) {
      if (_surface == null) {
        _open();
      } else if (_checkPage) {
        _checkPage = false;
        unawaited(_load(force: false));
      }
    }
    final wanted =
        visible && !tooOld && !_failed && session.isReady && _surface != null;
    if (wanted && _stage == null) {
      _listen(session);
    } else if (!wanted && _stage != null) {
      _unlisten();
    }
  }

  void _open() {
    final surface = widget.host.createSurface();
    _messages = surface.messages.listen(_onMessage);
    _errors = surface.errors.listen(
      (error) => _fail('the page did not load', error),
    );
    setState(() => _surface = surface);
    _checkPage = false;
    unawaited(_load(force: true));
  }

  void _closeSurface() {
    _loads++;
    _readyTimer?.cancel();
    unawaited(_messages?.cancel());
    unawaited(_errors?.cancel());
    _messages = null;
    _errors = null;
    _surface?.dispose();
    _surface = null;
    _page = null;
  }

  /// Loads the page into the surface. Without [force], only if it is no
  /// longer where it was: its server had to start again.
  Future<void> _load({required bool force}) async {
    final surface = _surface;
    if (surface == null) {
      return;
    }
    final load = ++_loads;
    bool overtaken() =>
        !mounted || load != _loads || !identical(surface, _surface);
    try {
      final page = await widget.host.serve();
      if (overtaken() || (!force && page == _page)) {
        return;
      }
      _page = page;
      _pusher.pageLoading();
      _pageErrors = 0;
      _readyTimer?.cancel();
      _readyTimer = Timer(
        stagePageTimeout,
        () => _fail('the page did not start', 'no ready message'),
      );
      await _paint(surface);
      if (overtaken()) {
        return;
      }
      await surface.load(page.replace(queryParameters: {'theme': _themeName}));
    } on Object catch (e) {
      if (!overtaken()) {
        _fail('could not load the page', e);
      }
    }
  }

  void _listen(Session session) {
    setState(() => _awaitingReset = true);
    // A plain callback that never waits: the session queues nothing for a
    // listener that pauses, and starts the stream over instead.
    _stage = session.watchStage().listen(_onStage);
  }

  void _unlisten() {
    unawaited(_stage?.cancel());
    _stage = null;
    _pusher.clear();
  }

  /// The page loaded again and needs the stream from its start.
  void _relisten() {
    if (_stage == null || !mounted) {
      return;
    }
    _unlisten();
    _listen(SessionScope.read(context));
  }

  void _onStage(StageEvent event) {
    if (event is StageReset && _awaitingReset && mounted) {
      setState(() => _awaitingReset = false);
    }
    _pusher.add(event);
  }

  void _onMessage(String raw) {
    Object? message;
    try {
      message = jsonDecode(raw);
    } on FormatException {
      message = null;
    }
    final type = message is Map<String, dynamic> ? message['type'] : null;
    switch (type) {
      case 'ready':
        _readyTimer?.cancel();
        _pusher.pageReady();
      case 'error':
        _pageError('${(message! as Map<String, dynamic>)['message']}');
      default:
        debugPrint('Lights: the page said something unknown: $raw');
    }
  }

  /// The page reported an error. Before it is ready, it did not start. Once
  /// it runs, one error is logged and the page carries on, as its contract
  /// says it does; several close together mean it is broken.
  void _pageError(String message) {
    if (!_pusher.pageIsReady) {
      _fail('the page did not start', message);
      return;
    }
    debugPrint('Lights: the page reported: $message');
    _pageErrors++;
    _errorWindow ??= Timer(stagePageErrorWindow, () {
      _errorWindow = null;
      _pageErrors = 0;
    });
    if (_pageErrors >= stagePageErrorLimit) {
      _fail('the page keeps failing', message);
    }
  }

  void _fail(String what, Object detail) {
    debugPrint('Lights: $what: $detail');
    if (!mounted || _failed) {
      return;
    }
    _readyTimer?.cancel();
    setState(() => _failed = true);
    _sync();
  }

  void _retry() {
    setState(() {
      _failed = false;
      _pageErrors = 0;
    });
    if (_surface == null) {
      _sync();
    } else {
      unawaited(_load(force: true));
      _sync();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_stage?.cancel());
    _errorWindow?.cancel();
    _closeSurface();
    _pusher.dispose();
    unawaited(widget.host.stop());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    return Column(
      children: [
        const PlayerBar(),
        const ReconnectBanner(),
        if (session.ambientMode != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 10, 18, 0),
            child: AmbientCard(session: session),
          ),
        const SizedBox(height: 12),
        Expanded(
          // The page's own colour, edge to edge, so the page has no seam.
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: colors.surface,
              border: Border(top: BorderSide(color: colors.line)),
            ),
            child: Padding(
              padding: const EdgeInsets.only(top: 1),
              child: ListenableBuilder(
                listenable: _pusher,
                builder: (context, _) => _stageArea(context, session),
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _stageArea(BuildContext context, Session session) {
    final colors = context.colors;
    if (session.stageMissingFromPlayer) {
      return _Cover(
        color: colors.surface,
        child: Text(
          lightsTooOldMessage,
          textAlign: TextAlign.center,
          style: AppText.body.copyWith(color: colors.amber),
        ),
      );
    }
    final surface = _surface;
    final live = session.isReady;
    final Widget? over;
    if (_failed) {
      over = _Cover(
        color: colors.surface,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              lightsFailedMessage,
              textAlign: TextAlign.center,
              style: AppText.body.copyWith(color: colors.muted),
            ),
            const SizedBox(height: 16),
            SizedBox(
              width: 200,
              child: ChunkyButton(
                height: 52,
                fontSize: 16,
                label: 'Try again',
                semanticLabel: 'Try again',
                onPressed: _retry,
              ),
            ),
          ],
        ),
      );
    } else if (!live) {
      // The link is down. Lights that were showing stay, dimmed like every
      // other tab's last state, under the "Reconnecting…" banner; with
      // nothing to show, nothing is shown.
      over = _pusher.showing && _pusher.pageIsReady
          ? ColoredBox(
              key: const Key('stage-dimmed'),
              color: colors.surface.withAlpha(170),
            )
          : ColoredBox(key: const Key('stage-blank'), color: colors.surface);
    } else if (!widget.active) {
      // Behind another tab: nothing that moves, so the app draws no frames
      // for a tab nobody sees.
      over = ColoredBox(color: colors.surface);
    } else if (surface == null ||
        !_pusher.pageIsReady ||
        !_pusher.showing ||
        _awaitingReset) {
      over = _Cover(
        color: colors.surface,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: 20,
              height: 20,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: colors.dim,
              ),
            ),
            const SizedBox(height: 14),
            Text(
              lightsWaitingMessage,
              textAlign: TextAlign.center,
              style: AppText.body.copyWith(color: colors.muted),
            ),
          ],
        ),
      );
    } else {
      over = null;
    }
    return Stack(
      fit: StackFit.expand,
      children: [
        if (surface != null)
          // A picture, not a control: the plot has nothing to tap, so no
          // touch reaches the page, and a screen reader is told what it is
          // instead of being walked through the page.
          Semantics(
            label: 'Stage lights',
            image: true,
            child: ExcludeSemantics(
              child: IgnorePointer(child: surface.widget),
            ),
          ),
        ?over,
      ],
    );
  }
}

/// Something said over the whole stage area, in the middle of it. It scrolls
/// if large text on a short phone leaves it no room.
class _Cover extends StatelessWidget {
  const _Cover({required this.color, required this.child});

  final Color color;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: color,
      child: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(24),
          child: child,
        ),
      ),
    );
  }
}
