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

export const SONGS = {
  songs: [
    {
      name: "Test Song Alpha",
      duration_ms: 180000,
      duration_display: "3:00",
      num_channels: 2,
      sample_format: "S24LE",
      track_count: 3,
      tracks: ["kick", "snare", "bass"],
      has_midi: true,
      has_lighting: true,
      base_dir: "Test Song Alpha",
      lighting_files: ["show.light"],
      midi_dmx_files: [],
    },
    {
      name: "Test Song Beta",
      duration_ms: 240000,
      duration_display: "4:00",
      num_channels: 2,
      sample_format: "S24LE",
      track_count: 2,
      tracks: ["guitar", "vocals"],
      has_midi: false,
      has_lighting: false,
      base_dir: "Test Song Beta",
      lighting_files: [],
      midi_dmx_files: [],
    },
  ],
  failures: [
    {
      name: "Broken Song",
      error: "Missing audio tracks",
      base_dir: "Broken Song",
      failed: true,
    },
  ],
};

export const PLAYLISTS = [
  { name: "all_songs", song_count: 2, is_active: false },
  { name: "setlist", song_count: 1, is_active: true },
  { name: "rehearsal", song_count: 2, is_active: false },
];

export const PLAYLIST_DETAILS: Record<
  string,
  { name: string; songs: string[]; available_songs: string[] }
> = {
  all_songs: {
    name: "all_songs",
    songs: ["Test Song Alpha", "Test Song Beta"],
    available_songs: [],
  },
  setlist: {
    name: "setlist",
    songs: ["Test Song Alpha"],
    available_songs: ["Test Song Beta"],
  },
  rehearsal: {
    name: "rehearsal",
    songs: ["Test Song Alpha", "Test Song Beta"],
    available_songs: [],
  },
};

export const CONFIG_YAML = `songs: songs
profiles:
  - hostname: test-host
    audio:
      device: default
samples: {}
`;

export const CONFIG_STORE = {
  yaml: CONFIG_YAML,
  checksum: "abc123def456",
};

export const STATUS = {
  build: {
    version: "0.1.0-test",
    git_hash: "deadbeef",
    build_time: "2026-01-01T00:00:00Z",
  },
  hardware: {
    init_done: true,
    hostname: "test-host",
    profile: "test-host",
    audio: { status: "connected", name: "Default Audio Device" },
    midi: { status: "not_connected", name: null },
    dmx: { status: "not_connected", name: null },
    trigger: { status: "not_connected", name: null },
  },
  controllers: [
    { kind: "osc", status: "running", detail: "0.0.0.0:9000", error: null },
  ],
};

export const AUDIO_DEVICES = [
  {
    name: "Default Audio Device",
    max_channels: 8,
    host_name: "ALSA",
    supported_sample_rates: [44100, 48000, 96000],
    supported_formats: [{ sample_format: "S24LE", bits_per_sample: 24 }],
  },
  {
    name: "USB Interface",
    max_channels: 18,
    host_name: "ALSA",
    supported_sample_rates: [44100, 48000],
    supported_formats: [
      { sample_format: "S24LE", bits_per_sample: 24 },
      { sample_format: "S16LE", bits_per_sample: 16 },
    ],
  },
];

export const MIDI_DEVICES = [
  { name: "MIDI Through Port-0", has_input: true, has_output: true },
  { name: "USB MIDI Controller", has_input: true, has_output: false },
];

export const PROFILE_FILES = [
  {
    filename: "test-host.yaml",
    hostname: "test-host",
    has_audio: true,
    has_midi: false,
    has_dmx: false,
    has_trigger: false,
    has_controllers: true,
  },
];

export const PROFILE_FILE_DETAIL = {
  profile: {
    hostname: "test-host",
    audio: { device: "Default Audio Device" },
    controllers: [{ kind: "osc", address: "0.0.0.0:9000" }],
  },
  yaml: 'hostname: test-host\naudio:\n  device: "Default Audio Device"\ncontrollers:\n  - kind: osc\n    address: "0.0.0.0:9000"\n',
};

export const PLAYBACK_STATE = {
  type: "playback",
  is_playing: false,
  elapsed_ms: 0,
  song_name: "Test Song Alpha",
  song_duration_ms: 180000,
  playlist_name: "setlist",
  playlist_position: 0,
  playlist_songs: ["Test Song Alpha", "Test Song Beta"],
  tracks: [
    { name: "kick", output_channels: [0, 1] },
    { name: "snare", output_channels: [2, 3] },
    { name: "bass", output_channels: [4, 5] },
  ],
  available_playlists: ["all_songs", "setlist"],
  persisted_playlist_name: "setlist",
  locked: false,
};

export const METADATA_STATE = {
  type: "metadata",
  fixtures: {
    "front-left": { tags: ["front", "left"], type: "par" },
    "front-right": { tags: ["front", "right"], type: "par" },
  },
};

export const FIXTURE_STATE = {
  type: "state",
  fixtures: {
    "front-left": { red: 255, green: 0, blue: 128, dimmer: 200, strobe: 0 },
    "front-right": { red: 0, green: 255, blue: 64, dimmer: 180, strobe: 0 },
  },
  active_effects: ["color_wash", "strobe_pulse"],
};

export const WAVEFORM_DATA = {
  type: "waveform",
  song_name: "Test Song Alpha",
  tracks: [
    { name: "kick", peaks: [0.5, 0.8, 0.3, 0.9, 0.2] },
    { name: "snare", peaks: [0.1, 0.4, 0.7, 0.2, 0.6] },
    { name: "bass", peaks: [0.6, 0.5, 0.4, 0.7, 0.3] },
  ],
};

export const LOG_LINES = {
  type: "logs",
  lines: [
    { level: "INFO", target: "mtrack::player", message: "Player started" },
    {
      level: "INFO",
      target: "mtrack::webui",
      message: "Web UI listening on 0.0.0.0:8080",
    },
  ],
};
