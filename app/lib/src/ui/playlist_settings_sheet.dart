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
  edit.loadSongPalettes();
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
class SheetFrame extends StatelessWidget {
  /// A sheet called [title] showing [children], with [note] under the
  /// title: by default, that nothing is sent until the playlist is saved.
  const SheetFrame({
    required this.title,
    required this.children,
    required this.onDone,
    this.note = sheetSavesWithPlaylist,
    super.key,
  });

  final String title;
  final List<Widget> children;

  /// Said under the title.
  final String note;

  /// Called on Done; false keeps the sheet open (a field has an error).
  final bool Function() onDone;

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
          Text(note, style: AppText.small.copyWith(color: colors.muted)),
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
                    if (onDone()) {
                      Navigator.of(context).pop();
                    }
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

/// Brings the first of [fields] that is on the tree into view, once the
/// error just set under it has been laid out.
void showFirstField(BuildContext context, Iterable<GlobalKey?> fields) {
  final target = fields
      .map((key) => key?.currentContext)
      .whereType<BuildContext>()
      .firstOrNull;
  if (target == null) {
    return;
  }
  WidgetsBinding.instance.addPostFrameCallback((_) {
    if (target.mounted) {
      Scrollable.ensureVisible(
        target,
        duration: const Duration(milliseconds: 200),
        alignment: 0.1,
      );
    }
  });
}

/// A section's small uppercase heading.
class SheetEyebrow extends StatelessWidget {
  /// A heading saying [text].
  const SheetEyebrow(this.text, {super.key});

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
class SheetLabel extends StatelessWidget {
  /// A label: [title], with [hint] under it.
  const SheetLabel(this.title, this.hint, {super.key});

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

/// A text field for a length, a time or a name, which the player reads.
class SheetField extends StatelessWidget {
  /// A field editing [controller].
  const SheetField({
    required this.controller,
    required this.label,
    required this.placeholder,
    required this.helper,
    required this.onChanged,
    this.suffix,
    this.errorText,
    this.fieldKey,
    this.keyboardType,
    this.enabled = true,
    super.key,
  });

  final TextEditingController controller;
  final String label;
  final String placeholder;
  final String? helper;
  final ValueChanged<String> onChanged;
  final Widget? suffix;

  /// Said in place of the helper when the text cannot be what the field
  /// takes.
  final String? errorText;

  /// Names the field, to scroll it into view.
  final GlobalKey? fieldKey;

  /// The keyboard to bring up; text by default.
  final TextInputType? keyboardType;

  /// Whether the field takes input.
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: fieldKey,
      padding: const EdgeInsets.only(top: 8),
      child: TextField(
        controller: controller,
        onChanged: onChanged,
        keyboardType: keyboardType,
        enabled: enabled,
        autocorrect: false,
        enableSuggestions: false,
        decoration: InputDecoration(
          labelText: label,
          hintText: placeholder,
          helperText: helper,
          helperMaxLines: 3,
          errorText: errorText,
          errorMaxLines: 3,
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
  late Map<String, String> _colours = widget.initial.hasPalette()
      ? Map.of(widget.initial.palette.slots)
      : {};
  late final TextEditingController _fadeOutText = TextEditingController(
    text: _fadeOut?.length ?? '',
  );
  late final TextEditingController _fadeInText = TextEditingController(
    text: _fadeIn?.length ?? '',
  );
  late final TextEditingController _gap = TextEditingController(
    text: _autoPlay ? autoPlayGap(widget.initial.autoPlay) : '',
  );
  final _gapKey = GlobalKey();
  final _fadeInKey = GlobalKey();
  final _fadeOutKey = GlobalKey();
  final _ambientKey = GlobalKey<_AmbientFieldsState>();
  bool _gapBad = false;
  bool _fadeInBad = false;
  bool _fadeOutBad = false;

  /// Checks the lengths for text that cannot be one; shows what is wrong
  /// and brings the first into view.
  bool _check() {
    setState(() {
      _gapBad = _autoPlay && !looksLikeTime(_gap.text);
      _fadeInBad = !looksLikeLength(_fadeInText.text);
      _fadeOutBad = !looksLikeLength(_fadeOutText.text);
    });
    final ambientBad = _ambientKey.currentState?.check() ?? false;
    final bad = _gapBad || ambientBad || _fadeInBad || _fadeOutBad;
    if (bad) {
      showFirstField(context, [
        if (_gapBad) _gapKey,
        if (ambientBad) _ambientKey.currentState?.firstBadKey,
        if (_fadeInBad) _fadeInKey,
        if (_fadeOutBad) _fadeOutKey,
      ]);
    }
    return !bad;
  }

  @override
  void dispose() {
    _fadeOutText.dispose();
    _fadeInText.dispose();
    _gap.dispose();
    super.dispose();
  }

  // The palette goes as it was read unless the colours were changed: a
  // sheet that never learned the song's palette must not drop it.
  player.EntrySettings _settings() => withPaletteSlots(
    player.EntrySettings(
      fadeOut: _fadeOut,
      fadeIn: _fadeIn,
      autoPlay: _autoPlay ? player.AutoPlay(gap: _gap.text.trim()) : null,
      ambient: _ambient,
      palette: widget.initial.hasPalette() ? widget.initial.palette : null,
    ),
    _colours,
  );

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final details = fadeDetails(_fadeOut);
    return SheetFrame(
      title: widget.song,
      onDone: () {
        if (!_check()) {
          return false;
        }
        widget.onDone(_settings());
        return true;
      },
      children: [
        const SheetEyebrow('BEFORE THIS SONG'),
        const SheetLabel(
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
          SheetField(
            controller: _gap,
            label: 'Gap',
            placeholder: 'at once',
            helper: 'Empty starts it at once. $timeUnitsHint',
            fieldKey: _gapKey,
            errorText: _gapBad ? notATimeMessage : null,
            onChanged: (_) => setState(() => _gapBad = false),
          ),
        _AmbientFields(
          key: _ambientKey,
          label: 'Ambient light',
          hint: 'What the rig plays in the gap before this song.',
          inheritLabel: "The playlist's",
          inheritPlaceholder: "playlist's",
          value: _ambient,
          shows: widget.edit,
          onChanged: (ambient) => setState(() => _ambient = ambient),
        ),
        const SheetLabel(
          'Fade in',
          'When this song starts under a song fading out. No effect '
              'otherwise.',
        ),
        SheetField(
          controller: _fadeInText,
          label: 'Length',
          placeholder: 'none (e.g. 1measure)',
          helper: lengthUnitsHint,
          fieldKey: _fadeInKey,
          errorText: _fadeInBad ? notALengthMessage : null,
          // From the fade as read, so emptying the field and typing again
          // keeps what the app does not edit.
          onChanged: (text) => setState(() {
            _fadeInBad = false;
            _fadeIn = withFadeLength(_initialIn, text);
          }),
        ),
        const SheetEyebrow('AFTER THIS SONG'),
        const SheetLabel(
          'Fade out',
          'Only for loop breaks: when Play, Next or Previous leaves this '
              'song while it loops, it plays on and fades out under the next '
              'one. A song that ends by itself just ends.',
        ),
        SheetField(
          controller: _fadeOutText,
          label: 'Length',
          placeholder: 'none (e.g. 1measure)',
          helper: lengthUnitsHint,
          fieldKey: _fadeOutKey,
          errorText: _fadeOutBad ? notALengthMessage : null,
          onChanged: (text) => setState(() {
            _fadeOutBad = false;
            _fadeOut = withFadeLength(_initialOut, text);
          }),
        ),
        ListenableBuilder(
          listenable: widget.edit,
          builder: (context, _) {
            final song = widget.edit.songPalettes?[widget.song];
            // Without the song's palette, and with nothing set on the
            // entry, there is nothing to show.
            if (song == null && _colours.isEmpty) {
              return const SizedBox.shrink();
            }
            if (widget.edit.songPalettes == null) {
              return const SizedBox.shrink();
            }
            return EntryColours(
              song: song,
              slots: _colours,
              onChanged: (slots) => setState(() => _colours = slots),
            );
          },
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
  final _ambientKey = GlobalKey<_AmbientFieldsState>();

  @override
  Widget build(BuildContext context) {
    return SheetFrame(
      title: widget.name,
      onDone: () {
        final fields = _ambientKey.currentState;
        if (fields != null && fields.check()) {
          showFirstField(context, [fields.firstBadKey]);
          return false;
        }
        widget.onDone(_ambient);
        return true;
      },
      children: [
        _AmbientFields(
          key: _ambientKey,
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
    super.key,
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

  final Map<AmbientField, GlobalKey> _keys = {
    for (final field in AmbientField.values) field: GlobalKey(),
  };
  final Set<AmbientField> _bad = {};

  /// The timings that cannot be times, shown as errors; whether there are
  /// any. Only an ambient of its own has any to check.
  bool check() {
    final own = ambientChoiceOf(widget.value) == AmbientChoice.own;
    setState(() {
      _bad
        ..clear()
        ..addAll(
          own
              ? [
                  for (final field in _timings)
                    if (!looksLikeTime(_text[field]!.text)) field,
                ]
              : const [],
        );
    });
    return _bad.isNotEmpty;
  }

  /// The first timing with an error, for scrolling to.
  GlobalKey? get firstBadKey {
    for (final field in _timings) {
      if (_bad.contains(field)) {
        return _keys[field];
      }
    }
    return null;
  }

  static const _timings = [
    AmbientField.delay,
    AmbientField.fadeIn,
    AmbientField.fadeOut,
  ];

  void _choose(AmbientChoice choice) {
    _bad.clear();
    final next = ambientForChoice(choice, widget.value);
    // Fields shown again start from what the setting holds.
    for (final field in AmbientField.values) {
      _text[field]!.text = ambientText(next, field);
    }
    widget.onChanged(next);
  }

  void _set(AmbientField field, String text) {
    final value = widget.value;
    if (_bad.remove(field)) {
      setState(() {});
    }
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
            SheetLabel(widget.label, widget.hint),
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
              SheetField(
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
                SheetField(
                  controller: _text[field]!,
                  label: label,
                  placeholder: widget.inheritPlaceholder,
                  helper: null,
                  fieldKey: _keys[field],
                  errorText: _bad.contains(field) ? notATimeMessage : null,
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

/// A colour as `#rrggbb` text, for painting.
Color paletteColour(String hex) {
  final digits = normalizeColour(hex)?.substring(1) ?? '000000';
  return Color(0xFF000000 | int.parse(digits, radix: 16));
}

/// A song's palette slots in an entry: each slot with the colour it has
/// without this entry's override (the song's, else the show's), and the
/// entry's own to pick or reset. A slot the song does not declare is
/// warned of and can be removed.
class EntryColours extends StatelessWidget {
  /// The section for a song with palette [song] (null: it has none),
  /// showing the entry's override [slots]; [onChanged] gets the new ones.
  const EntryColours({
    required this.song,
    required this.slots,
    required this.onChanged,
    super.key,
  });

  /// The song's palette, if it has one.
  final player.SongPalette? song;

  /// The entry's overrides, slot to `#rrggbb`.
  final Map<String, String> slots;

  /// Called with the entry's overrides after a change.
  final ValueChanged<Map<String, String>> onChanged;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final unknown = unknownSlots(song, slots);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SheetEyebrow('COLOURS'),
        Text(
          "Recolour this song's palette in this playlist only. It is fixed "
          'when the song starts.',
          style: AppText.small.copyWith(color: colors.muted, height: 1.4),
        ),
        for (final slot in song?.slots ?? const <player.PaletteSlot>[])
          _SlotRow(
            name: slot.name,
            base: baseColour(song!, slot.name)!,
            baseFrom: songOverrides(song!, slot.name) ? 'song' : 'show',
            chosen: slots[slot.name],
            onPick: (hex) => onChanged({...slots, slot.name: hex}),
            onReset: () => onChanged(Map.of(slots)..remove(slot.name)),
          ),
        for (final slot in unknown)
          _UnknownSlotRow(
            name: slot,
            colour: slots[slot]!,
            onRemove: () => onChanged(Map.of(slots)..remove(slot)),
          ),
      ],
    );
  }
}

class _Swatch extends StatelessWidget {
  const _Swatch(this.hex, {this.size = 32});

  /// Null draws an empty one.
  final String? hex;
  final double size;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: hex == null ? null : paletteColour(hex!),
        border: Border.all(color: colors.line, width: 2),
        borderRadius: BorderRadius.circular(8),
      ),
    );
  }
}

class _SlotRow extends StatelessWidget {
  const _SlotRow({
    required this.name,
    required this.base,
    required this.baseFrom,
    required this.chosen,
    required this.onPick,
    required this.onReset,
  });

  final String name;
  final String base;
  final String baseFrom;
  final String? chosen;
  final ValueChanged<String> onPick;
  final VoidCallback onReset;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Row(
        children: [
          _Swatch(base),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  name,
                  style: AppText.bodyStrong.copyWith(color: colors.text),
                ),
                Text(
                  chosen == null
                      ? '$baseFrom $base'
                      : 'was $baseFrom $base, now $chosen',
                  style: AppText.small.copyWith(color: colors.muted),
                ),
              ],
            ),
          ),
          if (chosen != null) ...[
            _Swatch(chosen),
            IconButton(
              tooltip: 'Reset $name',
              icon: const Icon(Icons.restart_alt),
              onPressed: onReset,
            ),
          ],
          IconButton(
            tooltip: 'Pick colour for $name',
            icon: const Icon(Icons.palette_outlined),
            onPressed: () async {
              final hex = await showColourPicker(context, name, chosen ?? base);
              if (hex != null) {
                onPick(hex);
              }
            },
          ),
        ],
      ),
    );
  }
}

class _UnknownSlotRow extends StatelessWidget {
  const _UnknownSlotRow({
    required this.name,
    required this.colour,
    required this.onRemove,
  });

  final String name;
  final String colour;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Row(
        children: [
          Icon(Icons.warning_amber_rounded, color: colors.amber),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  name,
                  style: AppText.bodyStrong.copyWith(color: colors.text),
                ),
                Text(
                  "Not in this song's palette, so it does nothing ($colour).",
                  style: AppText.small.copyWith(color: colors.amber),
                ),
              ],
            ),
          ),
          _Swatch(colour),
          IconButton(
            tooltip: 'Remove $name',
            icon: const Icon(Icons.delete_outline),
            onPressed: onRemove,
          ),
        ],
      ),
    );
  }
}

/// Asks for a colour for the slot [slot]: a grid of common ones and a hex
/// field, starting at [current]. Answers `#rrggbb`, or null when cancelled.
Future<String?> showColourPicker(
  BuildContext context,
  String slot,
  String current,
) => showDialog<String>(
  context: context,
  builder: (_) => _ColourPicker(slot: slot, current: current),
);

class _ColourPicker extends StatefulWidget {
  const _ColourPicker({required this.slot, required this.current});

  final String slot;
  final String current;

  @override
  State<_ColourPicker> createState() => _ColourPickerState();
}

class _ColourPickerState extends State<_ColourPicker> {
  late final TextEditingController _hex = TextEditingController(
    text: widget.current,
  );
  bool _bad = false;

  @override
  void dispose() {
    _hex.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final typed = normalizeColour(_hex.text);
    return AlertDialog(
      title: Text(widget.slot),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Wrap(
              spacing: 10,
              runSpacing: 10,
              children: [
                for (final hex in paletteChoices)
                  InkWell(
                    key: ValueKey('choice-$hex'),
                    borderRadius: BorderRadius.circular(8),
                    onTap: () => Navigator.of(context).pop(hex),
                    child: Semantics(
                      button: true,
                      label: 'Use $hex',
                      child: _Swatch(hex, size: 44),
                    ),
                  ),
              ],
            ),
            SheetField(
              controller: _hex,
              label: 'Hex',
              placeholder: '#rrggbb',
              helper: null,
              errorText: _bad ? 'Not a colour. Try #ff8800.' : null,
              onChanged: (_) => setState(() => _bad = false),
              suffix: typed == null ? null : _Swatch(typed, size: 20),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () {
            final hex = normalizeColour(_hex.text);
            if (hex == null) {
              setState(() => _bad = true);
              return;
            }
            Navigator.of(context).pop(hex);
          },
          child: const Text('Use'),
        ),
      ],
    );
  }
}
