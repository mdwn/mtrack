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

import '../app/session.dart';
import 'session_scope.dart';
import 'settings_sheet.dart';
import 'theme.dart';

/// The bar across the top of the tabs: the player's name with a light showing
/// whether the link is up, [trailing] text about the screen, and the way to
/// the settings.
class PlayerBar extends StatelessWidget {
  /// A bar with [trailing] at its right.
  const PlayerBar({this.trailing, super.key});

  /// What this screen wants said: the playlist, the song, a count.
  final String? trailing;

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final up = session.phase == ConnectionPhase.ready;
    final name = session.playerName ?? 'mtrack';
    return Padding(
      padding: const EdgeInsets.fromLTRB(18, 8, 6, 0),
      child: Row(
        children: [
          Semantics(
            label: up ? 'Connected' : 'Not connected',
            child: Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: up ? colors.green : colors.dim,
                boxShadow: up
                    ? [
                        BoxShadow(
                          color: colors.green.withAlpha(50),
                          spreadRadius: 3,
                        ),
                      ]
                    : null,
              ),
            ),
          ),
          const SizedBox(width: 8),
          Flexible(
            flex: 2,
            child: Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: AppText.bodyStrong.copyWith(color: colors.text),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            flex: 3,
            child: Text(
              trailing ?? '',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.end,
              style: AppText.small.copyWith(color: colors.muted),
            ),
          ),
          IconButton(
            tooltip: 'Settings',
            icon: Icon(Icons.settings_outlined, color: colors.muted),
            onPressed: () => showSettingsSheet(context),
          ),
        ],
      ),
    );
  }
}

/// A thin banner shown while the link is down: the last state is still on
/// screen, dimmed, and the app is trying to get back.
class ReconnectBanner extends StatelessWidget {
  /// The banner, shown only when the session is not ready.
  const ReconnectBanner({super.key});

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    if (session.isReady) {
      return const SizedBox.shrink();
    }
    final colors = context.colors;
    return Semantics(
      liveRegion: true,
      child: Container(
        width: double.infinity,
        color: colors.loopTint,
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 6),
        child: Row(
          children: [
            SizedBox(
              width: 12,
              height: 12,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: colors.amber,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                'Reconnecting…',
                style: AppText.small.copyWith(color: colors.amber),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
