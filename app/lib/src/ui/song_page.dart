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
import '../app/song_edit.dart';
import 'format.dart';
import 'player_bar.dart';
import 'playlist_dialogs.dart';
import 'playlist_settings_sheet.dart';
import 'session_scope.dart';
import 'settings_sheet.dart';
import 'song_sheets.dart';
import 'theme.dart';
import 'widgets.dart';

/// A song's page: what it has, then its looping, tracks and samples to
/// change, staged here and sent with one Save.
class SongPage extends StatefulWidget {
  /// The page of the song [name].
  const SongPage({required this.name, super.key});

  /// The song's name.
  final String name;

  @override
  State<SongPage> createState() => _SongPageState();
}

class _SongPageState extends State<SongPage> {
  SongEdit? _edit;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_edit == null) {
      final edit = SessionScope.read(context).editSong(widget.name);
      _edit = edit;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) {
          edit.load();
        }
      });
    }
  }

  @override
  void dispose() {
    _edit?.dispose();
    super.dispose();
  }

  Future<void> _leave() async {
    final edit = _edit!;
    if (edit.saving) {
      return;
    }
    final navigator = Navigator.of(context);
    if (edit.dirty &&
        !await confirmDiscard(
          context,
          unchanged: 'The song on the player has not been changed.',
        )) {
      return;
    }
    edit.revert();
    navigator.pop();
  }

  @override
  Widget build(BuildContext context) {
    final session = SessionScope.of(context);
    final edit = _edit!;
    return ListenableBuilder(
      listenable: edit,
      builder: (context, _) {
        final song = edit.song;
        return PopScope(
          canPop: !edit.dirty && !edit.saving,
          onPopInvokedWithResult: (didPop, _) {
            if (!didPop) {
              _leave();
            }
          },
          child: Scaffold(
            // Its fields are in sheets, which keep clear of the keyboard
            // themselves; the page under them stays as it is.
            resizeToAvoidBottomInset: false,
            body: SafeArea(
              bottom: false,
              child: Column(
                children: [
                  PlayerBar(
                    title: 'Song',
                    trailing: song != null && song.hasDuration()
                        ? formatClock(song.duration.toDart())
                        : null,
                  ),
                  const ReconnectBanner(),
                  Padding(
                    padding: const EdgeInsets.fromLTRB(6, 6, 18, 0),
                    child: Row(
                      children: [
                        IconButton(
                          tooltip: 'Back',
                          icon: const Icon(Icons.arrow_back),
                          onPressed: _leave,
                        ),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Semantics(
                                header: true,
                                child: Text(
                                  widget.name,
                                  maxLines: 2,
                                  overflow: TextOverflow.ellipsis,
                                  style: AppText.heading.copyWith(
                                    color: context.colors.text,
                                  ),
                                ),
                              ),
                              // Its folder under the songs directory, where
                              // its files are.
                              if (song != null && song.baseDir.isNotEmpty)
                                Semantics(
                                  label: 'Folder ${song.baseDir}',
                                  excludeSemantics: true,
                                  child: Text(
                                    song.baseDir,
                                    maxLines: 2,
                                    overflow: TextOverflow.ellipsis,
                                    style: AppText.small.copyWith(
                                      color: context.colors.dim,
                                    ),
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                  // Above the list, so it is seen wherever the list is.
                  _Problem(edit: edit),
                  Expanded(child: _body(context, session, edit)),
                  // Not while there is nothing it could do: a song that
                  // cannot be changed, or a locked player with nothing
                  // staged.
                  if (_offersSave(session, edit))
                    _SaveBar(session: session, edit: edit)
                  else
                    SizedBox(height: MediaQuery.viewPaddingOf(context).bottom),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  static bool _offersSave(Session session, SongEdit edit) =>
      edit.song != null &&
      edit.cannotEdit == null &&
      (session.locked != true || edit.dirty);

  Widget _body(BuildContext context, Session session, SongEdit edit) {
    final colors = context.colors;
    final song = edit.song;
    if (song == null) {
      final failure = edit.loadFailure;
      return Center(
        child: failure == null || edit.loading
            ? const CircularProgressIndicator()
            : Padding(
                padding: const EdgeInsets.all(18),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      'Could not load the song: $failure',
                      textAlign: TextAlign.center,
                      style: AppText.body.copyWith(color: colors.muted),
                    ),
                    TextButton(
                      onPressed: edit.load,
                      child: const Text('Try again'),
                    ),
                  ],
                ),
              ),
      );
    }
    final locked = session.locked == true;
    final playing = session.playing && session.status?.songName == widget.name;
    final enabled = edit.editable;
    final cut = edit.looping.cut;
    return ListView(
      padding: const EdgeInsets.fromLTRB(18, 4, 18, 18),
      children: [
        if (song.hasFailure())
          _Note(
            color: colors.red,
            text:
                '${song.failure}\n\nThis song failed to load and will not '
                'play until it is fixed.',
          ),
        if (edit.cannotEdit != null)
          _Note(color: colors.amber, text: edit.cannotEdit!),
        if (locked && edit.cannotEdit == null)
          _Note(
            color: colors.amber,
            text: lockedSongNote,
            action: TextButton(
              onPressed: () => showSettingsSheet(context),
              child: const Text('Unlock'),
            ),
          ),
        if (playing) _Note(color: colors.cyan, text: playingSongNote),
        // What the song has that the phone does not change.
        const SheetEyebrow('THIS SONG'),
        _Overview(edit: edit),
        const SheetEyebrow('LOOPING'),
        _ToggleRow(
          title: 'Loop Playback',
          hint:
              'Song will repeat from the beginning when it finishes, with '
              'audio crossfade at the loop boundary.',
          value: edit.looping.loopPlayback,
          onChanged: enabled ? edit.setLoopPlayback : null,
        ),
        const SizedBox(height: 8),
        _SettingRow(
          title: 'Loop crossfade',
          value: [
            edit.looping.crossfade.isEmpty
                ? 'none, passes back to back'
                : edit.looping.crossfade,
            if (cut.isNotEmpty) 'switch at the midpoint: ${cut.join(', ')}',
          ].join(' · '),
          onTap: enabled ? () => showLoopingSheet(context, edit) : null,
        ),
        const SheetEyebrow('TRACKS'),
        if (edit.tracks.isEmpty) const _Empty('No tracks configured'),
        for (var i = 0; i < edit.tracks.length; i++)
          _SettingRow(
            title: edit.tracks[i].name,
            value: trackSummary(edit.tracks[i]),
            onTap: enabled ? () => showTrackSheet(context, edit, i) : null,
          ),
        if (enabled)
          ChunkyButton(
            height: 48,
            fontSize: 14,
            label: '+ ADD TRACK',
            semanticLabel: 'Add track',
            onPressed: () => showTrackSheet(context, edit, null),
          ),
        const SheetEyebrow('SAMPLES'),
        Text(
          'Samples configured here override the global samples config for '
          'this song only.',
          style: AppText.small.copyWith(color: colors.muted, height: 1.4),
        ),
        const SizedBox(height: 8),
        if (edit.samples.isEmpty) const _Empty('No per-song sample overrides.'),
        for (var i = 0; i < edit.samples.length; i++)
          _SettingRow(
            title: edit.samples[i].name,
            value: sampleSummary(edit.samples[i]),
            onTap: enabled ? () => showSampleSheet(context, edit, i) : null,
          ),
        if (enabled)
          ChunkyButton(
            height: 48,
            fontSize: 14,
            label: '+ ADD SAMPLE',
            semanticLabel: 'Add sample',
            onPressed: () => showSampleSheet(context, edit, null),
          ),
        if (song.triggers.isNotEmpty) ...[
          const SizedBox(height: 14),
          Text(
            'Triggers',
            style: AppText.bodyStrong.copyWith(color: colors.text),
          ),
          for (final trigger in song.triggers)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                '${trigger.event} → ${trigger.sample}',
                style: AppText.body.copyWith(color: colors.text),
              ),
            ),
          const SizedBox(height: 4),
          Text(
            'Triggers are not changed from the phone; they are set in the '
            "song's file.",
            style: AppText.small.copyWith(color: colors.muted, height: 1.4),
          ),
        ],
      ],
    );
  }
}

/// Discard and Save, clear of the system's navigation bar.
class _SaveBar extends StatelessWidget {
  const _SaveBar({required this.session, required this.edit});

  final Session session;
  final SongEdit edit;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return SafeArea(
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
                  label: 'DISCARD',
                  semanticLabel: 'Discard changes',
                  onPressed: edit.dirty && !edit.saving ? edit.revert : null,
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
    );
  }
}

/// What stands in the way of Save, and what to do about it.
class _Problem extends StatelessWidget {
  const _Problem({required this.edit});

  final SongEdit edit;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final (message, actions) = switch (edit.problem) {
      SongProblem.none => (null, <Widget>[]),
      SongProblem.changed => (
        songChangedMessage,
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
      // Said once already, above, while the player is locked.
      SongProblem.locked => (null, <Widget>[]),
      SongProblem.refused => (
        'The player would not save this: ${edit.refusal}',
        <Widget>[],
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

/// What the song has that is shown, not changed.
class _Overview extends StatelessWidget {
  const _Overview({required this.edit});

  final SongEdit edit;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final song = edit.song!;
    final lines = song.hasOverview()
        ? overviewLines(song.overview)
        : const <String>[];
    return NoteBox(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (lines.isEmpty)
            Text(
              'No tempo map, sections, metronome, pilot, MIDI or light shows.',
              style: AppText.body.copyWith(color: colors.text),
            ),
          for (final line in lines)
            Text(line, style: AppText.body.copyWith(color: colors.text)),
          const SizedBox(height: 6),
          Text(
            'Shown, not changed from the phone: change these in the web UI.',
            style: AppText.small.copyWith(color: colors.muted, height: 1.4),
          ),
        ],
      ),
    );
  }
}

class _Note extends StatelessWidget {
  const _Note({required this.color, required this.text, this.action});

  final Color color;
  final String text;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: NoteBox(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              text,
              style: AppText.small.copyWith(color: color, height: 1.45),
            ),
            ?action,
          ],
        ),
      ),
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 8),
    child: Text(
      text,
      style: AppText.body.copyWith(color: context.colors.muted),
    ),
  );
}

/// A setting's name and value on a row that opens it; inert when [onTap]
/// is null.
class _SettingRow extends StatelessWidget {
  const _SettingRow({
    required this.title,
    required this.value,
    required this.onTap,
  });

  final String title;
  final String value;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: RowCard(
        onTap: onTap,
        semanticLabel: '$title: $value',
        semanticHint: onTap == null ? null : 'Opens it to change',
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: AppText.bodyStrong.copyWith(color: colors.text),
                  ),
                  Text(
                    value,
                    maxLines: 3,
                    overflow: TextOverflow.ellipsis,
                    style: AppText.small.copyWith(color: colors.muted),
                  ),
                ],
              ),
            ),
            if (onTap != null) Icon(Icons.chevron_right, color: colors.dim),
          ],
        ),
      ),
    );
  }
}

/// A switch with its name and what it does; off limits when [onChanged] is
/// null.
class _ToggleRow extends StatelessWidget {
  const _ToggleRow({
    required this.title,
    required this.hint,
    required this.value,
    required this.onChanged,
  });

  final String title;
  final String hint;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return RowCard(
      padding: const EdgeInsets.fromLTRB(14, 4, 4, 4),
      child: SwitchListTile(
        contentPadding: EdgeInsets.zero,
        title: Text(
          title,
          style: AppText.bodyStrong.copyWith(color: colors.text),
        ),
        subtitle: Text(
          hint,
          style: AppText.small.copyWith(color: colors.muted, height: 1.4),
        ),
        value: value,
        onChanged: onChanged,
      ),
    );
  }
}
