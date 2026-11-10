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
import '../app/failure.dart';
import '../app/playlist_edit.dart';
import '../app/session.dart';
import 'format.dart';
import 'player_bar.dart';
import 'playlist_dialogs.dart';
import 'session_scope.dart';
import 'theme.dart';
import 'widgets.dart';

/// Leaves the editor, asking first when there is something to lose.
Future<void> leaveEditor(BuildContext context) async {
  final session = SessionScope.read(context);
  final edit = session.editing;
  if (edit == null || edit.saving) {
    return;
  }
  if (edit.hasChanges && !await confirmDiscard(context)) {
    return;
  }
  session.endEditing();
}

/// The editor of a playlist: the songs in a list to drag, remove and add to,
/// and a Save that sends them to the player. It covers the tabs, so a stray
/// tap cannot leave half an edit.
class PlaylistEditor extends StatelessWidget {
  /// The editor of the session's playlist edit.
  const PlaylistEditor({super.key});

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final edit = session.editing;
    if (edit == null) {
      return const SizedBox.shrink();
    }
    return ListenableBuilder(
      listenable: edit,
      builder: (context, _) => _EditorBody(session: session, edit: edit),
    );
  }
}

class _EditorBody extends StatelessWidget {
  const _EditorBody({required this.session, required this.edit});

  final Session session;
  final PlaylistEdit edit;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final rows = edit.rows;
    final at = edit.isNew ? null : session.position;
    return Column(
      children: [
        PlayerBar(
          title: 'Editing',
          trailing: formatSongs(rows.length, edit.total),
        ),
        const ReconnectBanner(),
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 6, 6, 0),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  edit.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: AppText.bodyStrong.copyWith(color: colors.text),
                ),
              ),
              if (edit.isNew)
                TextButton(
                  onPressed: edit.saving ? null : () => _rename(context),
                  child: const Text('Rename'),
                ),
            ],
          ),
        ),
        _Problem(edit: edit, onRename: () => _rename(context)),
        Expanded(
          child: ReorderableListView.builder(
            buildDefaultDragHandles: false,
            padding: const EdgeInsets.fromLTRB(18, 8, 18, 12),
            itemCount: rows.length,
            onReorderItem: edit.saving ? (_, _) {} : edit.move,
            proxyDecorator: (child, _, _) =>
                Material(type: MaterialType.transparency, child: child),
            header: rows.isEmpty
                ? Padding(
                    padding: const EdgeInsets.symmetric(vertical: 18),
                    child: Text(
                      'No songs yet.',
                      textAlign: TextAlign.center,
                      style: AppText.body.copyWith(color: colors.muted),
                    ),
                  )
                : null,
            footer: Padding(
              padding: const EdgeInsets.only(top: 2),
              child: ChunkyButton(
                height: 52,
                fontSize: 15,
                label: '+ ADD SONGS',
                semanticLabel: 'Add songs',
                onPressed: session.isReady && !edit.saving
                    ? () => openAddSongs(context)
                    : null,
              ),
            ),
            itemBuilder: (context, index) {
              final row = rows[index];
              return _EditRowTile(
                key: ValueKey(row.id),
                index: index,
                row: row,
                current: at != null && row.originalIndex == at,
                locked: edit.saving,
                onRemove: () => edit.removeAt(index),
              );
            },
          ),
        ),
        // The tab bar is hidden here, and it was what kept the bottom clear
        // of the system's navigation bar.
        SafeArea(
          top: false,
          minimum: const EdgeInsets.fromLTRB(18, 4, 18, 12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!session.isReady)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Text(
                    notConnectedMessage,
                    textAlign: TextAlign.center,
                    style: AppText.small.copyWith(color: colors.amber),
                  ),
                ),
              Row(
                children: [
                  Expanded(
                    child: ChunkyButton(
                      height: 56,
                      fontSize: 16,
                      label: 'CANCEL',
                      semanticLabel: 'Cancel',
                      onPressed: edit.saving
                          ? null
                          : () => leaveEditor(context),
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: ChunkyButton(
                      height: 56,
                      fontSize: 16,
                      label: edit.saving ? 'SAVING…' : 'SAVE',
                      semanticLabel: edit.saving ? 'Saving' : 'Save',
                      fill: colors.play,
                      foreground: colors.onPlay,
                      borderColor: colors.shadow,
                      onPressed: edit.canSave ? () => edit.save() : null,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ],
    );
  }

  Future<void> _rename(BuildContext context) => askPlaylistName(
    context,
    title: 'Playlist name',
    action: 'Rename',
    initial: edit.name,
    accept: edit.rename,
  );
}

/// What stands in the way of Save, and what to do about it.
class _Problem extends StatelessWidget {
  const _Problem({required this.edit, required this.onRename});

  final PlaylistEdit edit;
  final VoidCallback onRename;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final (message, actions) = switch (edit.problem) {
      EditProblem.none => (null, <Widget>[]),
      EditProblem.changed => (
        playlistChangedMessage,
        [
          Expanded(
            child: ChunkyButton(
              height: 48,
              fontSize: 13,
              label: 'SAVE ANYWAY',
              semanticLabel: 'Save anyway',
              onPressed: edit.saving ? null : () => edit.saveAnyway(),
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: ChunkyButton(
              height: 48,
              fontSize: 13,
              label: 'DISCARD MY CHANGES',
              semanticLabel: 'Discard my changes',
              onPressed: edit.saving ? null : () => edit.discardAndReload(),
            ),
          ),
        ],
      ),
      EditProblem.locked => (lockedToSaveMessage, <Widget>[]),
      EditProblem.exists => (
        nameTakenMessage,
        [
          Expanded(
            child: ChunkyButton(
              height: 48,
              fontSize: 13,
              label: 'CHANGE NAME',
              semanticLabel: 'Change name',
              onPressed: edit.saving ? null : onRename,
            ),
          ),
        ],
      ),
    };
    if (message == null) {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(18, 6, 18, 0),
      child: Semantics(
        liveRegion: true,
        child: NoteBox(
          tint: colors.loopTint,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(message, style: AppText.body.copyWith(color: colors.text)),
              if (actions.isNotEmpty) ...[
                const SizedBox(height: 10),
                Row(children: actions),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _EditRowTile extends StatelessWidget {
  const _EditRowTile({
    required this.index,
    required this.row,
    required this.current,
    required this.locked,
    required this.onRemove,
    super.key,
  });

  final int index;
  final EditRow row;
  final bool current;
  final bool locked;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: RowCard(
        tint: current ? colors.currentTint : null,
        mark: current ? colors.cyan : null,
        padding: const EdgeInsets.only(left: 2, right: 2),
        child: Row(
          children: [
            ReorderableDragStartListener(
              index: index,
              enabled: !locked,
              child: Semantics(
                container: true,
                label: 'Reorder ${row.name}',
                child: SizedBox(
                  width: 48,
                  height: 48,
                  child: Icon(Icons.drag_indicator, color: colors.dim),
                ),
              ),
            ),
            Expanded(
              child: Semantics(
                container: true,
                label: current ? '${row.name}, current song' : row.name,
                excludeSemantics: true,
                child: Text(
                  row.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: (current ? AppText.rowCurrent : AppText.row).copyWith(
                    color: colors.text,
                  ),
                ),
              ),
            ),
            if (row.duration != null) ...[
              const SizedBox(width: 8),
              Text(
                formatClock(row.duration!),
                style: AppText.time(
                  12.5,
                  weight: 500,
                ).copyWith(color: colors.muted),
              ),
            ],
            IconButton(
              tooltip: 'Remove ${row.name}',
              constraints: const BoxConstraints(minWidth: 48, minHeight: 48),
              icon: Icon(Icons.close, color: colors.muted),
              onPressed: locked ? null : onRemove,
            ),
          ],
        ),
      ),
    );
  }
}

/// Opens the full-screen list of songs to add.
Future<void> openAddSongs(BuildContext context) {
  final session = SessionScope.read(context);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) =>
          SessionScope(session: session, child: const AddSongsScreen()),
    ),
  );
}

/// Every song on the player, with a search, to choose from and add to the end
/// of the list being edited.
class AddSongsScreen extends StatefulWidget {
  /// The screen, for the session's playlist edit.
  const AddSongsScreen({super.key});

  @override
  State<AddSongsScreen> createState() => _AddSongsScreenState();
}

class _AddSongsScreenState extends State<AddSongsScreen> {
  final TextEditingController _query = TextEditingController();
  final List<String> _chosen = [];

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        final edit = SessionScope.read(context).editing;
        if (edit != null && edit.library == null) {
          edit.loadLibrary();
        }
      }
    });
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  void _toggle(String name) {
    setState(() {
      if (!_chosen.remove(name)) {
        _chosen.add(name);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final edit = session.editing;
    final colors = context.colors;
    if (edit == null) {
      return const Scaffold();
    }
    return Scaffold(
      body: SafeArea(
        child: ListenableBuilder(
          listenable: edit,
          builder: (context, _) {
            final library = edit.library;
            final shown = library == null
                ? <player.PlaylistEntry>[]
                : filterSongs(library, _query.text);
            return Column(
              children: [
                PlayerBar(
                  title: 'Add songs',
                  trailing: _chosen.isEmpty ? null : '${_chosen.length} chosen',
                ),
                const ReconnectBanner(),
                Padding(
                  padding: const EdgeInsets.fromLTRB(18, 8, 18, 8),
                  child: TextField(
                    controller: _query,
                    onChanged: (_) => setState(() {}),
                    decoration: InputDecoration(
                      hintText: library == null
                          ? 'Search'
                          : 'Search ${library.length} songs',
                      prefixIcon: const Icon(Icons.search),
                      isDense: true,
                    ),
                  ),
                ),
                Expanded(
                  child: library == null
                      ? Center(
                          child: edit.libraryFailed
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
                                      onPressed: edit.loadLibrary,
                                      child: const Text('Try again'),
                                    ),
                                  ],
                                )
                              : const CircularProgressIndicator(),
                        )
                      : shown.isEmpty
                      ? Center(
                          child: Text(
                            'No song matches.',
                            style: AppText.body.copyWith(color: colors.muted),
                          ),
                        )
                      : ListView.builder(
                          padding: const EdgeInsets.fromLTRB(18, 0, 18, 12),
                          itemCount: shown.length,
                          itemBuilder: (context, i) => _SongChoice(
                            name: shown[i].songName,
                            duration: shown[i].hasDuration()
                                ? shown[i].duration.toDart()
                                : null,
                            inPlaylist: edit.contains(shown[i].songName),
                            chosen: _chosen.contains(shown[i].songName),
                            onTap: () => _toggle(shown[i].songName),
                          ),
                        ),
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(18, 4, 18, 12),
                  child: Row(
                    children: [
                      Expanded(
                        child: ChunkyButton(
                          height: 56,
                          fontSize: 16,
                          label: 'CANCEL',
                          semanticLabel: 'Cancel',
                          onPressed: () => Navigator.of(context).pop(),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        flex: 2,
                        child: ChunkyButton(
                          height: 56,
                          fontSize: 16,
                          label: _chosen.isEmpty
                              ? 'ADD SONGS'
                              : 'ADD ${_chosen.length} '
                                    '${_chosen.length == 1 ? 'SONG' : 'SONGS'}',
                          semanticLabel: _chosen.isEmpty
                              ? 'Add songs'
                              : 'Add ${_chosen.length} '
                                    '${_chosen.length == 1 ? 'song' : 'songs'}',
                          fill: colors.play,
                          foreground: colors.onPlay,
                          borderColor: colors.shadow,
                          onPressed: _chosen.isEmpty
                              ? null
                              : () {
                                  edit.addSongs(List.of(_chosen));
                                  Navigator.of(context).pop();
                                },
                        ),
                      ),
                    ],
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

class _SongChoice extends StatelessWidget {
  const _SongChoice({
    required this.name,
    required this.duration,
    required this.inPlaylist,
    required this.chosen,
    required this.onTap,
  });

  final String name;
  final Duration? duration;
  final bool inPlaylist;
  final bool chosen;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: RowCard(
        // Already in the list is a hint, not a ban: a song may be there twice.
        enabled: !inPlaylist || chosen,
        tint: chosen ? colors.currentTint : null,
        mark: chosen ? colors.cyan : null,
        onTap: onTap,
        semanticLabel:
            '$name${inPlaylist ? ', in playlist' : ''}'
            ', ${chosen ? 'chosen' : 'not chosen'}',
        semanticHint: chosen ? 'Tap to unchoose' : 'Tap to choose',
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 13),
        child: Row(
          children: [
            Icon(
              chosen ? Icons.check_box : Icons.check_box_outline_blank,
              color: chosen ? colors.cyan : colors.dim,
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: AppText.row.copyWith(color: colors.text),
              ),
            ),
            const SizedBox(width: 8),
            Text(
              inPlaylist
                  ? 'in playlist'
                  : (duration == null ? '' : formatClock(duration!)),
              style: AppText.time(
                12.5,
                weight: 500,
              ).copyWith(color: colors.muted),
            ),
          ],
        ),
      ),
    );
  }
}
