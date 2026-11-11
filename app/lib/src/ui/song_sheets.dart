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
import '../app/playlist_settings.dart';
import '../app/song_edit.dart';
import 'playlist_settings_sheet.dart';
import 'theme.dart';
import 'widgets.dart';

/// Said under each song sheet's title: nothing is sent until the song is.
const String sheetSavesWithSong =
    'Changes reach the player when you save the song.';

Future<void> _showSheet(BuildContext context, WidgetBuilder builder) =>
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: builder,
    );

/// Opens the song's loop crossfade and the tracks that cut over in it.
Future<void> showLoopingSheet(BuildContext context, SongEdit edit) =>
    _showSheet(context, (_) => LoopingSheet(edit: edit));

/// Opens the track at [index] of [edit], or a new one with null.
Future<void> showTrackSheet(BuildContext context, SongEdit edit, int? index) =>
    _showSheet(context, (_) => TrackSheet(edit: edit, index: index));

/// Opens the sample at [index] of [edit], or a new one with null.
Future<void> showSampleSheet(BuildContext context, SongEdit edit, int? index) =>
    _showSheet(context, (_) => SampleSheet(edit: edit, index: index));

/// Asks whether to remove [what]. True to remove it.
Future<bool> _confirmRemove(BuildContext context, String what) async {
  final colors = context.colors;
  final remove = await showDialog<bool>(
    context: context,
    builder: (dialog) => AlertDialog(
      title: Text('Remove $what?'),
      content: const Text('It goes when you save the song.'),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(false),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(true),
          child: Text('Remove', style: TextStyle(color: colors.red)),
        ),
      ],
    ),
  );
  return remove ?? false;
}

/// A red Remove button at the foot of a sheet.
class _RemoveButton extends StatelessWidget {
  const _RemoveButton({required this.label, required this.onPressed});

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 18),
    child: ChunkyButton(
      height: 48,
      fontSize: 14,
      label: label.toUpperCase(),
      semanticLabel: label,
      borderColor: context.colors.red,
      foreground: context.colors.red,
      onPressed: onPressed,
    ),
  );
}

/// The loop crossfade and the tracks that switch over at its midpoint, as
/// the web UI's song editor words them.
class LoopingSheet extends StatefulWidget {
  /// The sheet for [edit]'s looping.
  const LoopingSheet({required this.edit, super.key});

  /// The song being changed.
  final SongEdit edit;

  @override
  State<LoopingSheet> createState() => _LoopingSheetState();
}

class _LoopingSheetState extends State<LoopingSheet> {
  late final TextEditingController _crossfade = TextEditingController(
    text: widget.edit.looping.crossfade,
  );
  late final Set<String> _cut = {...widget.edit.looping.cut};
  final _crossfadeKey = GlobalKey();
  bool _bad = false;

  @override
  void dispose() {
    _crossfade.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    // The song's tracks, and any the file names that are not among them
    // (its metronome's, its pilot's).
    final names = [
      ...widget.edit.trackNames,
      for (final name in widget.edit.looping.cut)
        if (!widget.edit.trackNames.contains(name)) name,
    ];
    return SheetFrame(
      title: 'Loop crossfade',
      note: sheetSavesWithSong,
      onDone: () {
        setState(() => _bad = !looksLikeLength(_crossfade.text));
        if (_bad) {
          showFirstField(context, [_crossfadeKey]);
          return false;
        }
        widget.edit.setCrossfade(_crossfade.text);
        widget.edit.setCut([
          for (final name in names)
            if (_cut.contains(name)) name,
        ]);
        return true;
      },
      children: [
        const SheetLabel(
          'Loop crossfade',
          'Start each pass this long before the last ends, crossing the '
              'audio and lights over the overlap: 6s, 500ms, 8beats or '
              '2measures. Beats and measures count back from the end of the '
              'song. Leave empty for passes back to back.',
        ),
        SheetField(
          controller: _crossfade,
          label: 'Length',
          placeholder: 'none (e.g. 2measures)',
          helper: null,
          fieldKey: _crossfadeKey,
          errorText: _bad ? notALengthMessage : null,
          onChanged: (_) => setState(() => _bad = false),
        ),
        const SheetLabel(
          'Switch at the midpoint',
          'Tracks that switch over halfway through the crossfade instead of '
              'fading, such as a click, so they are never heard twice.',
        ),
        if (names.isEmpty)
          Text(
            'The song has no tracks.',
            style: AppText.small.copyWith(color: context.colors.muted),
          ),
        Wrap(
          spacing: 8,
          runSpacing: 4,
          children: [
            for (final name in names)
              FilterChip(
                label: Text(name),
                selected: _cut.contains(name),
                onSelected: (on) => setState(() {
                  if (on) {
                    _cut.add(name);
                  } else {
                    _cut.remove(name);
                  }
                }),
              ),
          ],
        ),
      ],
    );
  }
}

/// The audio files in the song's folder, one to choose; a file that is not
/// among them but is the current one is shown too.
class _FileChoice extends StatelessWidget {
  const _FileChoice({
    required this.files,
    required this.selected,
    required this.onSelected,
    this.none,
  });

  final List<player.SongFile> files;
  final String? selected;
  final ValueChanged<String?> onSelected;

  /// When set, a choice of no file at all, called this.
  final String? none;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final current = selected;
    final names = {for (final f in files) f.name: f.channels};
    final rows = <(String?, String)>[
      if (none != null) (null, none!),
      if (current != null && !names.containsKey(current))
        (current, '$current (not in the song\'s folder)'),
      for (final f in files)
        (
          f.name,
          f.channels == 0
              ? f.name
              : '${f.name} · ${f.channels == 1 ? 'mono' : '${f.channels} channels'}',
        ),
    ];
    if (rows.isEmpty) {
      return Text(
        'The song\'s folder has no audio files. Add them from the web UI.',
        style: AppText.small.copyWith(color: colors.muted),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (value, label) in rows)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: RowCard(
              tint: value == current ? colors.currentTint : null,
              mark: value == current ? colors.cyan : null,
              onTap: () => onSelected(value),
              semanticLabel: '$label${value == current ? ', chosen' : ''}',
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
              child: Text(
                label,
                style: AppText.body.copyWith(color: colors.text),
              ),
            ),
          ),
      ],
    );
  }
}

/// A track: its name, its file and the channel of the file it plays.
class TrackSheet extends StatefulWidget {
  /// The sheet for the track at [index] of [edit], or a new one.
  const TrackSheet({required this.edit, required this.index, super.key});

  /// The song being changed.
  final SongEdit edit;

  /// The track's place; null for a new one.
  final int? index;

  @override
  State<TrackSheet> createState() => _TrackSheetState();
}

class _TrackSheetState extends State<TrackSheet> {
  late final player.SongTrack? _initial = widget.index == null
      ? null
      : widget.edit.tracks[widget.index!];
  late final TextEditingController _name = TextEditingController(
    text: _initial?.name ?? '',
  );
  late final TextEditingController _channel = TextEditingController(
    text: _initial != null && _initial.hasFileChannel()
        ? '${_initial.fileChannel}'
        : '',
  );
  late String? _file = _initial?.file;
  final _nameKey = GlobalKey();
  final _channelKey = GlobalKey();
  String? _nameError;
  String? _fileError;
  String? _channelError;

  @override
  void dispose() {
    _name.dispose();
    _channel.dispose();
    super.dispose();
  }

  int? _channelsOf(String? file) {
    for (final f in widget.edit.files) {
      if (f.name == file && f.channels > 0) {
        return f.channels;
      }
    }
    return null;
  }

  void _choose(String? file) {
    setState(() {
      _fileError = null;
      // A new track is named after its file, as the web UI names one.
      if (_name.text.trim().isEmpty && file != null) {
        final dot = file.lastIndexOf('.');
        _name.text = dot > 0 ? file.substring(0, dot) : file;
      }
      _file = file;
    });
  }

  bool _done() {
    final name = _name.text.trim();
    final channelText = _channel.text.trim();
    final channel = channelText.isEmpty ? null : int.tryParse(channelText);
    setState(() {
      _nameError = name.isEmpty ? 'Give the track a name.' : null;
      _fileError = _file == null ? 'Choose a file.' : null;
      _channelError = channelText.isNotEmpty && (channel == null || channel < 1)
          ? 'A channel is a number from 1, or empty.'
          : null;
    });
    if (_nameError != null || _fileError != null || _channelError != null) {
      showFirstField(context, [
        if (_nameError != null) _nameKey,
        if (_channelError != null) _channelKey,
      ]);
      return false;
    }
    final track = player.SongTrack(name: name, file: _file);
    if (channel != null) {
      track.fileChannel = channel;
    }
    final index = widget.index;
    if (index == null) {
      widget.edit.addTrack(track);
    } else if (track != _initial) {
      widget.edit.setTrack(index, track);
    }
    return true;
  }

  Future<void> _remove() async {
    final navigator = Navigator.of(context);
    if (await _confirmRemove(context, 'the track ${_initial!.name}') &&
        mounted) {
      widget.edit.removeTrack(widget.index!);
      navigator.pop();
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final channels = _channelsOf(_file);
    return SheetFrame(
      title: _initial == null ? 'New track' : _initial.name,
      note: sheetSavesWithSong,
      onDone: _done,
      children: [
        const SheetLabel(
          'Name',
          'The name routes the track to an output, as the profile\'s track '
              'mappings name it.',
        ),
        SheetField(
          controller: _name,
          label: 'Name',
          placeholder: 'e.g. click',
          helper: null,
          fieldKey: _nameKey,
          errorText: _nameError,
          onChanged: (_) => setState(() => _nameError = null),
        ),
        const SheetLabel('File', "An audio file in the song's folder."),
        if (_fileError != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(
              _fileError!,
              style: AppText.small.copyWith(color: colors.red),
            ),
          ),
        _FileChoice(
          files: widget.edit.files,
          selected: _file,
          onSelected: _choose,
        ),
        const SheetLabel(
          'Channel',
          'Which channel of the file the track plays, from 1. Empty for a '
              'file with one channel.',
        ),
        SheetField(
          controller: _channel,
          label: 'Channel',
          placeholder: 'the only one',
          helper: channels == null
              ? null
              : channels == 1
              ? 'This file has one channel.'
              : 'This file has $channels channels.',
          keyboardType: TextInputType.number,
          fieldKey: _channelKey,
          errorText: _channelError,
          onChanged: (_) => setState(() => _channelError = null),
        ),
        if (_initial != null)
          _RemoveButton(label: 'Remove track', onPressed: _remove),
      ],
    );
  }
}

/// A sample of the song's own, as the web UI's samples section has one:
/// its file, where it plays, and what a release and a retrigger do. Its
/// velocity is shown and kept as it is.
class SampleSheet extends StatefulWidget {
  /// The sheet for the sample at [index] of [edit], or a new one.
  const SampleSheet({required this.edit, required this.index, super.key});

  /// The song being changed.
  final SongEdit edit;

  /// The sample's place; null for a new one.
  final int? index;

  @override
  State<SampleSheet> createState() => _SampleSheetState();
}

class _SampleSheetState extends State<SampleSheet> {
  late final player.SongSample _initial = widget.index == null
      ? player.SongSample()
      : widget.edit.samples[widget.index!];
  late final TextEditingController _name = TextEditingController(
    text: _initial.name,
  );
  late final TextEditingController _outputTrack = TextEditingController(
    text: _initial.hasOutputTrack() ? _initial.outputTrack : '',
  );
  late final TextEditingController _channels = TextEditingController(
    text: _initial.outputChannels.join(', '),
  );
  late final TextEditingController _maxVoices = TextEditingController(
    text: _initial.hasMaxVoices() ? '${_initial.maxVoices}' : '',
  );
  late final TextEditingController _fade = TextEditingController(
    text: _initial.hasFadeTimeMs() && _initial.fadeTimeMs != defaultSampleFadeMs
        ? '${_initial.fadeTimeMs}'
        : '',
  );
  late String? _file = _initial.hasFile() ? _initial.file : null;
  late String _release = releaseOf(_initial);
  late String _retrigger = retriggerOf(_initial);
  final Map<String, GlobalKey> _keys = {
    for (final field in ['name', 'channels', 'voices', 'fade'])
      field: GlobalKey(),
  };
  final Map<String, String> _errors = {};

  bool get _isNew => widget.index == null;

  @override
  void dispose() {
    for (final c in [_name, _outputTrack, _channels, _maxVoices, _fade]) {
      c.dispose();
    }
    super.dispose();
  }

  bool _done() {
    final name = _name.text.trim();
    final channels = parseChannels(_channels.text);
    final voicesText = _maxVoices.text.trim();
    final voices = voicesText.isEmpty ? null : int.tryParse(voicesText);
    final fadeText = _fade.text.trim();
    final fade = fadeText.isEmpty ? null : int.tryParse(fadeText);
    setState(() {
      _errors.clear();
      if (name.isEmpty) {
        _errors['name'] = 'Give the sample a name.';
      } else if (widget.edit.sampleNameTaken(name, index: widget.index)) {
        _errors['name'] = 'Another sample has that name.';
      }
      if (channels == null) {
        _errors['channels'] = 'Channel numbers from 1, such as 1, 2.';
      }
      if (voicesText.isNotEmpty && (voices == null || voices < 1)) {
        _errors['voices'] = 'A number from 1, or empty for the global limit.';
      }
      if (fadeText.isNotEmpty && (fade == null || fade < 0)) {
        _errors['fade'] = 'A number of milliseconds, or empty for 50.';
      }
    });
    if (_errors.isNotEmpty) {
      showFirstField(context, [
        for (final field in ['name', 'channels', 'voices', 'fade'])
          if (_errors.containsKey(field)) _keys[field],
      ]);
      return false;
    }
    final sample = _initial.deepCopy()
      ..name = name
      ..releaseBehavior = _release
      ..retrigger = _retrigger
      ..fadeTimeMs = fade ?? defaultSampleFadeMs;
    sample.outputChannels
      ..clear()
      ..addAll(channels!);
    if (_file == null) {
      sample.clearFile();
    } else {
      sample.file = _file!;
    }
    final track = _outputTrack.text.trim();
    if (track.isEmpty) {
      sample.clearOutputTrack();
    } else {
      sample.outputTrack = track;
    }
    if (voices == null) {
      sample.clearMaxVoices();
    } else {
      sample.maxVoices = voices;
    }
    final index = widget.index;
    if (index == null) {
      widget.edit.addSample(sample);
    } else if (sample != _initial) {
      widget.edit.setSample(index, sample);
    }
    return true;
  }

  Future<void> _remove() async {
    final navigator = Navigator.of(context);
    if (await _confirmRemove(context, 'the sample ${_initial.name}') &&
        mounted) {
      widget.edit.removeSample(widget.index!);
      navigator.pop();
    }
  }

  SheetField _field(
    String key,
    TextEditingController controller,
    String label,
    String placeholder, {
    String? helper,
    TextInputType? keyboardType,
  }) => SheetField(
    controller: controller,
    label: label,
    placeholder: placeholder,
    helper: helper,
    keyboardType: keyboardType,
    fieldKey: _keys[key],
    errorText: _errors[key],
    onChanged: (_) => setState(() => _errors.remove(key)),
  );

  Widget _choices(
    List<(String, String)> choices,
    String value,
    ValueChanged<String> onSelected,
  ) => Wrap(
    spacing: 8,
    runSpacing: 4,
    children: [
      for (final (key, label) in choices)
        ChoiceChip(
          label: Text(label),
          selected: value == key,
          onSelected: (_) => setState(() => onSelected(key)),
        ),
    ],
  );

  @override
  Widget build(BuildContext context) {
    final velocity = velocitySummary(_initial);
    final layered =
        _initial.hasVelocity() && _initial.velocity.mode == 'layers';
    return SheetFrame(
      title: _isNew ? 'New sample' : _initial.name,
      note: sheetSavesWithSong,
      onDone: _done,
      children: [
        _field(
          'name',
          _name,
          'Name',
          'e.g. kick',
          helper: 'Its triggers name it.',
        ),
        const SheetLabel('File', "An audio file in the song's folder."),
        _FileChoice(
          files: widget.edit.files,
          selected: _file,
          // A sample with velocity layers may play those alone.
          none: layered ? 'None: the velocity layers only' : null,
          onSelected: (file) => setState(() => _file = file),
        ),
        const SheetLabel(
          'Output Track',
          "Name of a track mapping (defined in Audio settings) to route this "
              "sample's output. Overrides output channels if set.",
        ),
        _field('track', _outputTrack, 'Output Track', 'Track mapping name'),
        const SheetLabel(
          'Output Channels',
          'Device output channels for this sample, as comma-separated '
              "numbers (e.g. '1, 2'). If both output channels and output "
              'track are empty, uses the default stereo output.',
        ),
        _field(
          'channels',
          _channels,
          'Output Channels',
          'default stereo',
          keyboardType: TextInputType.text,
        ),
        const SheetEyebrow('ADVANCED'),
        const SheetLabel(
          'Release Behavior',
          'What happens when a release event fires for this sample. '
              "'Play to Completion' ignores the release; 'Stop' cuts "
              "immediately; 'Fade' fades out over the fade time.",
        ),
        _choices(releaseChoices, _release, (v) => _release = v),
        const SheetLabel(
          'Retrigger',
          'Behavior when this sample is triggered while already playing. '
              "'Cut' stops the current voice first; 'Polyphonic' layers a "
              'new voice on top.',
        ),
        _choices(retriggerChoices, _retrigger, (v) => _retrigger = v),
        _field(
          'voices',
          _maxVoices,
          'Max Voices',
          'Global limit',
          keyboardType: TextInputType.number,
        ),
        _field(
          'fade',
          _fade,
          'Fade Time (ms)',
          '$defaultSampleFadeMs',
          keyboardType: TextInputType.number,
        ),
        if (velocity != null)
          Padding(
            padding: const EdgeInsets.only(top: 14),
            child: NoteBox(
              child: Text(
                '$velocity. Kept as it is; change it from the web UI.',
              ),
            ),
          ),
        if (!_isNew) _RemoveButton(label: 'Remove sample', onPressed: _remove),
      ],
    );
  }
}
