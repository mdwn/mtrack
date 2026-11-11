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

import 'package:flutter/material.dart';

import '../app/session.dart';
import 'connect_screen.dart';
import 'home_shell.dart';
import 'session_scope.dart';
import 'theme.dart';

/// The app: both themes, following the phone's setting, over a [Session].
class MtrackApp extends StatelessWidget {
  /// The app over [session]. It does not dispose the session.
  const MtrackApp({required this.session, this.themeMode, super.key});

  /// The session every screen reads.
  final Session session;

  /// Overrides the phone's light or dark setting. Null follows it.
  final ThemeMode? themeMode;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'mtrack',
      debugShowCheckedModeBanner: false,
      theme: lightTheme,
      darkTheme: darkTheme,
      themeMode: themeMode ?? ThemeMode.system,
      home: SessionScope(session: session, child: const _Root()),
    );
  }
}

/// Chooses between the connect screen and the tabs, reports whether the app
/// is in the foreground, and shows the session's notices.
class _Root extends StatefulWidget {
  const _Root();

  @override
  State<_Root> createState() => _RootState();
}

class _RootState extends State<_Root> with WidgetsBindingObserver {
  StreamSubscription<String>? _notices;
  Session? _session;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final session = SessionScope.read(context);
    if (!identical(session, _session)) {
      unawaited(_notices?.cancel());
      _session = session;
      _notices = session.notices.listen(_show);
    }
  }

  void _show(String message) {
    if (!mounted) {
      return;
    }
    final messenger = ScaffoldMessenger.of(context);
    messenger
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _session?.setForeground(state == AppLifecycleState.resumed);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_notices?.cancel());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final showTabs =
        session.phase == ConnectionPhase.ready ||
        (session.phase == ConnectionPhase.lost && session.status != null);
    return Scaffold(
      body: SafeArea(
        // The tab bar keeps itself above the system's bar; the connect
        // screen has no tab bar, so its buttons need the room here.
        bottom: !showTabs,
        child: showTabs ? const HomeShell() : const ConnectScreen(),
      ),
    );
  }
}
