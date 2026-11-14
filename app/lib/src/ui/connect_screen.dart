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

import '../app/player_link.dart';
import '../app/ports.dart';
import '../app/session.dart';
import '../app/wifi_address.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// The screen that finds a player, connects and pairs. It is shown while no
/// player is remembered, and while a connection to one is being made for the
/// first time.
class ConnectScreen extends StatefulWidget {
  /// The connect screen.
  const ConnectScreen({super.key});

  @override
  State<ConnectScreen> createState() => _ConnectScreenState();
}

class _ConnectScreenState extends State<ConnectScreen> {
  bool _scanned = false;
  final TextEditingController _address = TextEditingController();
  bool _entering = false;
  String? _problem;

  @override
  void dispose() {
    _address.dispose();
    super.dispose();
  }

  void _connectToAddress(Session session) {
    final WifiAddress address;
    try {
      address = parseWifiAddress(_address.text);
    } on FormatException catch (e) {
      setState(() => _problem = e.message);
      return;
    }
    setState(() => _problem = null);
    session.connectToAddress(address);
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final phase = session.phase;
    final name = session.playerName ?? 'the player';
    final connected = phase == ConnectionPhase.ready;

    final Widget content;
    switch (phase) {
      case ConnectionPhase.noPlayer:
        if (!_scanned) {
          _scanned = true;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) {
              session.startScan(keepError: true);
            }
          });
        }
        content = _Finder(
          session: session,
          address: _AddressEntry(
            controller: _address,
            entering: _entering,
            problem: _problem,
            onOpen: () => setState(() => _entering = true),
            onChanged: () {
              if (_problem != null) {
                setState(() => _problem = null);
              }
            },
            onConnect: () => _connectToAddress(session),
          ),
        );
      case ConnectionPhase.connecting:
      case ConnectionPhase.lost:
        _scanned = false;
        content = _Progress(
          title: phase == ConnectionPhase.lost
              ? 'Looking for $name…'
              : 'Connecting to $name…',
          action: phase == ConnectionPhase.lost
              ? _Action('Choose another player', session.forget)
              : _Action('Cancel', session.cancelConnecting),
        );
      case ConnectionPhase.pairing:
        _scanned = false;
        content = _Progress(
          title: 'Pairing with $name…',
          detail: 'Your phone may show a code. Confirm it.',
          action: _Action('Cancel', session.cancelConnecting),
        );
      case ConnectionPhase.pairingRefused:
        _scanned = false;
        content = _Refused(session: session);
      case ConnectionPhase.ready:
        content = const SizedBox.shrink();
    }

    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 16, 18, 10),
          child: Row(
            children: [
              Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: connected ? colors.green : colors.dim,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                'Not connected',
                style: AppText.bodyStrong.copyWith(color: colors.text),
              ),
            ],
          ),
        ),
        Expanded(child: content),
      ],
    );
  }
}

class _Action {
  const _Action(this.label, this.run);

  final String label;
  final Future<void> Function() run;
}

class _Progress extends StatelessWidget {
  const _Progress({required this.title, this.detail, required this.action});

  final String title;
  final String? detail;
  final _Action action;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return ScrollingColumn(
      children: [
        Text(
          'Find your player',
          style: AppText.heading.copyWith(color: colors.text),
        ),
        const SizedBox(height: 24),
        Row(
          children: [
            const SizedBox(
              width: 20,
              height: 20,
              child: CircularProgressIndicator(strokeWidth: 2.5),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Semantics(
                liveRegion: true,
                child: Text(
                  title,
                  style: AppText.bodyStrong.copyWith(
                    color: colors.text,
                    fontSize: 16,
                  ),
                ),
              ),
            ),
          ],
        ),
        if (detail != null) ...[
          const SizedBox(height: 14),
          NoteBox(child: Text(detail!)),
        ],
        const Spacer(),
        ChunkyButton(
          height: 52,
          fontSize: 16,
          label: action.label,
          semanticLabel: action.label,
          onPressed: action.run,
        ),
      ],
    );
  }
}

class _Refused extends StatelessWidget {
  const _Refused({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final name = session.playerName ?? 'This player';
    return ScrollingColumn(
      children: [
        Text(
          'This player is not accepting new phones',
          style: AppText.heading.copyWith(color: colors.text),
        ),
        const SizedBox(height: 16),
        NoteBox(
          child: Text.rich(
            TextSpan(
              children: [
                TextSpan(text: '$name is not taking new phones right now. '),
                const TextSpan(
                  text:
                      "Open the player's web page, go to Config → Controllers "
                      'and press ',
                ),
                TextSpan(
                  text: 'Pair a phone',
                  style: TextStyle(
                    color: colors.text,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const TextSpan(text: ', then try again here.'),
              ],
            ),
          ),
        ),
        const Spacer(),
        ChunkyButton(
          height: 52,
          fontSize: 16,
          label: 'Try again',
          semanticLabel: 'Try again',
          fill: colors.surface,
          borderColor: colors.cyan,
          foreground: colors.cyan,
          onPressed: session.retryNow,
        ),
        const SizedBox(height: 10),
        ChunkyButton(
          height: 52,
          fontSize: 16,
          label: 'Choose another player',
          semanticLabel: 'Choose another player',
          onPressed: session.cancelConnecting,
        ),
      ],
    );
  }
}

class _Finder extends StatelessWidget {
  const _Finder({required this.session, required this.address});

  final Session session;
  final Widget address;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final adapter = session.adapter;
    return ScrollingColumn(
      children: [
        Text(
          'Find your player',
          style: AppText.heading.copyWith(color: colors.text),
        ),
        const SizedBox(height: 14),
        if (session.connectError != null) ...[
          NoteBox(
            tint: colors.loopTint,
            child: Text(
              session.connectError!,
              style: AppText.body.copyWith(color: colors.text),
            ),
          ),
          const SizedBox(height: 12),
        ],
        _Results(session: session),
        if (adapter != AdapterState.ready) ...[
          const SizedBox(height: 14),
          _AdapterProblem(session: session, adapter: adapter),
        ],
        const SizedBox(height: 14),
        address,
        const SizedBox(height: 14),
        const NoteBox(
          child: Text.rich(
            TextSpan(
              children: [
                TextSpan(
                  text: 'First time? ',
                  style: TextStyle(fontWeight: FontWeight.w700),
                ),
                TextSpan(
                  text:
                      'A new player accepts its first phone for five minutes '
                      "after it starts. For another phone, open the player's "
                      'web page, go to Config → Controllers and press ',
                ),
                TextSpan(
                  text: 'Pair a phone',
                  style: TextStyle(fontWeight: FontWeight.w700),
                ),
                TextSpan(text: '.'),
              ],
            ),
          ),
        ),
        const Spacer(),
        const SizedBox(height: 14),
        const NoteBox(
          child: Text('Your phone may show a code while pairing. Confirm it.'),
        ),
      ],
    );
  }
}

class _AdapterProblem extends StatelessWidget {
  const _AdapterProblem({required this.session, required this.adapter});

  final Session session;
  final AdapterState adapter;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final String title;
    final String detail;
    switch (adapter) {
      case AdapterState.off:
        title = 'Bluetooth is off';
        detail = "Turn on Bluetooth in your phone's settings, then try again.";
      case AdapterState.permissionDenied:
        title = 'mtrack needs Bluetooth';
        detail =
            'Allow mtrack to use Bluetooth so it can find your player. It '
            'does not use your location.';
      case AdapterState.unsupported:
        title = 'No Bluetooth Low Energy';
        detail = 'This phone cannot find a player over Bluetooth.';
      case AdapterState.ready:
        title = '';
        detail = '';
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          title,
          style: AppText.bodyStrong.copyWith(color: colors.text, fontSize: 17),
        ),
        const SizedBox(height: 6),
        Text(detail, style: AppText.body.copyWith(color: colors.muted)),
        const SizedBox(height: 14),
        if (adapter != AdapterState.unsupported)
          ChunkyButton(
            height: 52,
            fontSize: 16,
            label: adapter == AdapterState.permissionDenied
                ? 'Allow Bluetooth'
                : 'Try again',
            semanticLabel: adapter == AdapterState.permissionDenied
                ? 'Allow Bluetooth'
                : 'Try again',
            onPressed: adapter == AdapterState.permissionDenied
                ? session.requestAccess
                : session.startScan,
          ),
      ],
    );
  }
}

class _Results extends StatelessWidget {
  const _Results({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final wifi = session.wifiFound;
    final found = session.found;
    final empty = wifi.isEmpty && found.isEmpty;
    final searching = session.searching;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final player in wifi)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: _WifiRow(session: session, player: player),
          ),
        for (final player in found)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: _BluetoothRow(session: session, player: player),
          ),
        if (session.browseFailed) ...[
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: Text(
              'Could not search the network.',
              style: AppText.body.copyWith(color: colors.muted),
            ),
          ),
        ],
        if (searching)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: Row(
              children: [
                const SizedBox(
                  width: 16,
                  height: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
                const SizedBox(width: 12),
                Text(
                  'Searching…',
                  style: AppText.body.copyWith(color: colors.muted),
                ),
              ],
            ),
          )
        else if (empty) ...[
          Text(
            'No players found. Make sure the player is on, and that your '
            'phone is on its network or close by.',
            style: AppText.body.copyWith(color: colors.muted),
          ),
          const SizedBox(height: 12),
          ChunkyButton(
            height: 52,
            fontSize: 16,
            label: 'Look again',
            semanticLabel: 'Look again',
            onPressed: session.startScan,
          ),
        ] else
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(
              onPressed: session.startScan,
              child: Text(
                'Look again',
                style: AppText.bodyStrong.copyWith(color: colors.cyan),
              ),
            ),
          ),
      ],
    );
  }
}

/// A player found on the network: its name, "Wi-Fi" and its first address.
/// A tap connects, with no pairing.
class _WifiRow extends StatelessWidget {
  const _WifiRow({required this.session, required this.player});

  final Session session;
  final FoundPlayer player;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final small = AppText.time(11, weight: 500).copyWith(color: colors.muted);
    final hosts = player.orderedHosts;
    final address = hosts.isEmpty ? null : hosts.first;
    return RowCard(
      onTap: () => session.connectToFound(player),
      semanticLabel:
          '${player.name}, Wi-Fi${address == null ? '' : ', $address'}',
      semanticHint: 'Connect to this player',
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  player.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: AppText.row.copyWith(color: colors.text),
                ),
                if (address != null)
                  Text(
                    address,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: small,
                  ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          Text('Wi-Fi', style: small),
        ],
      ),
    );
  }
}

/// A player found by Bluetooth: its name, "Bluetooth" and its signal. A tap
/// connects and pairs if need be.
class _BluetoothRow extends StatelessWidget {
  const _BluetoothRow({required this.session, required this.player});

  final Session session;
  final DiscoveredPlayer player;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final small = AppText.time(11, weight: 500).copyWith(color: colors.muted);
    return RowCard(
      onTap: () => session.connectTo(player),
      semanticLabel:
          '${player.name}, Bluetooth'
          '${player.signalLabel.isEmpty ? '' : ', ${player.signalLabel} signal'}',
      semanticHint: 'Connect to this player',
      child: Row(
        children: [
          Expanded(
            child: Text(
              player.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: AppText.row.copyWith(color: colors.text),
            ),
          ),
          const SizedBox(width: 8),
          Column(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Text('Bluetooth', style: small),
              if (player.signalLabel.isNotEmpty)
                Text(player.signalLabel, style: small),
            ],
          ),
        ],
      ),
    );
  }
}

/// "Enter an address": the way to a player over Wi-Fi when it has not been
/// paired by Bluetooth, or Bluetooth is not to be had.
class _AddressEntry extends StatelessWidget {
  const _AddressEntry({
    required this.controller,
    required this.entering,
    required this.problem,
    required this.onOpen,
    required this.onChanged,
    required this.onConnect,
  });

  final TextEditingController controller;
  final bool entering;
  final String? problem;
  final VoidCallback onOpen;
  final VoidCallback onChanged;
  final VoidCallback onConnect;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    if (!entering) {
      // A row of buttons: "Scan this network" goes beside this one.
      return Row(
        children: [
          Expanded(
            child: ChunkyButton(
              height: 52,
              fontSize: 16,
              label: 'Enter an address',
              semanticLabel: 'Enter an address',
              fill: colors.surface,
              borderColor: colors.cyan,
              foreground: colors.cyan,
              onPressed: onOpen,
            ),
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: controller,
          autofocus: true,
          autocorrect: false,
          enableSuggestions: false,
          keyboardType: TextInputType.url,
          textInputAction: TextInputAction.go,
          onSubmitted: (_) => onConnect(),
          onChanged: (_) => onChanged(),
          decoration: InputDecoration(
            labelText: 'Player address',
            hintText: 'mtrack.local',
            helperText:
                'A name or number, such as mtrack.local or 192.168.1.40. '
                'Add :port if the player uses a port other than 43234.',
            helperMaxLines: 3,
            errorText: problem,
            errorMaxLines: 3,
          ),
        ),
        const SizedBox(height: 12),
        ChunkyButton(
          height: 52,
          fontSize: 16,
          label: 'Connect',
          semanticLabel: 'Connect to this address',
          onPressed: onConnect,
        ),
      ],
    );
  }
}
