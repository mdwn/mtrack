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
import '../app/playlist_edit.dart';
import '../app/playlist_settings.dart';
import 'theme.dart';
import 'widgets.dart';

/// Said under each sheet's title: nothing is sent until the playlist is.
const String sheetSavesWithPlaylist =
    'Changes reach the player when you save the playlist.';

/// Opens the settings of [row] in [edit]: its fades, auto play and ambient
/// light. Done puts them in the edit; Cancel leaves it as it was.
Future<void> showEntrySettings(
  BuildContext context,
  PlaylistEdit edit,
  EditRow row,
) {
  edit.loadLightShows();
  return _showSheet(
    context,
    (_) => EntrySettingsSheet(
      song: row.name,
      initial: row.settings,
      edit: edit,
      onDone: (settings) {
        if (settings != row.settings) {
          edit.setSettings(row.id, settings);
        }
      },
    ),
  );
}

/// Opens the playlist's own ambient light in [edit].
Future<void> showPlaylistAmbient(BuildContext context, PlaylistEdit edit) {
  edit.loadLightShows();
  final initial = edit.ambient;
  return _showSheet(
    context,
    (_) => PlaylistAmbientSheet(
      name: edit.name,
      initial: initial,
      edit: edit,
      onDone: (ambient) {
        if (ambient != initial) {
          edit.setAmbient(ambient);
        }
      },
    ),
  );
}

Future<void> _showSheet(BuildContext context, WidgetBuilder builder) =>
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: builder,
    );

/// A sheet's frame: its content scrolling above Cancel and Done, clear of
/// the keyboard and of the system's navigation bar.
class _SheetFrame extends StatelessWidget {
  const _SheetFrame({
    required this.title,
    required this.children,
    required this.onDone,
  });

  final String title;
  final List<Widget> children;
  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    // The sheet's own safe area leaves the bottom alone, so the buttons
    // would sit under the navigation bar; the keyboard comes up over it.
    final below =
        MediaQuery.viewPaddingOf(context).bottom +
        MediaQuery.viewInsetsOf(context).bottom;
    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(18, 18, 18, 24 + below),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(title, style: AppText.heading.copyWith(color: colors.text)),
          const SizedBox(height: 4),
          Text(
            sheetSavesWithPlaylist,
            style: AppText.small.copyWith(color: colors.muted),
          ),
          ...children,
          const SizedBox(height: 22),
          Row(
            children: [
              Expanded(
                child: ChunkyButton(
                  height: 52,
                  fontSize: 15,
                  label: 'CANCEL',
                  semanticLabel: 'Cancel',
                  onPressed: () => Navigator.of(context).pop(),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: ChunkyButton(
                  height: 52,
                  fontSize: 15,
                  label: 'DONE',
                  semanticLabel: 'Done',
                  fill: colors.play,
                  foreground: colors.onPlay,
                  borderColor: colors.shadow,
                  onPressed: () {
                    onDone();
                    Navigator.of(context).pop();
                  },
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// A section's small uppercase heading.
class _Eyebrow extends StatelessWidget {
  const _Eyebrow(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 22, bottom: 4),
    child: Text(
      text,
      style: AppText.eyebrow.copyWith(color: context.colors.dim),
    ),
  );
}

/// A setting's name and what it does.
class _Label extends StatelessWidget {
  const _Label(this.title, this.hint);

  final String title;
  final String hint;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(top: 14, bottom: 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(title, style: AppText.bodyStrong.copyWith(color: colors.text)),
          const SizedBox(height: 2),
          Text(
            hint,
            style: AppText.small.copyWith(color: colors.muted, height: 1.4),
          ),
        ],
      ),
    );
  }
}

/// A text field for a length or a time, which the player reads.
class _LengthField extends StatelessWidget {
  const _LengthField({
    required this.controller,
    required this.label,
    required this.placeholder,
    required this.helper,
    required this.onChanged,
    this.suffix,
  });

  final TextEditingController controller;
  final String label;
  final String placeholder;
  final String? helper;
  final ValueChanged<String> onChanged;
  final Widget? suffix;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: TextField(
        controller: controller,
        onChanged: onChanged,
        autocorrect: false,
        enableSuggestions: false,
        decoration: InputDecoration(
          labelText: label,
          hintText: placeholder,
          helperText: helper,
          helperMaxLines: 3,
          isDense: true,
          suffixIcon: suffix,
        ),
      ),
    );
  }
}

/// A song's settings in a playlist: auto play, ambient light and fade in
/// before it, its fade out after, as the web UI's transitions panel has
/// them.
class EntrySettingsSheet extends StatefulWidget {
  /// The sheet for [song], showing [initial]; [onDone] gets the settings
  /// as they are when Done is pressed.
  const EntrySettingsSheet({
    required this.song,
    required this.initial,
    required this.edit,
    required this.onDone,
    super.key,
  });

  /// The song's name.
  final String song;

  /// Its settings when the sheet opened.
  final player.EntrySettings initial;

  /// The edit, for the player's light shows.
  final PlaylistEdit edit;

  /// Called with the settings on Done.
  final ValueChanged<player.EntrySettings> onDone;

  @override
  State<EntrySettingsSheet> createState() => _EntrySettingsSheetState();
}

class _EntrySettingsSheetState extends State<EntrySettingsSheet> {
  late final player.Fade? _initialOut = widget.initial.hasFadeOut()
      ? widget.initial.fadeOut
      : null;
  late final player.Fade? _initialIn = widget.initial.hasFadeIn()
      ? widget.initial.fadeIn
      : null;
  late player.Fade? _fadeOut = _initialOut;
  late player.Fade? _fadeIn = _initialIn;
  late bool _autoPlay = widget.initial.hasAutoPlay();
  late player.AmbientSetting? _ambient = widget.initial.hasAmbient()
      ? widget.initial.ambient
      : null;
  late final TextEditingController _fadeOutText = TextEditingController(
    text: _fadeOut?.length ?? '',
  );
  late final TextEditingController _fadeInText = TextEditingController(
    text: _fadeIn?.length ?? '',
  );
  late final TextEditingController _gap = TextEditingController(
    text: _autoPlay ? autoPlayGap(widget.initial.autoPlay) : '',
  );

  @override
  void dispose() {
    _fadeOutText.dispose();
    _fadeInText.dispose();
    _gap.dispose();
    super.dispose();
  }

  player.EntrySettings _settings() => player.EntrySettings(
    fadeOut: _fadeOut,
    fadeIn: _fadeIn,
    autoPlay: _autoPlay ? player.AutoPlay(gap: _gap.text.trim()) : null,
    ambient: _ambient,
  );

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final details = fadeDetails(_fadeOut);
    return _SheetFrame(
      title: widget.song,
      onDone: () => widget.onDone(_settings()),
      children: [
        const _Eyebrow('BEFORE THIS SONG'),
        const _Label(
          'Auto play',
          'Start this song by itself when the playlist reaches it: the '
              'song before it ending, or Next/Previous while stopped.',
        ),
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          title: Text(
            _autoPlay ? 'On' : 'Off',
            style: AppText.body.copyWith(color: colors.text),
          ),
          value: _autoPlay,
          onChanged: (on) => setState(() => _autoPlay = on),
        ),
        if (_autoPlay)
          _LengthField(
            controller: _gap,
            label: 'Gap',
            placeholder: 'at once',
            helper: 'Empty starts it at once. $timeUnitsHint',
            onChanged: (_) => setState(() {}),
          ),
        _AmbientFields(
          label: 'Ambient light',
          hint: 'What the rig plays in the gap before this song.',
          inheritLabel: "The playlist's",
          inheritPlaceholder: "playlist's",
          value: _ambient,
          shows: widget.edit,
          onChanged: (ambient) => setState(() => _ambient = ambient),
        ),
        const _Label(
          'Fade in',
          'When this song starts under a song fading out. No effect '
              'otherwise.',
        ),
        _LengthField(
          controller: _fadeInText,
          label: 'Length',
          placeholder: 'none (e.g. 1measure)',
          helper: lengthUnitsHint,
          // From the fade as read, so emptying the field and typing again
          // keeps what the app does not edit.
          onChanged: (text) =>
              setState(() => _fadeIn = withFadeLength(_initialIn, text)),
        ),
        const _Eyebrow('AFTER THIS SONG'),
        const _Label(
          'Fade out',
          'Only for loop breaks: when Play, Next or Previous leaves this '
              'song while it loops, it plays on and fades out under the next '
              'one. A song that ends by itself just ends.',
        ),
        _LengthField(
          controller: _fadeOutText,
          label: 'Length',
          placeholder: 'none (e.g. 1measure)',
          helper: lengthUnitsHint,
          onChanged: (text) =>
              setState(() => _fadeOut = withFadeLength(_initialOut, text)),
        ),
        if (details != null)
          Padding(
            padding: const EdgeInsets.only(top: 10),
            child: NoteBox(
              child: Text(
                'Also set: $details. Kept as it is; change it from the '
                'web UI.',
              ),
            ),
          ),
      ],
    );
  }
}

/// A playlist's own ambient light.
class PlaylistAmbientSheet extends StatefulWidget {
  /// The sheet for the playlist [name], showing [initial] (null: the
  /// profile's); [onDone] gets the setting on Done.
  const PlaylistAmbientSheet({
    required this.name,
    required this.initial,
    required this.edit,
    required this.onDone,
    super.key,
  });

  /// The playlist's name.
  final String name;

  /// Its ambient light when the sheet opened.
  final player.AmbientSetting? initial;

  /// The edit, for the player's light shows.
  final PlaylistEdit edit;

  /// Called with the setting on Done.
  final ValueChanged<player.AmbientSetting?> onDone;

  @override
  State<PlaylistAmbientSheet> createState() => _PlaylistAmbientSheetState();
}

class _PlaylistAmbientSheetState extends State<PlaylistAmbientSheet> {
  late player.AmbientSetting? _ambient = widget.initial;

  @override
  Widget build(BuildContext context) {
    return _SheetFrame(
      title: widget.name,
      onDone: () => widget.onDone(_ambient),
      children: [
        _AmbientFields(
          label: 'Ambient light',
          hint: 'What the rig plays between songs with this playlist.',
          inheritLabel: "The profile's",
          inheritPlaceholder: "profile's",
          value: _ambient,
          shows: widget.edit,
          onChanged: (ambient) => setState(() => _ambient = ambient),
        ),
      ],
    );
  }
}

/// Ambient light set at one level: the level above's, none, or its own,
/// with its own show and timings, each left empty to take the level
/// above's. As the web UI's ambient fields.
class _AmbientFields extends StatefulWidget {
  const _AmbientFields({
    required this.label,
    required this.hint,
    required this.inheritLabel,
    required this.inheritPlaceholder,
    required this.value,
    required this.shows,
    required this.onChanged,
  });

  final String label;
  final String hint;
  final String inheritLabel;
  final String inheritPlaceholder;
  final player.AmbientSetting? value;
  final PlaylistEdit shows;
  final ValueChanged<player.AmbientSetting?> onChanged;

  @override
  State<_AmbientFields> createState() => _AmbientFieldsState();
}

class _AmbientFieldsState extends State<_AmbientFields> {
  late final Map<AmbientField, TextEditingController> _text = {
    for (final field in AmbientField.values)
      field: TextEditingController(text: ambientText(widget.value, field)),
  };

  @override
  void dispose() {
    for (final c in _text.values) {
      c.dispose();
    }
    super.dispose();
  }

  void _choose(AmbientChoice choice) {
    final next = ambientForChoice(choice, widget.value);
    // Fields shown again start from what the setting holds.
    for (final field in AmbientField.values) {
      _text[field]!.text = ambientText(next, field);
    }
    widget.onChanged(next);
  }

  void _set(AmbientField field, String text) {
    final value = widget.value;
    if (value == null || !value.on) {
      return;
    }
    widget.onChanged(withAmbientText(value, field, text));
  }

  Future<void> _pickShow() async {
    final shows = widget.shows.lightShows ?? const <String>[];
    final picked = await showModalBottomSheet<String>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (sheet) => _ShowPicker(shows: shows),
    );
    if (picked != null && mounted) {
      _text[AmbientField.show]!.text = picked;
      _set(AmbientField.show, picked);
    }
  }

  @override
  Widget build(BuildContext context) {
    final choice = ambientChoiceOf(widget.value);
    return ListenableBuilder(
      listenable: widget.shows,
      builder: (context, _) {
        final shows = widget.shows.lightShows;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Label(widget.label, widget.hint),
            Wrap(
              spacing: 8,
              runSpacing: 4,
              children: [
                for (final (option, text) in [
                  (AmbientChoice.inherit, widget.inheritLabel),
                  (AmbientChoice.off, 'None'),
                  (AmbientChoice.own, 'Its own'),
                ])
                  ChoiceChip(
                    label: Text(text),
                    selected: choice == option,
                    onSelected: (_) => _choose(option),
                  ),
              ],
            ),
            if (choice == AmbientChoice.own) ...[
              _LengthField(
                controller: _text[AmbientField.show]!,
                label: 'Show',
                placeholder: widget.inheritPlaceholder,
                helper: 'A .light file, relative to the project.',
                onChanged: (text) => _set(AmbientField.show, text),
                suffix: shows == null || shows.isEmpty
                    ? null
                    : IconButton(
                        tooltip: 'Choose a show',
                        icon: const Icon(Icons.list),
                        onPressed: _pickShow,
                      ),
              ),
              for (final (field, label) in [
                (AmbientField.delay, 'Delay'),
                (AmbientField.fadeIn, 'Fade in'),
                (AmbientField.fadeOut, 'Fade out'),
              ])
                _LengthField(
                  controller: _text[field]!,
                  label: label,
                  placeholder: widget.inheritPlaceholder,
                  helper: null,
                  onChanged: (text) => _set(field, text),
                ),
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(
                  "Delay holds the song's last frame first; fade in comes up "
                  "from whatever is lit; fade out crosses into the next "
                  "song's show. $timeUnitsHint Empty takes the "
                  '${widget.inheritPlaceholder}.',
                  style: AppText.small.copyWith(
                    color: context.colors.muted,
                    height: 1.4,
                  ),
                ),
              ),
            ],
          ],
        );
      },
    );
  }
}

/// The player's light shows, to choose one from.
class _ShowPicker extends StatelessWidget {
  const _ShowPicker({required this.shows});

  final List<String> shows;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final below = MediaQuery.viewPaddingOf(context).bottom;
    return ListView(
      shrinkWrap: true,
      padding: EdgeInsets.fromLTRB(18, 18, 18, 18 + below),
      children: [
        Text(
          'Choose a show',
          style: AppText.heading.copyWith(color: colors.text),
        ),
        const SizedBox(height: 8),
        for (final show in shows)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: RowCard(
              onTap: () => Navigator.of(context).pop(show),
              semanticLabel: show,
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 13),
              child: Text(
                show,
                style: AppText.row.copyWith(color: colors.text),
              ),
            ),
          ),
      ],
    );
  }
}
