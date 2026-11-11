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
import '../app/session.dart';
import '../app/song_edit.dart';
import 'format.dart';
import 'session_scope.dart';
import 'song_page.dart';
import 'theme.dart';
import 'widgets.dart';

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

/// The Library tab's Songs side: every song on the player under its
/// folder, as the web UI's song list has them, with a search; a tap opens a
/// song's page.
class SongsView extends StatefulWidget {
  /// The library. It is fetched each time [showing] turns true: songs
  /// change on the player.
  const SongsView({required this.showing, super.key});

  /// Whether this side of the tab is the one on show.
  final bool showing;

  @override
  State<SongsView> createState() => _SongsViewState();
}

class _SongsViewState extends State<SongsView> {
  final TextEditingController _query = TextEditingController();

  @override
  void initState() {
    super.initState();
    if (widget.showing) {
      _loadSoon();
    }
  }

  @override
  void didUpdateWidget(SongsView old) {
    super.didUpdateWidget(old);
    if (widget.showing && !old.showing) {
      _loadSoon();
    }
  }

  void _loadSoon() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
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
    return ListenableBuilder(
      listenable: library,
      builder: (context, _) {
        final songs = library.songs;
        final searching = _query.text.trim().isNotEmpty;
        return Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(18, 8, 18, 8),
              child: TextField(
                controller: _query,
                onChanged: (_) => setState(() {}),
                enabled: !library.tooOld,
                decoration: InputDecoration(
                  hintText: songs == null
                      ? 'Search'
                      : 'Search ${songs.length} songs',
                  prefixIcon: const Icon(Icons.search),
                  suffixIcon: searching
                      ? IconButton(
                          tooltip: 'Clear the search',
                          icon: const Icon(Icons.close),
                          onPressed: () => setState(_query.clear),
                        )
                      : null,
                  isDense: true,
                ),
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
                  : _list(context, session, library, songs, searching),
            ),
          ],
        );
      },
    );
  }

  /// The songs under their folders, or while searching, the songs that
  /// match, each saying its folder.
  Widget _list(
    BuildContext context,
    Session session,
    SongLibrary library,
    List<player.SongListing> songs,
    bool searching,
  ) {
    final colors = context.colors;
    final items = <Widget Function()>[];
    if (searching) {
      for (final song in filterListings(songs, _query.text)) {
        items.add(
          () => _SongListingRow(
            song: song,
            folder: songFolder(song),
            current: song.name == session.status?.songName,
            onTap: () => openSong(context, song.name),
          ),
        );
      }
    } else {
      for (final group in groupListings(songs)) {
        final collapsed = library.isCollapsed(group.folder);
        if (group.folder.isNotEmpty) {
          items.add(
            () => _FolderHeader(
              folder: group.folder,
              count: group.songs.length,
              collapsed: collapsed,
              onTap: () => library.toggleFolder(group.folder),
            ),
          );
        }
        if (collapsed && group.folder.isNotEmpty) {
          continue;
        }
        for (final song in group.songs) {
          items.add(
            () => _SongListingRow(
              song: song,
              current: song.name == session.status?.songName,
              onTap: () => openSong(context, song.name),
            ),
          );
        }
      }
    }
    if (items.isEmpty) {
      return Center(
        child: Text(
          songs.isEmpty ? 'The player has no songs.' : 'No song matches.',
          style: AppText.body.copyWith(color: colors.muted),
        ),
      );
    }
    return RefreshIndicator(
      onRefresh: library.load,
      child: ListView.builder(
        // Its own place, kept apart from the search's.
        key: PageStorageKey(searching ? 'songs-search' : 'songs-folders'),
        // The keyboard comes up over the tab: the last rows scroll up
        // above it.
        padding: EdgeInsets.fromLTRB(
          18,
          0,
          18,
          12 + MediaQuery.viewInsetsOf(context).bottom,
        ),
        itemCount: items.length,
        itemBuilder: (context, i) => items[i](),
      ),
    );
  }
}

/// A folder's name over its songs, which opens and closes it, as the web
/// UI's song list heads a folder: the path, how many songs, and an arrow
/// that points down while it is open.
class _FolderHeader extends StatelessWidget {
  const _FolderHeader({
    required this.folder,
    required this.count,
    required this.collapsed,
    required this.onTap,
  });

  final String folder;
  final int count;
  final bool collapsed;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Semantics(
      button: true,
      expanded: !collapsed,
      label: 'Folder $folder, ${count == 1 ? '1 song' : '$count songs'}',
      hint: collapsed ? 'Shows its songs' : 'Hides its songs',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 44),
            padding: const EdgeInsets.fromLTRB(0, 6, 4, 6),
            margin: const EdgeInsets.only(bottom: 4),
            child: Row(
              children: [
                Icon(
                  collapsed ? Icons.chevron_right : Icons.expand_more,
                  size: 20,
                  color: colors.dim,
                ),
                const SizedBox(width: 6),
                Expanded(
                  child: Text(
                    folder.toUpperCase(),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: AppText.eyebrow.copyWith(color: colors.dim),
                  ),
                ),
                const SizedBox(width: 8),
                Text(
                  '$count',
                  style: AppText.time(
                    11.5,
                    weight: 500,
                  ).copyWith(color: colors.dim),
                ),
              ],
            ),
          ),
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
    this.folder = '',
  });

  final player.SongListing song;

  /// The folder it is in, said under its name: in a search, where the
  /// folders are not.
  final String folder;
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
          if (folder.isNotEmpty) 'in $folder',
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
                  if (folder.isNotEmpty)
                    Text(
                      folder,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: AppText.small.copyWith(color: colors.dim),
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
