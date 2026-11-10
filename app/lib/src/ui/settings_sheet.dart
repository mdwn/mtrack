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

import '../app/session.dart';
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
          if (session.ambientMode != null) ...[
            _AmbientCard(session: session),
            const SizedBox(height: 22),
          ],
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

/// What the ambient light is doing, in a word or two; null when unknown.
String? ambientStateLabel(String? state) => switch (state) {
  'playing' => 'playing',
  'pending' => 'waiting',
  'off' => 'off',
  'unavailable' => 'no ambient show',
  _ => null,
};

const List<(String, String)> _ambientModes = [
  ('auto', 'Auto'),
  ('on', 'On'),
  ('off', 'Off'),
];

class _AmbientCard extends StatelessWidget {
  const _AmbientCard({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final state = ambientStateLabel(session.ambientState);
    final usable = session.isReady && !session.isBusy(ActionKind.setAmbient);
    return Container(
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      decoration: BoxDecoration(
        color: colors.page,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Text(
                'Ambient light',
                style: AppText.bodyStrong.copyWith(color: colors.text),
              ),
              const SizedBox(width: 10),
              if (state != null)
                Expanded(
                  child: Text(
                    state,
                    textAlign: TextAlign.end,
                    style: AppText.small.copyWith(color: colors.muted),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 10),
          Opacity(
            opacity: usable ? 1 : 0.5,
            child: Container(
              padding: const EdgeInsets.all(3),
              decoration: BoxDecoration(
                color: colors.raised,
                borderRadius: BorderRadius.circular(10),
              ),
              child: Row(
                children: [
                  for (final (mode, label) in _ambientModes)
                    Expanded(
                      child: _Segment(
                        label: label,
                        selected: session.ambientMode == mode,
                        onTap: usable
                            ? () => session.setAmbientMode(mode)
                            : null,
                      ),
                    ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Semantics(
      button: true,
      selected: selected,
      enabled: onTap != null,
      label: 'Ambient light $label',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: selected ? colors.cyan : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 48),
            alignment: Alignment.center,
            child: Text(
              label,
              style: AppText.bodyStrong.copyWith(
                color: selected ? colors.page : colors.muted,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
