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

import '../../gen/player/v1/player.pb.dart' as player;
import '../app/song_edit.dart';
import 'format.dart';
import 'player_bar.dart';
import 'session_scope.dart';
import 'song_page.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens the song library over the tabs.
Future<void> openSongs(BuildContext context) {
  final session = SessionScope.read(context);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) =>
          SessionScope(session: session, child: const SongsScreen()),
    ),
  );
}

/// Opens the page of the song [name].
Future<void> openSong(BuildContext context, String name) {
  final session = SessionScope.read(context);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => SessionScope(
        session: session,
        child: SongPage(name: name),
      ),
    ),
  );
}

/// Every song on the player, with a search; a tap opens a song's page.
class SongsScreen extends StatefulWidget {
  /// The screen. Fetches the library when it opens.
  const SongsScreen({super.key});

  @override
  State<SongsScreen> createState() => _SongsScreenState();
}

class _SongsScreenState extends State<SongsScreen> {
  final TextEditingController _query = TextEditingController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        // Fetched each time it opens: songs change on the player.
        SessionScope.read(context).songLibrary.load();
      }
    });
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final library = session.songLibrary;
    final colors = context.colors;
    return Scaffold(
      body: SafeArea(
        child: ListenableBuilder(
          listenable: library,
          builder: (context, _) {
            final songs = library.songs;
            final shown = songs == null
                ? <player.SongListing>[]
                : filterListings(songs, _query.text);
            return Column(
              children: [
                PlayerBar(
                  title: 'Songs',
                  trailing: songs == null ? null : '${songs.length} songs',
                ),
                const ReconnectBanner(),
                Padding(
                  padding: const EdgeInsets.fromLTRB(6, 8, 18, 8),
                  child: Row(
                    children: [
                      IconButton(
                        tooltip: 'Back',
                        icon: const Icon(Icons.arrow_back),
                        onPressed: () => Navigator.of(context).maybePop(),
                      ),
                      Expanded(
                        child: TextField(
                          controller: _query,
                          onChanged: (_) => setState(() {}),
                          enabled: !library.tooOld,
                          decoration: InputDecoration(
                            hintText: songs == null
                                ? 'Search'
                                : 'Search ${songs.length} songs',
                            prefixIcon: const Icon(Icons.search),
                            isDense: true,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                Expanded(
                  child: library.tooOld
                      ? ScrollingColumn(
                          children: [
                            NoteBox(
                              child: Text(
                                olderPlayerSongsNote,
                                style: AppText.small.copyWith(
                                  color: colors.amber,
                                  height: 1.45,
                                ),
                              ),
                            ),
                          ],
                        )
                      : songs == null
                      ? Center(
                          child: library.failure != null && !library.loading
                              ? Column(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    Text(
                                      'Could not load the songs.',
                                      style: AppText.body.copyWith(
                                        color: colors.muted,
                                      ),
                                    ),
                                    TextButton(
                                      onPressed: library.load,
                                      child: const Text('Try again'),
                                    ),
                                  ],
                                )
                              : const CircularProgressIndicator(),
                        )
                      : shown.isEmpty
                      ? Center(
                          child: Text(
                            songs.isEmpty
                                ? 'The player has no songs.'
                                : 'No song matches.',
                            style: AppText.body.copyWith(color: colors.muted),
                          ),
                        )
                      : RefreshIndicator(
                          onRefresh: library.load,
                          child: ListView.builder(
                            padding: const EdgeInsets.fromLTRB(18, 0, 18, 12),
                            itemCount: shown.length,
                            itemBuilder: (context, i) => _SongListingRow(
                              song: shown[i],
                              current:
                                  shown[i].name == session.status?.songName,
                              onTap: () => openSong(context, shown[i].name),
                            ),
                          ),
                        ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _SongListingRow extends StatelessWidget {
  const _SongListingRow({
    required this.song,
    required this.current,
    required this.onTap,
  });

  final player.SongListing song;
  final bool current;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final flags = listingFlags(song);
    final length = song.hasDuration()
        ? formatClock(song.duration.toDart())
        : '';
    final failed = song.hasFailure();
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: RowCard(
        tint: current ? colors.currentTint : null,
        mark: failed ? colors.red : (current ? colors.cyan : null),
        onTap: onTap,
        semanticLabel: [
          song.name,
          if (current) 'current song',
          if (length.isNotEmpty) length,
          if (flags.isNotEmpty) flags,
        ].join(', '),
        semanticHint: 'Opens the song',
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    song.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: (current ? AppText.rowCurrent : AppText.row)
                        .copyWith(color: colors.text),
                  ),
                  if (flags.isNotEmpty)
                    Text(
                      flags,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: AppText.small.copyWith(
                        color: failed ? colors.red : colors.muted,
                      ),
                    ),
                ],
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
            Icon(Icons.chevron_right, color: colors.dim),
          ],
        ),
      ),
    );
  }
}
