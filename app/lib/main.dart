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
import 'package:flutter/material.dart';

void main() {
  runApp(const MtrackApp());
}

/// The app's root widget.
class MtrackApp extends StatelessWidget {
  const MtrackApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'mtrack',
      theme: ThemeData(
        brightness: Brightness.dark,
        colorSchemeSeed: Colors.deepOrange,
        useMaterial3: true,
      ),
      home: const _PlaceholderScreen(),
    );
  }
}

/// The screen shown until the connection screens exist.
class _PlaceholderScreen extends StatelessWidget {
  const _PlaceholderScreen();

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('mtrack', style: textTheme.displayMedium),
            const SizedBox(height: 8),
            Text('not connected', style: textTheme.titleMedium),
          ],
        ),
      ),
    );
  }
}
