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
/// What the demo player starts with: an invented library of songs, the
/// playlists made of them, the output tracks of an invented rig and the light
/// shows it could play. Nothing here is a real band, song or artist.
library;

import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import '../../gen/player/v1/player.pb.dart' as player;

/// The name of the playlist of every song, as the real player calls it.
const String demoAllSongs = 'all_songs';

/// The name the demo player gives itself.
const String demoPlayerName = 'Demo player';

/// The output tracks of the demo's rig, in the player's order, with the gain
/// each starts at.
const List<(String, double)> demoOutputTracks = [
  ('click', 0),
  ('cues', -4.5),
  ('backing', -2),
  ('drums', 0),
  ('bass', -1.5),
  ('keys', 0),
  ('guitars', -3),
  ('vocals', -2),
];

/// The `.light` files the demo offers for an ambient show.
const List<String> demoLightShows = [
  'lighting/ambient_cool.light',
  'lighting/ambient_warm.light',
  'lighting/slow_chase.light',
  'songs/Harbor Lights/harbor_lights.light',
  'songs/Sundial/sundial.light',
];

/// The ambient show the demo plays between songs.
const String demoAmbientShow = 'lighting/ambient_warm.light';

/// One section of a song: [bars] bars long, in [color].
class DemoSection {
  /// A section.
  const DemoSection(this.name, this.bars, this.color);

  /// Its name.
  final String name;

  /// Its length in bars.
  final int bars;

  /// Its colour in the section strip, as `#rrggbb`.
  final String color;
}

DemoSection _s(String name, int bars) => DemoSection(name, bars, switch (name) {
  'Intro' || 'Outro' => '#6c7a89',
  'Verse' => '#2f80ed',
  'Chorus' => '#eb5757',
  _ => '#9b51e0',
});

/// A song of the demo library: its fixed shape, and the settings the demo
/// lets a person change.
class DemoSong {
  /// A song of [name] at [bpm], [beatsPerBar] beats to the bar, made of
  /// [sections]. [folder] is where it is listed ("" for the top).
  DemoSong({
    required this.name,
    required this.bpm,
    required this.sections,
    required this.tracks,
    this.beatsPerBar = 4,
    this.folder = '',
    this.loops = false,
    this.crossfade = '',
    this.cut = const [],
    this.samples = const [],
    this.triggers = const [],
    this.extraFiles = const [],
    this.palette,
    this.hasLighting = true,
  }) : looping = player.SongLooping(
         loopPlayback: loops,
         crossfade: crossfade,
         cut: cut,
       ),
       trackList = List.unmodifiable([
         for (final track in tracks)
           player.SongTrack(
             name: track.$1,
             file: track.$2,
             fileChannel: track.$3,
           ),
       ]),
       sampleList = List.unmodifiable(samples);

  /// The song's name.
  final String name;

  /// Its tempo.
  final double bpm;

  /// The beats in a bar.
  final int beatsPerBar;

  /// Its sections, in order.
  final List<DemoSection> sections;

  /// The tracks it starts with: name, file and file channel.
  final List<(String, String, int?)> tracks;

  /// The folder under the songs directory it is listed in ("" for the top).
  final String folder;

  /// Whether it starts out looping.
  final bool loops;

  /// The loop crossfade it starts with.
  final String crossfade;

  /// The tracks that cut over at the crossfade's midpoint it starts with.
  final List<String> cut;

  /// The samples it starts with.
  final List<player.SongSample> samples;

  /// The triggers that play its samples.
  final List<player.SongTrigger> triggers;

  /// Audio files in its folder that no track uses yet.
  final List<(String, int)> extraFiles;

  /// Its lighting palette, if it has one.
  final player.SongPalette? palette;

  /// Whether it has a light show of its own.
  final bool hasLighting;

  /// The looping now (changed by a save).
  player.SongLooping looping;

  /// The tracks now (changed by a save).
  List<player.SongTrack> trackList;

  /// The samples now (changed by a save).
  List<player.SongSample> sampleList;

  /// Counts the saves; the revision a client must quote to save over it.
  int revision = 0;

  /// The song's bars.
  int get bars => sections.fold(0, (sum, s) => sum + s.bars);

  /// How long it plays.
  Duration get duration => Duration(
    microseconds:
        (bars * beatsPerBar * 60 / bpm * Duration.microsecondsPerSecond)
            .round(),
  );

  /// The song's beat grid: every beat in seconds, and which beat starts each
  /// bar.
  player.BeatGrid get beatGrid {
    final beats = bars * beatsPerBar;
    return player.BeatGrid(
      beats: [for (var i = 0; i < beats; i++) i * 60 / bpm],
      measureStarts: [for (var m = 0; m < bars; m++) m * beatsPerBar],
    );
  }

  /// Its sections as the player sends them: measures count from 1 and the end
  /// is exclusive.
  List<player.Section> get protoSections {
    var at = 1;
    return [
      for (final section in sections)
        player.Section(
          name: section.name,
          startMeasure: at,
          endMeasure: (at += section.bars),
          color: section.color,
        ),
    ];
  }

  /// The names of the tracks it has now.
  List<String> get trackNames => [for (final t in trackList) t.name];

  /// The audio files in its folder: those its tracks, its samples and the
  /// extras name. A save does not add files to the folder or take them away.
  List<player.SongFile> get files {
    final names = <String, int>{};
    for (final (_, file, _) in tracks) {
      names.putIfAbsent(file, () => _channelsOf(file));
    }
    for (final sample in samples) {
      if (sample.hasFile()) {
        names.putIfAbsent(sample.file, () => 2);
      }
    }
    for (final (file, channels) in extraFiles) {
      names[file] = channels;
    }
    final sorted = names.keys.toList()..sort();
    return [
      for (final file in sorted)
        player.SongFile(name: file, channels: names[file]),
    ];
  }

  // A click and the cues are mono; the rest of what the demo plays is stereo.
  static int _channelsOf(String file) =>
      file == 'click.wav' || file == 'cues.wav' ? 1 : 2;

  /// The `Song` message of the player's calls.
  player.Song toProto() => player.Song(
    name: name,
    duration: pb.Duration.fromDart(duration),
    tracks: trackNames,
    beatGrid: beatGrid,
    sections: protoSections,
  );
}

player.SongSample _sample(
  String name,
  String file, {
  String release = 'play_to_completion',
  String retrigger = 'cut',
  int? voices,
  int fadeMs = 50,
  List<int> channels = const [],
  String? track,
}) => player.SongSample(
  name: name,
  originalName: name,
  file: file,
  outputChannels: channels,
  outputTrack: track,
  releaseBehavior: release,
  retrigger: retrigger,
  maxVoices: voices,
  fadeTimeMs: fadeMs,
  velocity: player.SampleVelocity(mode: 'ignore', layers: 0),
);

/// The songs of the demo library, sorted by name as the real player lists
/// them.
List<DemoSong> demoSongs() => [
  DemoSong(
    name: 'Copper Wire',
    bpm: 132,
    sections: [
      _s('Intro', 4),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Bridge', 8),
      _s('Chorus', 8),
      _s('Outro', 4),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('drums', 'drums.wav', null),
      ('bass', 'bass.wav', null),
      ('guitars', 'guitars.wav', null),
      ('vocals', 'vocals.wav', null),
    ],
    samples: [_sample('clap', 'clap.wav', voices: 4, track: 'drums')],
    triggers: [
      player.SongTrigger(event: 'Note On, channel 10, key 39', sample: 'clap'),
    ],
  ),
  DemoSong(
    name: 'Harbor Lights',
    bpm: 104,
    sections: [
      _s('Intro', 8),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Bridge', 8),
      _s('Chorus', 16),
      _s('Outro', 8),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('backing', 'backing.wav', null),
      ('drums', 'drums.wav', null),
      ('bass', 'bass.wav', null),
      ('keys', 'keys.wav', null),
      ('guitars', 'guitars.wav', null),
      ('vocals', 'vocals.wav', null),
    ],
    samples: [
      _sample('kick', 'kick.wav', track: 'drums', retrigger: 'polyphonic'),
      _sample(
        'snare',
        'snare.wav',
        track: 'drums',
        release: 'fade',
        fadeMs: 120,
      ),
    ],
    triggers: [
      player.SongTrigger(event: 'Note On, channel 10, key 36', sample: 'kick'),
      player.SongTrigger(event: 'Note On, channel 10, key 38', sample: 'snare'),
    ],
    extraFiles: const [('alt-intro.wav', 2)],
    palette: player.SongPalette(
      slots: [
        player.PaletteSlot(name: 'primary', color: '#2f80ed'),
        player.PaletteSlot(name: 'accent', color: '#f2994a'),
      ],
    ),
  ),
  DemoSong(
    name: 'Long Way Down',
    bpm: 110,
    sections: [
      _s('Intro', 4),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Bridge', 8),
      _s('Chorus', 16),
      _s('Outro', 8),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('backing', 'backing.wav', null),
      ('keys', 'keys.wav', null),
      ('vocals', 'vocals.wav', null),
    ],
    folder: 'Rehearsal',
  ),
  DemoSong(
    name: 'Night Shift',
    bpm: 96,
    sections: [
      _s('Intro', 4),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Bridge', 8),
      _s('Chorus', 8),
      _s('Outro', 4),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('backing', 'backing.wav', null),
      ('bass', 'bass.wav', null),
      ('keys', 'keys.wav', null),
    ],
    folder: 'Rehearsal',
    hasLighting: false,
  ),
  DemoSong(
    name: 'Quiet Engines',
    bpm: 90,
    beatsPerBar: 3,
    sections: [
      _s('Intro', 8),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Bridge', 8),
      _s('Outro', 10),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('backing', 'backing.wav', null),
      ('keys', 'keys.wav', null),
      ('guitars', 'guitars.wav', null),
    ],
  ),
  DemoSong(
    name: 'Second Wind',
    bpm: 84,
    sections: [
      _s('Intro', 4),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Verse', 16),
      _s('Chorus', 8),
      _s('Bridge', 8),
      _s('Outro', 4),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('backing', 'backing.wav', null),
      ('keys', 'keys.wav', null),
      ('guitars', 'guitars.wav', null),
    ],
    loops: true,
    crossfade: '2s',
    cut: const ['keys'],
    folder: 'Rehearsal',
  ),
  DemoSong(
    name: 'Slow Orbit',
    bpm: 68,
    sections: [
      _s('Intro', 4),
      _s('Verse', 12),
      _s('Chorus', 8),
      _s('Verse', 12),
      _s('Chorus', 8),
      _s('Bridge', 8),
      _s('Chorus', 8),
      _s('Outro', 4),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('backing', 'backing.wav', null),
      ('bass', 'bass.wav', null),
      ('keys', 'keys.wav', 1),
      ('vocals', 'vocals.wav', null),
    ],
  ),
  DemoSong(
    name: 'Sundial',
    bpm: 120,
    sections: [
      _s('Intro', 8),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Verse', 16),
      _s('Chorus', 16),
      _s('Outro', 8),
    ],
    tracks: const [
      ('click', 'click.wav', null),
      ('cues', 'cues.wav', null),
      ('backing', 'backing.wav', null),
      ('drums', 'drums.wav', null),
      ('bass', 'bass.wav', null),
    ],
    samples: [
      _sample('rise', 'rise.wav', channels: const [3, 4]),
      _sample('crash', 'crash.wav', track: 'drums', voices: 2),
    ],
    triggers: [
      player.SongTrigger(event: 'Note On, channel 10, key 49', sample: 'crash'),
    ],
    palette: player.SongPalette(
      slots: [
        player.PaletteSlot(name: 'warm', color: '#f2c94c'),
        player.PaletteSlot(name: 'cool', color: '#56ccf2'),
      ],
    ),
  ),
]..sort((a, b) => a.name.compareTo(b.name));

/// A playlist entry as the demo keeps it.
class DemoEntry {
  /// An entry for [song] with [settings].
  DemoEntry(this.song, [player.EntrySettings? settings])
    : settings = settings ?? player.EntrySettings();

  /// The song's name.
  final String song;

  /// The entry's fades, auto play, ambient light and palette overrides.
  final player.EntrySettings settings;
}

/// A named playlist as the demo keeps it.
class DemoPlaylist {
  /// A playlist of [entries].
  DemoPlaylist(this.entries, {this.ambient});

  /// Its entries, in order.
  List<DemoEntry> entries;

  /// Its own ambient light, or null for the profile's.
  player.AmbientSetting? ambient;

  /// Counts the saves.
  int revision = 0;

  /// Where the playlist is: the index of its current entry.
  int cursor = 0;
}

/// The playlists the demo starts with, besides the one of every song.
Map<String, DemoPlaylist> demoPlaylists() => {
  'Short set': DemoPlaylist([
    DemoEntry(
      'Harbor Lights',
      player.EntrySettings(fadeOut: player.Fade(length: '4s')),
    ),
    DemoEntry(
      'Copper Wire',
      player.EntrySettings(autoPlay: player.AutoPlay(gap: '5s')),
    ),
    DemoEntry('Sundial'),
    DemoEntry('Long Way Down'),
  ], ambient: player.AmbientSetting(on: true, show: demoAmbientShow)),
  'Rehearsal list': DemoPlaylist([
    DemoEntry('Night Shift'),
    DemoEntry('Second Wind'),
    DemoEntry('Slow Orbit'),
    DemoEntry('Quiet Engines'),
    DemoEntry('Harbor Lights'),
  ]),
};
