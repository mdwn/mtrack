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

import '../app/failure.dart';
import '../app/session.dart';
import '../protocol/player_channel.dart';
import 'session_scope.dart';
import 'settings_sheet.dart';
import 'theme.dart';

/// The bar across the top of the tabs: the player's name with a light showing
/// whether the link is up, [trailing] text about the screen, the player's
/// lock, and the way to the settings.
class PlayerBar extends StatelessWidget {
  /// A bar with [trailing] at its right. [title] takes the place of the
  /// player's name, for a screen that is not one of the tabs.
  const PlayerBar({this.trailing, this.title, super.key});

  /// What to call the screen, instead of the player.
  final String? title;

  /// What this screen wants said: the playlist, the song, a count.
  final String? trailing;

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final up = session.phase == ConnectionPhase.ready;
    final carrier = session.carrier;
    final over = carrier == null ? '' : ' over ${carrierName(carrier)}';
    final name = title ?? session.playerName ?? 'mtrack';
    return Padding(
      padding: const EdgeInsets.fromLTRB(18, 8, 6, 0),
      child: Row(
        children: [
          Semantics(
            label: up ? 'Connected$over' : 'Not connected',
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
          if (carrier != null) ...[
            const SizedBox(width: 6),
            Icon(
              carrier == Carrier.wifi ? Icons.wifi : Icons.bluetooth,
              size: 15,
              color: colors.muted,
              semanticLabel: carrierName(carrier),
            ),
          ],
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
          if (session.lockAvailable) ...[
            const SizedBox(width: 8),
            LockPill(locked: session.locked!),
          ],
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

/// The player's lock as a small label in the top bar: muted when locked,
/// amber when unlocked, as a reminder that anyone can change things. Tapping
/// it opens the settings, where the lock is.
class LockPill extends StatelessWidget {
  /// A pill for a player that is [locked] or not.
  const LockPill({required this.locked, super.key});

  /// Whether the player is locked.
  final bool locked;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final color = locked ? colors.muted : colors.amber;
    return Semantics(
      button: true,
      label: locked ? 'Player locked' : 'Player unlocked',
      hint: 'Open settings',
      excludeSemantics: true,
      onTap: () => showSettingsSheet(context),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: () => showSettingsSheet(context),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Center(
            widthFactor: 1,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
              decoration: BoxDecoration(
                color: locked ? null : colors.loopTint,
                borderRadius: BorderRadius.circular(12),
                border: Border.all(color: locked ? colors.line : color),
              ),
              child: Text(
                locked ? 'Locked' : 'Unlocked',
                maxLines: 1,
                style: AppText.small.copyWith(
                  color: color,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          ),
        ),
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
