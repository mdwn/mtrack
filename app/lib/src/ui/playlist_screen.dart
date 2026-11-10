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
import 'format.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// The Playlist tab: the songs, the current one marked, and the way to another
/// playlist.
class PlaylistScreen extends StatefulWidget {
  /// The Playlist tab. [active] is true while the tab is the one on show.
  const PlaylistScreen({required this.active, super.key});

  /// Whether this tab is showing.
  final bool active;

  @override
  State<PlaylistScreen> createState() => _PlaylistScreenState();
}

class _PlaylistScreenState extends State<PlaylistScreen> {
  final ScrollController _scroll = ScrollController();
  int? _revealed;
  double _extent = 64;

  @override
  void initState() {
    super.initState();
    _revealSoon(center: true);
  }

  @override
  void didUpdateWidget(PlaylistScreen old) {
    super.didUpdateWidget(old);
    if (widget.active && !old.active) {
      _revealSoon(center: true);
    }
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  void _revealSoon({required bool center}) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        _reveal(center: center);
      }
    });
  }

  /// Scrolls so the current song is on screen: centred when the tab opens,
  /// the least that shows it when it changes under the person's eyes.
  void _reveal({required bool center}) {
    final at = SessionScope.read(context).position;
    if (at == null || !_scroll.hasClients) {
      return;
    }
    final position = _scroll.position;
    final top = at * _extent;
    final view = position.viewportDimension;
    double? target;
    if (center) {
      target = top - (view - _extent) / 2;
    } else if (top < position.pixels) {
      target = top;
    } else if (top + _extent > position.pixels + view) {
      target = top + _extent - view;
    }
    if (target != null) {
      _scroll.jumpTo(target.clamp(0.0, position.maxScrollExtent));
    }
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final entries = session.entries;
    final scaler = MediaQuery.textScalerOf(context);
    _extent = scaler.scale(15) * 1.5 + 20 + 8;
    if (session.position != _revealed) {
      _revealed = session.position;
      if (widget.active) {
        _revealSoon(center: false);
      }
    }

    var total = Duration.zero;
    for (final entry in entries) {
      if (entry.hasDuration()) {
        total += entry.duration.toDart();
      }
    }
    final summary = entries.isEmpty
        ? null
        : '${entries.length} ${entries.length == 1 ? 'song' : 'songs'}'
              '${total > Duration.zero ? ' · ${formatTotal(total)}' : ''}';
    final playing = session.playing;
    final inert = !session.isReady || playing;

    return Column(
      children: [
        PlayerBar(trailing: summary),
        const ReconnectBanner(),
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 8, 18, 10),
          child: _PlaylistPick(session: session),
        ),
        if (playing)
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 0, 18, 8),
            child: Text(
              'Stop the song to choose another.',
              style: AppText.small.copyWith(color: colors.dim),
            ),
          ),
        Expanded(
          child: entries.isEmpty
              ? Center(
                  child: Text(
                    session.isReady ? 'This playlist is empty' : '',
                    style: AppText.body.copyWith(color: colors.muted),
                  ),
                )
              : ListView.builder(
                  controller: _scroll,
                  padding: const EdgeInsets.fromLTRB(18, 0, 18, 12),
                  itemExtent: _extent,
                  itemCount: entries.length,
                  itemBuilder: (context, index) => Padding(
                    padding: const EdgeInsets.only(bottom: 8),
                    child: _SongRow(
                      index: index,
                      name: entries[index].songName,
                      duration: entries[index].hasDuration()
                          ? entries[index].duration.toDart()
                          : null,
                      current: index == session.position,
                      inert: inert,
                      onTap:
                          inert ||
                              index == session.position ||
                              session.isBusy(ActionKind.navigate)
                          ? null
                          : () => session.navigateTo(
                              index,
                              entries[index].songName,
                            ),
                    ),
                  ),
                ),
        ),
      ],
    );
  }
}

class _SongRow extends StatelessWidget {
  const _SongRow({
    required this.index,
    required this.name,
    required this.duration,
    required this.current,
    required this.inert,
    required this.onTap,
  });

  final int index;
  final String name;
  final Duration? duration;
  final bool current;
  final bool inert;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final length = duration == null ? '' : formatClock(duration!);
    return RowCard(
      tint: current ? colors.currentTint : null,
      mark: current ? colors.cyan : null,
      enabled: !inert || current,
      onTap: onTap,
      semanticLabel:
          '${index + 1}, $name${length.isEmpty ? '' : ', $length'}'
          '${current ? ', current song' : ''}',
      semanticHint: onTap == null ? null : 'Make this the current song',
      child: Row(
        children: [
          SizedBox(
            width: 26,
            child: Text(
              '${index + 1}',
              style: AppText.time(12, weight: 500).copyWith(color: colors.dim),
            ),
          ),
          Expanded(
            child: Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: (current ? AppText.rowCurrent : AppText.row).copyWith(
                color: colors.text,
              ),
            ),
          ),
          const SizedBox(width: 8),
          Text(
            length,
            style: AppText.time(
              12.5,
              weight: 500,
            ).copyWith(color: colors.muted),
          ),
        ],
      ),
    );
  }
}

class _PlaylistPick extends StatelessWidget {
  const _PlaylistPick({required this.session});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final name = session.status?.playlistName ?? '';
    final enabled = session.isReady && !session.playing;
    return Semantics(
      button: true,
      enabled: enabled,
      label: 'Playlist $name',
      hint: 'Change playlist',
      excludeSemantics: true,
      onTap: enabled ? () => _open(context) : null,
      child: Opacity(
        opacity: enabled ? 1 : 0.5,
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(
            borderRadius: BorderRadius.circular(12),
            onTap: enabled ? () => _open(context) : null,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(12),
                border: Border.all(color: colors.line, width: 1.5),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      name.isEmpty ? 'No playlist' : name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: AppText.bodyStrong.copyWith(color: colors.text),
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(
                    'change',
                    style: AppText.small.copyWith(color: colors.dim),
                  ),
                  Icon(Icons.arrow_drop_down, color: colors.dim),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _open(BuildContext context) {
    final scoped = SessionScope.read(context);
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (sheet) =>
          SessionScope(session: scoped, child: const PlaylistPicker()),
    );
  }
}

/// The sheet that lists the player's playlists and switches to the one tapped.
class PlaylistPicker extends StatefulWidget {
  /// A picker. Fetches the list of playlists when it opens.
  const PlaylistPicker({super.key});

  @override
  State<PlaylistPicker> createState() => _PlaylistPickerState();
}

class _PlaylistPickerState extends State<PlaylistPicker> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        SessionScope.read(context).loadPlaylists();
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final colors = context.colors;
    final choices = session.choices;
    return Padding(
      padding: const EdgeInsets.fromLTRB(18, 18, 18, 8),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Choose a playlist',
            style: AppText.heading.copyWith(color: colors.text),
          ),
          const SizedBox(height: 12),
          if (choices == null)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 24),
              child: Center(
                child: session.loadingChoices
                    ? const CircularProgressIndicator()
                    : Text(
                        'Could not load the playlists.',
                        style: AppText.body.copyWith(color: colors.muted),
                      ),
              ),
            )
          else
            Flexible(
              child: ListView(
                shrinkWrap: true,
                children: [
                  for (final name in choices.names)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 8),
                      child: RowCard(
                        tint: name == choices.active
                            ? colors.currentTint
                            : null,
                        mark: name == choices.active ? colors.cyan : null,
                        semanticLabel: name == choices.active
                            ? '$name, active'
                            : name,
                        onTap: session.isBusy(ActionKind.switchPlaylist)
                            ? null
                            : () async {
                                final navigator = Navigator.of(context);
                                final failure = await session.switchPlaylist(
                                  name,
                                );
                                if (failure == null) {
                                  navigator.pop();
                                }
                              },
                        child: Text(
                          name,
                          style:
                              (name == choices.active
                                      ? AppText.rowCurrent
                                      : AppText.row)
                                  .copyWith(color: colors.text),
                        ),
                      ),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}
