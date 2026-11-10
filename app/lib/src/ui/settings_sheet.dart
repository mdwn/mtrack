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
import 'package:package_info_plus/package_info_plus.dart';

import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// Where the app's version comes from. Tests replace it.
Future<String> Function() appVersion = () async {
  final info = await PackageInfo.fromPlatform();
  return '${info.version} (${info.buildNumber})';
};

/// Opens the settings: the connected player, forgetting it, the licences and
/// the app's version.
Future<void> showSettingsSheet(BuildContext context) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    builder: (sheet) => SessionScope(
      session: SessionScope.read(context),
      child: const _SettingsSheet(),
    ),
  );
}

class _SettingsSheet extends StatelessWidget {
  const _SettingsSheet();

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final name = session.playerName;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(18, 18, 18, 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('Settings', style: AppText.heading.copyWith(color: colors.text)),
          const SizedBox(height: 16),
          Text('PLAYER', style: AppText.eyebrow.copyWith(color: colors.dim)),
          const SizedBox(height: 6),
          Text(
            name ?? 'No player',
            style: AppText.bodyStrong.copyWith(
              color: colors.text,
              fontSize: 17,
            ),
          ),
          Text(
            session.isReady ? 'Connected' : 'Not connected',
            style: AppText.small.copyWith(color: colors.muted),
          ),
          const SizedBox(height: 14),
          if (name != null)
            ChunkyButton(
              height: 52,
              fontSize: 16,
              label: 'Forget this player',
              semanticLabel: 'Forget this player',
              borderColor: colors.red,
              foreground: colors.red,
              onPressed: () async {
                Navigator.of(context).pop();
                await session.forget();
              },
            ),
          const SizedBox(height: 22),
          Text('ABOUT', style: AppText.eyebrow.copyWith(color: colors.dim)),
          const SizedBox(height: 6),
          FutureBuilder<String>(
            future: appVersion(),
            builder: (context, snapshot) => Text(
              'mtrack remote ${snapshot.data ?? ''}'.trim(),
              style: AppText.body.copyWith(color: colors.muted),
            ),
          ),
          const SizedBox(height: 10),
          ChunkyButton(
            height: 52,
            fontSize: 16,
            label: 'Open-source licences',
            semanticLabel: 'Open-source licences',
            onPressed: () =>
                showLicensePage(context: context, applicationName: 'mtrack'),
          ),
        ],
      ),
    );
  }
}
