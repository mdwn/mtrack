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
import 'package:flutter/widgets.dart';

import '../app/session.dart';

/// Makes a [Session] available below it and rebuilds what reads it when it
/// changes.
class SessionScope extends InheritedNotifier<Session> {
  /// Provides [session] to [child].
  const SessionScope({
    required Session session,
    required super.child,
    super.key,
  }) : super(notifier: session);

  /// The session in scope. Rebuilds the caller when it changes.
  static Session of(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<SessionScope>();
    assert(scope != null, 'No SessionScope above this context');
    return scope!.notifier!;
  }

  /// The session in scope, without rebuilding the caller when it changes.
  static Session read(BuildContext context) {
    final scope = context
        .getElementForInheritedWidgetOfExactType<SessionScope>()
        ?.widget;
    assert(scope != null, 'No SessionScope above this context');
    return (scope! as SessionScope).notifier!;
  }
}
