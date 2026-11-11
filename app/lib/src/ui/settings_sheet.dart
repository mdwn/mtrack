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
import 'hold_to_stop.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// Where the app's version comes from. Tests replace it.
Future<String> Function() appVersion = () async {
  final info = await PackageInfo.fromPlatform();
  return '${info.version} (${info.buildNumber})';
};

/// Opens the settings: the player's lock, the connected player, forgetting
/// it, the licences and the app's version.
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

/// Why some of the app is not there: the player is older than the app. An
/// app with fewer features and no reason given looks like a broken app.
String olderPlayerNote(List<String> missing) {
  final list = missing.length == 1
      ? missing.single
      : '${missing.sublist(0, missing.length - 1).join(', ')} '
            'and ${missing.last}';
  return 'This player runs an older mtrack, so $list '
      '${missing.length == 1 ? 'is' : 'are'} not shown. '
      'Update mtrack on the player to use '
      '${missing.length == 1 ? 'it' : 'them'}.';
}

class _SettingsSheet extends StatelessWidget {
  const _SettingsSheet();

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final name = session.playerName;
    // The sheet's own safe area leaves the bottom alone, so the last button
    // would sit under the system's navigation bar.
    final below = MediaQuery.viewPaddingOf(context).bottom;
    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(18, 18, 18, 24 + below),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('Settings', style: AppText.heading.copyWith(color: colors.text)),
          const SizedBox(height: 16),
          if (session.lockAvailable) ...[
            const _LockCard(),
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
          if (session.missingFromPlayer.isNotEmpty) ...[
            const SizedBox(height: 8),
            Text(
              olderPlayerNote(session.missingFromPlayer),
              style: AppText.small.copyWith(color: colors.amber),
            ),
          ],
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

/// The player's lock: a tap locks it, and unlocking takes a hold, as stopping
/// does, so a brushed screen cannot let anyone change the playlists.
class _LockCard extends StatefulWidget {
  const _LockCard();

  @override
  State<_LockCard> createState() => _LockCardState();
}

class _LockCardState extends State<_LockCard> {
  bool _tapped = false;

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final locked = session.locked ?? true;
    final usable = session.isReady && !session.isBusy(ActionKind.setLock);
    return Container(
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 14),
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
                'Player lock',
                style: AppText.bodyStrong.copyWith(color: colors.text),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  locked ? 'locked' : 'unlocked',
                  textAlign: TextAlign.end,
                  style: AppText.small.copyWith(
                    color: locked ? colors.muted : colors.amber,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 6),
          Text(
            locked
                ? 'While locked, playlists and settings cannot be changed from '
                      'any screen. Playing, levels and ambient light still '
                      'work.'
                : 'The player stays unlocked until someone locks it. Lock it '
                      'before the show.',
            style: AppText.small.copyWith(color: colors.muted, height: 1.45),
          ),
          const SizedBox(height: 12),
          if (locked)
            HoldButton(
              label: _tapped ? 'KEEP HOLDING' : 'HOLD TO UNLOCK',
              fontSize: 15,
              height: 52,
              enabled: usable,
              semanticLabel: 'Hold to unlock the player',
              semanticHint: 'Press and hold to unlock',
              fill: colors.surface,
              progress: colors.amber.withAlpha(90),
              foreground: colors.amber,
              borderColor: colors.amber,
              onComplete: () {
                setState(() => _tapped = false);
                session.setLock(false);
              },
              onEarlyRelease: () => setState(() => _tapped = true),
            )
          else
            ChunkyButton(
              height: 52,
              fontSize: 15,
              label: 'LOCK',
              semanticLabel: 'Lock the player',
              onPressed: usable ? () => session.setLock(true) : null,
            ),
        ],
      ),
    );
  }
}
