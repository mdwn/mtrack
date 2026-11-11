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

import 'package:flutter/material.dart';

import '../app/ports.dart';
import 'player_bar.dart';
import 'playlist_screen.dart';
import 'session_scope.dart';
import 'songs_screen.dart';
import 'widgets.dart';

/// The two sides of the Library tab.
enum LibrarySide {
  /// The playlist playing from.
  playlist,

  /// Every song on the player.
  songs,
}

/// Where the side last shown is kept across launches.
const String libraryViewKey = 'library.view';

/// The Library tab: a switch at the top between the playlist and every song
/// on the player, as the Lights tab switches between the plot and 3D. Each
/// side keeps its place while the other shows, and the side last shown is
/// the one a launch opens on.
class LibraryScreen extends StatefulWidget {
  /// The tab. [active] is true while it is the one on show; [views] keeps
  /// the side last shown.
  const LibraryScreen({required this.active, required this.views, super.key});

  /// Whether this tab is showing.
  final bool active;

  /// Where the side last shown is kept.
  final ViewStore views;

  @override
  State<LibraryScreen> createState() => _LibraryScreenState();
}

class _LibraryScreenState extends State<LibraryScreen> {
  LibrarySide _shown = LibrarySide.playlist;

  /// Whether the person has chosen a side since the launch, so the kept one
  /// arriving late does not undo it.
  bool _chosen = false;

  @override
  void initState() {
    super.initState();
    unawaited(_recall());
  }

  Future<void> _recall() async {
    final kept = await widget.views.read(libraryViewKey);
    final side = LibrarySide.values.where((s) => s.name == kept).firstOrNull;
    if (mounted && !_chosen && side != null && side != _shown) {
      setState(() => _shown = side);
    }
  }

  void _choose(LibrarySide side) {
    // The search keeps no keyboard up over the playlist.
    FocusManager.instance.primaryFocus?.unfocus();
    _chosen = true;
    setState(() => _shown = side);
    unawaited(widget.views.write(libraryViewKey, side.name));
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final library = session.songLibrary;
    final songs = _shown == LibrarySide.songs;
    return Column(
      children: [
        ListenableBuilder(
          listenable: library,
          builder: (context, _) => PlayerBar(
            trailing: songs
                ? (library.songs == null
                      ? null
                      : '${library.songs!.length} songs')
                : playlistSummary(session),
          ),
        ),
        const ReconnectBanner(),
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 8, 18, 0),
          child: ViewSwitch<LibrarySide>(
            semanticLabel: 'Playlist or songs',
            choices: const [
              (LibrarySide.playlist, 'Playlist'),
              (LibrarySide.songs, 'Songs'),
            ],
            selected: _shown,
            onChanged: _choose,
          ),
        ),
        Expanded(
          // Both stay built, each where it was left; the one not showing
          // is still, and out of reach.
          child: Stack(
            fit: StackFit.expand,
            children: [
              _side(
                showing: !songs,
                child: PlaylistView(active: widget.active, showing: !songs),
              ),
              _side(
                showing: songs,
                child: SongsView(showing: widget.active && songs),
              ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _side({required bool showing, required Widget child}) => Offstage(
    offstage: !showing,
    child: TickerMode(enabled: showing && widget.active, child: child),
  );
}
