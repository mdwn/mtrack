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

import 'package:flutter_test/flutter_test.dart';

import '../support/app_harness.dart';

/// Loads the bundled fonts once per UI test file, before any test builds a
/// widget. It lives in test/ui so the other tests keep their own binding (the
/// widget binding answers every HTTP request with a 400).
///
/// Loading them inside a test leaves the engine's "system fonts changed"
/// message to arrive after the test has torn its tree down, and since Flutter
/// 3.50 the relayout it schedules is reported as an animation still running.
/// Loaded here, there is nothing on screen for the message to relayout.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  TestWidgetsFlutterBinding.ensureInitialized();
  await loadAppFonts();
  await testMain();
}
