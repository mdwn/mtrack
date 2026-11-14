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

import type { saveVenue } from "../../src/lib/api/config";
import type {
  FixtureTypeSaved,
  AimPoints,
  FixtureTypeDeleted,
  MvrKept,
  SampleUploaded,
  MvrImportAnswer,
  FixtureSettingsData,
  FixtureTypeEntry,
  FixtureTypeList,
  FixtureTypeGdtf,
  GdtfImport,
  GdtfInspection,
  SettingsSaved,
  Venue,
  VenueDetail,
  VenueList,
  VenueScene,
  TestMode,
} from "../../src/gen/rules/mtrack_rules";
import express from "express";
import { readFileSync } from "node:fs";
import nodePath from "node:path";
import { createServer } from "http";
import {
  SONGS,
  PLAYLISTS,
  PLAYLIST_DETAILS,
  CONFIG_STORE,
  STATUS,
  AUDIO_DEVICES,
  AUDIO_PROBE_RESULTS,
  MIDI_DEVICES,
  NOISE_FLOOR,
  CALIBRATION,
  PROFILE_FILES,
  PROFILE_FILE_DETAIL,
  PLAYBACK_STATE,
  METADATA_STATE,
  FIT,
  LIGHTING_GROUPS,
  READINESS,
  FIXTURE_STATE,
  TEST_RIG,
  TEST_SCENERY,
  WAVEFORM_DATA,
  songConfigYaml,
  songWaveform,
  SONG_FILES,
  LOG_LINES,
  MVR_INSPECTION,
  MVR_EXPORT_SUMMARY,
} from "./test-data.js";
import {
  create,
  fromBinary,
  toBinary,
  type DescMessage,
  type DescMethodUnary,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf";
import { Code } from "@connectrpc/connect";
import {
  AddProfileRequestSchema,
  BluetoothStatusSchema,
  BrowseDirectoryRequestSchema,
  BrowseDirectoryResponseSchema,
  BulkImportSongsRequestSchema,
  BulkImportSongsResponseSchema,
  CalibrationNoiseFloorSchema,
  CalibrationResultSchema,
  CancelCalibrationResponseSchema,
  CaptureCalibrationResponseSchema,
  CreateSongInDirectoryRequestSchema,
  CreateSongInDirectoryResponseSchema,
  CreateSongRequestSchema,
  CreateSongResponseSchema,
  DeletePlaylistRequestSchema,
  DeletePlaylistResponseSchema,
  DeleteProfileFileRequestSchema,
  DeleteSongRequestSchema,
  DeleteSongResponseSchema,
  ForgetBluetoothDeviceRequestSchema,
  GetConfigResponseSchema,
  GetPlaylistRequestSchema,
  GetPlaylistResponseSchema,
  GetProfileFileResponseSchema,
  GetSongConfigRequestSchema,
  GetSongConfigResponseSchema,
  GetSongWaveformRequestSchema,
  GuessSongTempoRequestSchema,
  ImportSongFileRequestSchema,
  ImportSongFileResponseSchema,
  LightingService,
  ListAudioDevicesResponseSchema,
  ListMidiDevicesResponseSchema,
  ListPlaylistsResponseSchema,
  ListProfileFilesResponseSchema,
  ListSongFilesRequestSchema,
  ListSongFilesResponseSchema,
  ListSongsResponseSchema,
  ProbeAudioDeviceRequestSchema,
  ProbeAudioDeviceResponseSchema,
  ProfileFileSavedSchema,
  RemoveProfileRequestSchema,
  RestartControllersResponseSchema,
  SavePlaylistRequestSchema,
  SaveProfileFileRequestSchema,
  SaveSongConfigRequestSchema,
  SaveSongConfigResponseSchema,
  SetBluetoothPairingRequestSchema,
  SetLockRequestSchema,
  SetLockResponseSchema,
  SwitchToPlaylistResponseSchema,
  UpdateConfigResponseSchema,
  UpdateMetronomeRequestSchema,
  UpdateProfileRequestSchema,
  UpdateSamplesRequestSchema,
  WaveformSchema,
} from "../../src/gen/player/v1/player_pb.js";
import {
  errorHeaders,
  requestMessage,
  unaryBody,
} from "../support/grpc-web.js";
import {
  bluetoothReply,
  browseReply,
  bulkImportReply,
  calibrationReply,
  createInDirectoryReply,
  createSongReply,
  getConfigReply,
  getSongConfigReply,
  importSongFileReply,
  listSongFilesReply,
  getProfileFileReply,
  listAudioDevicesReply,
  listMidiDevicesReply,
  listProfileFilesReply,
  noiseFloorReply,
  probeAudioReply,
  profileFileSavedReply,
  restartControllersReply,
  savedReply,
  waveformReply,
} from "../support/config-rpc.js";
import {
  getPlaylistReply,
  listPlaylistsReply,
  listSongsReply,
} from "../support/library-rpc.js";
import {
  fitReply,
  groupsReply,
  lightShowReply,
  previewReply,
  readinessReply,
  testAnswerReply,
  testOptionsReply,
  validationReply,
} from "../support/lighting-rpc.js";
import { StageStream, type StageMessage } from "../support/stage-stream.js";
import { watchFrame, type WatchPush } from "../support/watch-stream.js";

const app = express();
app.use(express.json());
app.use(express.text({ type: "text/*" }));

// --- The player's gRPC-Web calls that the pages make for the song list, the
// playlists, the lock and the config store ---
// Answered from the same fixtures the REST routes of the other pages use; a
// spec that needs other answers or the requests uses mockRpc
// (e2e/support/rpc.ts), which is consulted first.

const grpcRaw = express.raw({ type: () => true, limit: "1mb" });

/** A unary call of the player service: `answer` makes the response message
 *  from the decoded request bytes, or throws `GrpcFailure`. */
class GrpcFailure extends Error {
  constructor(
    readonly code: Code,
    message: string,
  ) {
    super(message);
  }
}

function grpcUnary(
  method: string,
  answer: (request: Uint8Array, req: express.Request) => Uint8Array,
  service = "PlayerService",
): void {
  app.post(`/player.v1.${service}/${method}`, grpcRaw, (req, res) => {
    try {
      const body = answer(requestMessage(new Uint8Array(req.body ?? [])), req);
      res
        .status(200)
        .set({ "content-type": "application/grpc-web+proto" })
        .end(Buffer.from(unaryBody(body)));
    } catch (e) {
      if (!(e instanceof GrpcFailure)) throw e;
      res.status(200).set(errorHeaders(e.code, e.message)).end();
    }
  });
}

grpcUnary("ListSongs", () =>
  toBinary(
    ListSongsResponseSchema,
    create(ListSongsResponseSchema, listSongsReply(SONGS)),
  ),
);

grpcUnary("ListPlaylists", () =>
  toBinary(
    ListPlaylistsResponseSchema,
    create(ListPlaylistsResponseSchema, listPlaylistsReply(PLAYLISTS)),
  ),
);

grpcUnary("GetPlaylist", (request) => {
  const { name } = fromBinary(GetPlaylistRequestSchema, request);
  const detail = PLAYLIST_DETAILS[name];
  if (!detail) throw new GrpcFailure(Code.NotFound, "Playlist not found");
  return toBinary(
    GetPlaylistResponseSchema,
    create(GetPlaylistResponseSchema, getPlaylistReply(detail)),
  );
});

// A save, a delete and a switch change nothing here: the fixtures are fixed,
// as the REST routes they replace were.
grpcUnary("SavePlaylist", (request) => {
  const { name } = fromBinary(SavePlaylistRequestSchema, request);
  return toBinary(
    GetPlaylistResponseSchema,
    create(GetPlaylistResponseSchema, { name }),
  );
});

grpcUnary("DeletePlaylist", (request) => {
  fromBinary(DeletePlaylistRequestSchema, request);
  return toBinary(
    DeletePlaylistResponseSchema,
    create(DeletePlaylistResponseSchema),
  );
});

grpcUnary("SwitchToPlaylist", () =>
  toBinary(
    SwitchToPlaylistResponseSchema,
    create(SwitchToPlaylistResponseSchema),
  ),
);

grpcUnary("SetLock", (request) => {
  const { locked } = fromBinary(SetLockRequestSchema, request);
  return toBinary(
    SetLockResponseSchema,
    create(SetLockResponseSchema, { locked }),
  );
});

// The config store: a read answers the fixture, and a profile write answers
// the fixture under a new checksum, applied at once, as the REST routes they
// replace did.
grpcUnary("GetConfig", () =>
  toBinary(
    GetConfigResponseSchema,
    create(GetConfigResponseSchema, getConfigReply(CONFIG_STORE)),
  ),
);

function profileSaved(checksum: string): Uint8Array {
  return toBinary(
    UpdateConfigResponseSchema,
    create(
      UpdateConfigResponseSchema,
      savedReply({ ...CONFIG_STORE, checksum }),
    ),
  );
}

grpcUnary("AddProfile", (request) => {
  fromBinary(AddProfileRequestSchema, request);
  return profileSaved("new-checksum-1");
});

grpcUnary("UpdateProfile", (request) => {
  fromBinary(UpdateProfileRequestSchema, request);
  return profileSaved("new-checksum-2");
});

grpcUnary("RemoveProfile", (request) => {
  fromBinary(RemoveProfileRequestSchema, request);
  return profileSaved("new-checksum-3");
});

// --- Songs ---

grpcUnary(
  "GetSongConfig",
  (request) => {
    const { name } = fromBinary(GetSongConfigRequestSchema, request);
    const song = SONGS.songs.find((s) => s.name === name);
    if (!song)
      throw new GrpcFailure(Code.NotFound, `unable to find song ${name}`);
    return toBinary(
      GetSongConfigResponseSchema,
      create(
        GetSongConfigResponseSchema,
        getSongConfigReply(songConfigYaml(song)),
      ),
    );
  },
  "ConfigService",
);

// A create, a save, a delete and an import change nothing here: the fixtures
// are fixed, as the REST routes they replace were.
grpcUnary(
  "CreateSong",
  (request) => {
    const { name } = fromBinary(CreateSongRequestSchema, request);
    return toBinary(
      CreateSongResponseSchema,
      create(CreateSongResponseSchema, createSongReply(name)),
    );
  },
  "ConfigService",
);

grpcUnary(
  "SaveSongConfig",
  (request) => {
    fromBinary(SaveSongConfigRequestSchema, request);
    return toBinary(
      SaveSongConfigResponseSchema,
      create(SaveSongConfigResponseSchema, {}),
    );
  },
  "ConfigService",
);

grpcUnary(
  "DeleteSong",
  (request) => {
    const { name } = fromBinary(DeleteSongRequestSchema, request);
    return toBinary(
      DeleteSongResponseSchema,
      create(DeleteSongResponseSchema, { name }),
    );
  },
  "ConfigService",
);

grpcUnary(
  "GetSongWaveform",
  (request) => {
    const { name } = fromBinary(GetSongWaveformRequestSchema, request);
    return toBinary(
      WaveformSchema,
      create(WaveformSchema, waveformReply(songWaveform(name))),
    );
  },
  "ConfigService",
);

// No song here has a MIDI file or a beat grid to guess from.
grpcUnary(
  "GuessSongTempo",
  (request) => {
    fromBinary(GuessSongTempoRequestSchema, request);
    throw new GrpcFailure(Code.NotFound, "Song has no MIDI file or beat grid");
  },
  "ConfigService",
);

grpcUnary(
  "ListSongFiles",
  (request) => {
    fromBinary(ListSongFilesRequestSchema, request);
    return toBinary(
      ListSongFilesResponseSchema,
      create(ListSongFilesResponseSchema, listSongFilesReply(SONG_FILES)),
    );
  },
  "ConfigService",
);

app.put("/api/songs/:name/tracks/:filename", (_req, res) => {
  res.json({ status: "uploaded" });
});

app.post("/api/songs/:name/tracks", (_req, res) => {
  res.json({ status: "uploaded" });
});

grpcUnary(
  "ImportSongFile",
  (request) => {
    const { name, path, filename } = fromBinary(
      ImportSongFileRequestSchema,
      request,
    );
    return toBinary(
      ImportSongFileResponseSchema,
      create(
        ImportSongFileResponseSchema,
        importSongFileReply({
          file: filename ?? path.split("/").pop() ?? path,
          song: name,
        }),
      ),
    );
  },
  "ConfigService",
);

// --- Devices, the audio probe and the controller restart: ConfigService ---

grpcUnary(
  "ListAudioDevices",
  () =>
    toBinary(
      ListAudioDevicesResponseSchema,
      create(
        ListAudioDevicesResponseSchema,
        listAudioDevicesReply(AUDIO_DEVICES),
      ),
    ),
  "ConfigService",
);

// Canned probe results keyed by device name, so a test can drive each branch
// of the result panel without any audio hardware in the loop.
grpcUnary(
  "ProbeAudioDevice",
  (request) => {
    const device = fromBinary(
      ProbeAudioDeviceRequestSchema,
      request,
    ).device.trim();
    if (!device) {
      throw new GrpcFailure(Code.InvalidArgument, "device is required");
    }
    return toBinary(
      ProbeAudioDeviceResponseSchema,
      create(
        ProbeAudioDeviceResponseSchema,
        probeAudioReply(
          AUDIO_PROBE_RESULTS[device] ?? AUDIO_PROBE_RESULTS.default,
        ),
      ),
    );
  },
  "ConfigService",
);

grpcUnary(
  "ListMidiDevices",
  () =>
    toBinary(
      ListMidiDevicesResponseSchema,
      create(ListMidiDevicesResponseSchema, listMidiDevicesReply(MIDI_DEVICES)),
    ),
  "ConfigService",
);

grpcUnary(
  "RestartControllers",
  () =>
    toBinary(
      RestartControllersResponseSchema,
      create(
        RestartControllersResponseSchema,
        restartControllersReply(STATUS.controllers),
      ),
    ),
  "ConfigService",
);

// --- Profile files, samples and the metronome: ConfigService ---

// A read answers the fixture, and a write answers it as applied at once and
// changes nothing, as the REST routes they replace did.
grpcUnary(
  "ListProfileFiles",
  () =>
    toBinary(
      ListProfileFilesResponseSchema,
      create(
        ListProfileFilesResponseSchema,
        listProfileFilesReply(PROFILE_FILES),
      ),
    ),
  "ConfigService",
);

grpcUnary(
  "GetProfileFile",
  () =>
    toBinary(
      GetProfileFileResponseSchema,
      create(
        GetProfileFileResponseSchema,
        getProfileFileReply(PROFILE_FILE_DETAIL),
      ),
    ),
  "ConfigService",
);

grpcUnary(
  "SaveProfileFile",
  (request) => {
    const { filename } = fromBinary(SaveProfileFileRequestSchema, request);
    return toBinary(
      ProfileFileSavedSchema,
      create(ProfileFileSavedSchema, profileFileSavedReply({ filename })),
    );
  },
  "ConfigService",
);

grpcUnary(
  "DeleteProfileFile",
  (request) => {
    const { filename } = fromBinary(DeleteProfileFileRequestSchema, request);
    return toBinary(
      ProfileFileSavedSchema,
      create(
        ProfileFileSavedSchema,
        profileFileSavedReply({ status: "deleted", filename }),
      ),
    );
  },
  "ConfigService",
);

grpcUnary(
  "UpdateSamples",
  (request) => {
    fromBinary(UpdateSamplesRequestSchema, request);
    return profileSaved("new-checksum-4");
  },
  "ConfigService",
);

grpcUnary(
  "UpdateMetronome",
  (request) => {
    fromBinary(UpdateMetronomeRequestSchema, request);
    return profileSaved("new-checksum-5");
  },
  "ConfigService",
);

// --- Samples ---

app.put("/api/samples/upload/:filename", (_req, res) => {
  res.json({
    status: "uploaded",
    file: "sample.wav",
    path: "/samples/sample.wav",
  } satisfies SampleUploaded);
});

// --- Bluetooth ---

// The Bluetooth controller's pairing, kept per test: a test names its own
// state with the `x-mock-session` header (the default session is shared by
// every test that sends none), so tests running side by side do not see
// each other's window or phones.
interface BluetoothSim {
  openUntil: number;
  phones: { address: string; name: string | null; connected: boolean }[];
}
const bluetoothSims = new Map<string, BluetoothSim>();

function bluetoothSim(req: express.Request): BluetoothSim {
  const key = String(req.header("x-mock-session") ?? "");
  let sim = bluetoothSims.get(key);
  if (!sim) {
    sim = {
      openUntil: 0,
      phones: [
        { address: "AA:BB:CC:DD:EE:01", name: "Pixel", connected: true },
        { address: "AA:BB:CC:DD:EE:02", name: null, connected: false },
      ],
    };
    bluetoothSims.set(key, sim);
  }
  return sim;
}

function bluetoothBody(sim: BluetoothSim): Uint8Array {
  const left = Math.max(0, Math.ceil((sim.openUntil - Date.now()) / 1000));
  return toBinary(
    BluetoothStatusSchema,
    create(
      BluetoothStatusSchema,
      bluetoothReply({
        configured: true,
        running: true,
        error: null,
        pairing: { open: left > 0, seconds_left: left },
        devices: sim.phones,
      }),
    ),
  );
}

grpcUnary(
  "GetBluetooth",
  (_request, req) => bluetoothBody(bluetoothSim(req)),
  "ConfigService",
);

grpcUnary(
  "SetBluetoothPairing",
  (request, req) => {
    const { open, seconds } = fromBinary(
      SetBluetoothPairingRequestSchema,
      request,
    );
    const sim = bluetoothSim(req);
    const window = Math.min(600, Math.max(10, seconds ?? 120));
    sim.openUntil = open ? Date.now() + window * 1000 : 0;
    return bluetoothBody(sim);
  },
  "ConfigService",
);

grpcUnary(
  "ForgetBluetoothDevice",
  (request, req) => {
    const { address } = fromBinary(ForgetBluetoothDeviceRequestSchema, request);
    const sim = bluetoothSim(req);
    const before = sim.phones.length;
    sim.phones = sim.phones.filter((p) => p.address !== address);
    if (sim.phones.length === before) {
      throw new GrpcFailure(
        Code.NotFound,
        "Not a phone paired with this player",
      );
    }
    return bluetoothBody(sim);
  },
  "ConfigService",
);

// --- Lighting ---

// The three forms a fixture type comes in: a v1 `.light` channel map, a
// hand-written rich `.fixture`, and one distilled from a GDTF archive —
// which has no channels of its own until the lighting system expands it.
const FIXTURE_TYPES: Record<string, FixtureTypeEntry> = {
  par: {
    fixture_type: {
      name: "par",
      channels: { red: 1, green: 2, blue: 3, dimmer: 4 },
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "par.light",
    owner: { file: "par.light", editable: true, reason: null },
    form_editable: true,
    extension: "light",
    referential: false,
    rich: false,
    dsl: 'fixture_type "par" { red: 1, green: 2, blue: 3, dimmer: 4 }',
  },
  mover: {
    fixture_type: {
      name: "mover",
      channels: { pan: 1, tilt: 3, dimmer: 5 },
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "mover.fixture",
    owner: { file: "mover.fixture", editable: true, reason: null },
    form_editable: false,
    extension: "fixture",
    referential: false,
    rich: true,
    dsl: 'fixture_type "mover" {\n  channel "pan" @ 1 fine 2 range -270deg..270deg\n}\n',
  },
  pixelbrick: {
    fixture_type: {
      name: "pixelbrick",
      channels: {},
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "pixelbrick.fixture",
    owner: { file: "pixelbrick.fixture", editable: true, reason: null },
    form_editable: false,
    extension: "fixture",
    referential: true,
    rich: false,
    dsl: 'fixture_type "pixelbrick" from gdtf("library/pb15.gdtf") {\n}\n',
    gdtf: {
      fixture: "PB15 PixelBrick",
      manufacturer: "Astera LED Technology",
      modes: 2,
      beam: { type: "Wash", angle: 13 },
      thumbnail: null,
      used_by: 3,
      in_use: [
        { mode: "8: RGBS", count: 2 },
        { mode: "1: RGB", count: 1 },
      ],
    },
  },
};

app.get("/api/lighting/fixture-types", (_req, res) => {
  // The listing carries everything but the source text, which GET-one has.
  const fixture_types = Object.fromEntries(
    Object.entries(FIXTURE_TYPES).map(([name, entry]) => [
      name,
      { ...entry, dsl: undefined },
    ]),
  );
  res.json({ fixture_types, errors: [] } satisfies FixtureTypeList);
});

app.get("/api/lighting/fixture-types/:name", (req, res) => {
  const entry = FIXTURE_TYPES[req.params.name];
  if (!entry) {
    return res.status(404).json({ error: "Fixture type not found" });
  }
  res.json(entry);
});

app.put("/api/lighting/fixture-types/:name", (req, res) => {
  // A rename (`?from=`) retypes the venue lines naming the old name.
  const from = typeof req.query.from === "string" ? req.query.from : null;
  res.json({
    status: "saved",
    name: req.params.name,
    rename:
      from && from !== req.params.name
        ? {
            from,
            to: req.params.name,
            lines: 3,
            venues: [
              { venue: "built-in", file: "built_in.venue", lines: 2 },
              { venue: "club", file: "club.venue", lines: 1 },
            ],
          }
        : null,
    venue_error: null,
  } satisfies FixtureTypeSaved);
});

app.delete("/api/lighting/fixture-types/:name", (req, res) => {
  res.json({
    status: "deleted",
    name: req.params.name,
    archive_removed: null,
    archive_kept: false,
  } satisfies FixtureTypeDeleted);
});

// The PB15's modes, as the inspect endpoint and the details view answer them.
const GDTF_INSPECTION: GdtfInspection = {
  modes: [
    {
      name: "1: RGB",
      footprint: 3,
      capabilities: ["color", "strobe"],
      wants: ["color", "dimmer", "strobe"],
      cells: 0,
      channels: [
        [1, "red"],
        [2, "green"],
        [3, "blue"],
      ],
    },
    {
      name: "8: RGBS",
      footprint: 4,
      capabilities: ["color", "strobe"],
      wants: ["color", "dimmer", "strobe"],
      cells: 0,
      strobe_range: { min_hz: 0.4, max_hz: 25 },
      channels: [
        [1, "red"],
        [2, "green"],
        [3, "blue"],
        [4, "strobe"],
      ],
    },
  ],
};

// A GDTF type's own settings, for the fixture page's form. Stateless: a save
// answers what it did and changes nothing here, so tests never see another
// test's save. Tests that need overlaps route their own answer.
app.get("/api/lighting/fixture-types/:name/settings", (req, res) => {
  const entry = FIXTURE_TYPES[req.params.name];
  if (!entry || !entry.referential) {
    return res.status(404).json({ error: "not a GDTF type" });
  }
  res.json({
    name: req.params.name,
    movement: { max_pan_speed: null, max_tilt_speed: null },
    strobe_curve: null,
    // The PB15's strobe function has only its endpoints.
    strobe: { steps: 0, automatic: "declared" },
    version: "mock-v1",
  } satisfies FixtureSettingsData);
});

app.post("/api/lighting/fixture-types/:name/settings", (req, res) => {
  const from = req.params.name;
  const body = req.body ?? {};
  const renamed = body.name && body.name !== from;
  res.json({
    version: body.write ? "mock-v2" : "mock-v1",
    rename: renamed
      ? {
          from,
          to: body.name,
          lines: 3,
          venues: [
            { venue: "built-in", file: "built_in.venue", lines: 2 },
            { venue: "club", file: "club.venue", lines: 1 },
          ],
        }
      : null,
    venue_versions: { "built_in.venue": "v-b", "club.venue": "v-c" },
    venue_error: null,
  } satisfies SettingsSaved);
});

// A referential type's archive, for its details view: the mock's one rig
// model stands in for the PB15's. Any other type is not a GDTF type.
app.get("/api/lighting/fixture-types/:name/gdtf", (req, res) => {
  const entry = FIXTURE_TYPES[req.params.name];
  if (!entry) {
    return res.status(404).json({ error: "Fixture type not found" });
  }
  if (!entry.referential) {
    return res.status(404).json({
      error: `fixture type "${req.params.name}" is not a GDTF type`,
    });
  }
  res.json({
    archive: "library/pb15.gdtf",
    rig: "test-archive/rig-test-v1.json",
    thumbnail: null,
    beam: {
      type: "Wash",
      beam_angle: 13,
      field_angle: 25,
      luminous_flux: 475,
      color_temperature: 5500,
      power: 12,
    },
    about:
      "Battery-powered uplight and spotlight; bricks connect into clusters.",
    venues: [
      {
        name: "built-in",
        fixtures: [
          { name: "Brick1", mode: "8: RGBS" },
          { name: "Brick2", mode: "8: RGBS" },
        ],
      },
      { name: "club", fixtures: [{ name: "Solo", mode: "1: RGB" }] },
    ],
    inspection: GDTF_INSPECTION,
  } satisfies FixtureTypeGdtf);
});

// Testing a fixture from its page. Stateless: the options are derived from
// the type (the PB15's two modes, or a hand-written type's channels), a send
// answers the frame the controls would give, and a release is accepted.
// Tests that check what was sent route their own with `page.route`.
function testModeFor(
  name: string | null,
  channels: Record<string, number>,
  strobe: { min_hz: number; max_hz: number } | null,
): TestMode {
  const has = (c: string) => c in channels;
  const color = has("red") && has("green") && has("blue");
  return {
    name,
    drivable: true,
    error: null,
    footprint: Math.max(0, ...Object.values(channels)),
    controls: {
      color,
      dimmer: has("dimmer") ? "channel" : color ? "folded" : null,
      strobe: has("strobe") ? (strobe ?? { min_hz: 0, max_hz: 20 }) : null,
      pan: has("pan") ? { min: -270, max: 270 } : null,
      tilt: has("tilt") ? { min: -135, max: 135 } : null,
      levels: ["white", "amber", "uv"].filter(has),
    },
    channels: Object.entries(channels)
      .map(([n, offset]) => ({ offset, name: n }))
      .sort((a, b) => a.offset - b.offset),
  };
}

// --- Lighting: LightingService ---
// The lighting pages' facts are fixtures; a save and a delete of a light show
// change nothing, as the REST routes they replace did not; the fixture test
// is stateless, derived from the type (the PB15's two modes, or a
// hand-written type's channels). A spec that needs other answers or the
// requests uses mockRpc (e2e/support/rpc.ts), which is consulted first.

function lightingUnary<I extends DescMessage, O extends DescMessage>(
  method: DescMethodUnary<I, O>,
  answer: (request: MessageShape<I>) => MessageInitShape<O>,
): void {
  grpcUnary(
    method.name,
    (request) =>
      toBinary(
        method.output,
        create(method.output, answer(fromBinary(method.input, request))),
      ),
    "LightingService",
  );
}

lightingUnary(LightingService.method.getLightingReadiness, () =>
  readinessReply(READINESS),
);
lightingUnary(LightingService.method.getLightingFit, () => fitReply(FIT));
lightingUnary(LightingService.method.listLightingGroups, () =>
  groupsReply(LIGHTING_GROUPS),
);
lightingUnary(LightingService.method.validateLightShow, () =>
  validationReply({ valid: true }),
);
lightingUnary(LightingService.method.getLightShow, () =>
  lightShowReply(
    'show "Main" {\n    @00:00.000\n    front: static, color: "blue", duration: 5s\n}\n',
  ),
);
lightingUnary(LightingService.method.saveLightShow, () => ({}));
lightingUnary(LightingService.method.deleteLightShow, () => ({}));

// The 3D view's Preview: an idle show. Tests that need a moment answer their
// own.
lightingUnary(LightingService.method.evaluateSongLighting, (req) =>
  previewReply({
    song: req.song,
    evaluations: req.times.map((time) => ({
      time,
      fixtures: {},
      poses: {},
      cells: {},
      strobe_rates: {},
      active_effects: [],
    })),
    untouched: [],
    venue_error: null,
  }),
);

lightingUnary(LightingService.method.getFixtureTestOptions, (req) => {
  const name = req.fixtureType;
  const entry = FIXTURE_TYPES[name];
  if (!entry) {
    throw new GrpcFailure(Code.NotFound, `no fixture type named "${name}"`);
  }
  const modes = entry.referential
    ? GDTF_INSPECTION.modes.map((m) =>
        testModeFor(
          m.name,
          Object.fromEntries((m.channels ?? []).map(([o, n]) => [n, o])),
          m.strobe_range
            ? {
                min_hz: m.strobe_range.min_hz ?? 0,
                max_hz: m.strobe_range.max_hz,
              }
            : null,
        ),
      )
    : [testModeFor(null, entry.fixture_type.channels, null)];
  return testOptionsReply({
    fixture_type: name,
    gdtf: entry.referential,
    modes,
    default_mode: entry.referential ? "8: RGBS" : null,
    universes: [{ universe: 1, name: "main", patched: true }],
    olad: { reachable: true, port: 9090 },
    available: true,
    unavailable: null,
    unavailable_message: null,
  });
});

lightingUnary(LightingService.method.startFixtureTest, (req) => {
  const address = req.address;
  const hex = req.controls?.color ?? "#000000";
  const rgb = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) || 0);
  const frame = ["red", "green", "blue", "strobe"].map((name, i) => {
    const offset = i + 1;
    const raw = req.raw[String(offset)];
    const value =
      raw !== undefined
        ? raw
        : i < 3
          ? rgb[i]
          : req.controls?.strobe?.hertz
            ? 128
            : 0;
    return {
      offset,
      address: address + i,
      name,
      value,
      raw: raw !== undefined,
    };
  });
  return testAnswerReply({
    active: true,
    universe: req.universe,
    address,
    footprint: 4,
    fixture_type: req.fixtureType,
    mode: req.mode ?? null,
    expires_in_secs: 5,
    frame,
    warnings: [],
  });
});

lightingUnary(LightingService.method.stopFixtureTest, () => ({
  active: false,
}));

// One-step import: no mode, no name. The mock answers as if the archive
// became the mock's "pixelbrick", so the page it opens exists.
app.post("/api/lighting/gdtf/import", (req, res) => {
  res.json({
    type_name: (req.query.name as string) || "pixelbrick",
    fixture: "PB15 PixelBrick",
    manufacturer: "Astera LED Technology",
    modes: 2,
    archive: "lighting/library/pb15.gdtf",
    already_imported: false,
    renamed_from: null,
    refused_modes: [],
    fixture_file: null,
  } satisfies GdtfImport);
});

// MVR import and export (lighting UI design section 11). Tests that need a
// different answer route their own with `page.route`.
app.post("/api/lighting/mvr/inspect", (_req, res) => {
  res.json(MVR_INSPECTION);
});

app.post("/api/lighting/mvr/import", (_req, res) => {
  res.json({
    write: false,
    plan: { ...MVR_INSPECTION.report, origin: [0, 1, 0.5] },
  } satisfies MvrImportAnswer);
});

app.get("/api/lighting/mvr/export/summary", (_req, res) => {
  res.json(MVR_EXPORT_SUMMARY);
});

app.get("/api/lighting/mvr/export", (req, res) => {
  const file = (req.query.file as string) || "kellys.mvr";
  res.set("Content-Type", "application/octet-stream");
  res.set("Content-Disposition", `attachment; filename="${file}"`);
  res.send(Buffer.from("PK-mock-mvr"));
});

// Keeping a copy is its own POST; the first keep of a name succeeds and a
// second is a 409 until `overwrite=true`, like the real server.
const keptExports = new Set<string>();
app.post("/api/lighting/mvr/export/keep", (req, res) => {
  const file = (req.query.file as string) || "kellys.mvr";
  if (keptExports.has(file) && req.query.overwrite !== "true") {
    res.status(409).json({
      error: `lighting/export/${file} already exists`,
      existing: file,
    });
    return;
  }
  keptExports.add(file);
  res.json({ kept: `lighting/export/${file}` } satisfies MvrKept);
});

app.post("/api/lighting/venues/:name/aim-points", (_req, res) => {
  res.json({
    created: [{ name: "Brick 2 aim", fixture: "Brick 2", point: [2, 2.5, 0] }],
    reloaded: false,
    version: "mock-v2",
    venue_error: null,
  } satisfies AimPoints);
});

// The one mesh the mock store serves, read once at startup (the server
// runs from the package root, see playwright.config.ts).
const TEST_BOX_GLB = readFileSync(
  nodePath.resolve(process.cwd(), "e2e/mock-server/box.glb"),
);

// The asset store (design §16.2): one rig model for tests of the 3D view;
// anything else is a 404 like the real store.
app.get("/api/lighting/assets/{*path}", (req, res) => {
  const path = (req.params as { path?: string | string[] }).path;
  const joined = Array.isArray(path) ? path.join("/") : (path ?? "");
  if (joined === "test-archive/rig-test-v1.json") {
    res.set("Content-Type", "application/json");
    res.json(TEST_RIG);
    return;
  }
  if (joined === "scenery/test/scene-v1.json") {
    res.set("Content-Type", "application/json");
    res.json(TEST_SCENERY);
    return;
  }
  if (joined === "scenery/test/models/box.glb") {
    res.set("Content-Type", "model/gltf-binary");
    res.send(TEST_BOX_GLB);
    return;
  }
  res.status(404).json({ error: "Asset not found" });
});

app.get("/api/lighting/venues", (_req, res) => {
  res.json({
    venues: {
      "test-venue": {
        name: "test-venue",
        fixtures: {
          "front-left": {
            name: "front-left",
            fixture_type: "par",
            universe: 1,
            start_channel: 1,
            tags: ["front", "left"],
            position: null,
            rotation: null,
          },
        },
        focus_points: {},
        source: null,
      },
    },
    owners: {
      "test-venue": { file: "test-venue.venue", editable: true, reason: null },
    },
    errors: [],
    versions: { "test-venue": "mock-v1" },
  } satisfies VenueList);
});

// Venue saves are remembered per name so a second edit sees the first,
// the way the real server's file does.
/** A venue save's body, as the page sends it. */
type VenueBody = Parameters<typeof saveVenue>[1];
const savedVenues = new Map<string, VenueBody>();
const MOCK_VENUE: VenueBody = {
  fixtures: [
    {
      name: "front-left",
      fixture_type: "par",
      universe: 1,
      start_channel: 1,
      tags: ["front", "left"],
      position: [-2, 0.5, 3],
      rotation: [0, 0, 0],
    },
  ],
  focus_points: { drummer: [0, 2.8, 1.4] },
  source: null,
};

app.get("/api/lighting/venues/:name", (req, res) => {
  const body = savedVenues.get(req.params.name) ?? MOCK_VENUE;
  const fixtures: Venue["fixtures"] = {};
  for (const f of body.fixtures)
    fixtures[f.name] = {
      ...f,
      mode: f.mode ?? undefined,
      beam_angle: f.beam_angle ?? undefined,
    };
  res.json({
    venue: {
      name: req.params.name,
      fixtures,
      focus_points: body.focus_points ?? {},
      source: body.source ?? null,
    },
    dsl: "venue test-venue {\n  fixture front-left par@1:1 [front, left]\n}",
    version: "mock-v1",
  } satisfies VenueDetail);
});

// What the 3D view draws for a venue that is not the current one: the
// venue's file in the metadata's shape. No rigs here (every fixture draws
// generically); "nowhere" is a venue the server does not have.
app.get("/api/lighting/venues/:name/scene", (req, res) => {
  if (req.params.name === "nowhere") {
    res.status(404).json({ error: "Venue not found" });
    return;
  }
  const body = savedVenues.get(req.params.name) ?? MOCK_VENUE;
  const fixtures: VenueScene["fixtures"] = {};
  for (const f of body.fixtures) {
    fixtures[f.name] = {
      tags: f.tags,
      type: f.fixture_type,
      mode: null,
      position: f.position ?? null,
      rotation: f.rotation ?? null,
      beam_angle: f.beam_angle ?? null,
      rig: null,
    };
  }
  res.json({
    fixtures,
    venue: {
      name: req.params.name,
      focus_points: body.focus_points ?? {},
      scenery: null,
      scenery_error: null,
    },
  } satisfies VenueScene);
});

// Keyed by venue name so parallel tests, each editing its own venue, never
// read each other's save. A single global slot made the stage-view geometry
// tests race whenever two of them ran at once.
const lastVenuePuts = new Map<string, unknown>();
app.put("/api/lighting/venues/:name", (req, res) => {
  lastVenuePuts.set(req.params.name, req.body);
  savedVenues.set(req.params.name, req.body as VenueBody);
  res.json({ status: "saved" });
});
// GET /test/last-venue-put?name=<venue> — what that venue's last save sent,
// for tests of the stage view's geometry editing.
app.get("/test/last-venue-put", (req, res) => {
  const name = String(req.query.name ?? "");
  if (!lastVenuePuts.has(name)) {
    res.json(null);
    return;
  }
  res.json({ name, body: lastVenuePuts.get(name) });
});

app.delete("/api/lighting/venues/:name", (_req, res) => {
  res.json({ status: "deleted" });
});

// --- Browse ---

grpcUnary(
  "BrowseDirectory",
  (request) => {
    fromBinary(BrowseDirectoryRequestSchema, request);
    return toBinary(
      BrowseDirectoryResponseSchema,
      create(
        BrowseDirectoryResponseSchema,
        browseReply({ path: "/songs", root: "/songs", entries: [] }),
      ),
    );
  },
  "ConfigService",
);

grpcUnary(
  "CreateSongInDirectory",
  (request) => {
    const { path, name } = fromBinary(
      CreateSongInDirectoryRequestSchema,
      request,
    );
    return toBinary(
      CreateSongInDirectoryResponseSchema,
      create(
        CreateSongInDirectoryResponseSchema,
        createInDirectoryReply({
          path: `${path}/song.yaml`,
          song: name ?? path.split("/").pop() ?? path,
        }),
      ),
    );
  },
  "ConfigService",
);

grpcUnary(
  "BulkImportSongs",
  (request) => {
    fromBinary(BulkImportSongsRequestSchema, request);
    return toBinary(
      BulkImportSongsResponseSchema,
      create(
        BulkImportSongsResponseSchema,
        bulkImportReply({ created: [], skipped: [], failed: [] }),
      ),
    );
  },
  "ConfigService",
);

// --- Calibration: ConfigService ---

grpcUnary(
  "StartCalibration",
  () =>
    toBinary(
      CalibrationNoiseFloorSchema,
      create(CalibrationNoiseFloorSchema, noiseFloorReply(NOISE_FLOOR)),
    ),
  "ConfigService",
);

grpcUnary(
  "CaptureCalibration",
  () =>
    toBinary(
      CaptureCalibrationResponseSchema,
      create(CaptureCalibrationResponseSchema),
    ),
  "ConfigService",
);

grpcUnary(
  "StopCalibration",
  () =>
    toBinary(
      CalibrationResultSchema,
      create(CalibrationResultSchema, calibrationReply(CALIBRATION)),
    ),
  "ConfigService",
);

grpcUnary(
  "CancelCalibration",
  () =>
    toBinary(
      CancelCalibrationResponseSchema,
      create(CancelCalibrationResponseSchema),
    ),
  "ConfigService",
);

// --- Test Control ---
// How long a test's POST waits for the page it just opened: the browser may
// not have opened its stream yet. Waiting makes the endpoints mean "deliver
// to this wsId", which is what every caller assumes.
const WS_WAIT_MS = 5000;

// --- The stage: the player's gRPC stream WatchStage ---
// The page reads the stage from a gRPC-Web server stream. Each page that
// opens one is held open here, keyed by the wsId of the page it came from
// (the browser names the page in the request's Referer, so the web UI has
// no test hook for it), and written to as tests push (POST /test/send-stage,
// see e2e/support/stage.ts). A page that reconnects starts from the default
// stage again, as a reopened stream gets its opening events again.
/** A page's open stage stream. */
type StageSession = { res: express.Response; stream: StageStream };
const stageSessions = new Map<string, StageSession>();

/** The wsId in the query of the page a request came from, or "". */
function wsIdOfReferer(referer: string | undefined): string {
  try {
    return new URL(referer ?? "").searchParams.get("wsId") ?? "";
  } catch {
    return "";
  }
}

/** Writes `message` into a page's stream. */
function writeStage(session: StageSession, message: StageMessage): void {
  for (const frame of session.stream.push(message)) session.res.write(frame);
}

app.post("/player.v1.PlayerService/WatchStage", (req, res) => {
  const wsId = wsIdOfReferer(req.headers.referer);
  res.status(200).set({
    "content-type": "application/grpc-web+proto",
    "cache-control": "no-cache",
    "x-accel-buffering": "no",
  });
  res.flushHeaders();
  const session: StageSession = { res, stream: new StageStream() };
  // The opening burst: the default venue, and unless the page asked for
  // none ("-nostate": for a test that must see the page before the engine
  // has reported anything), the default fixtures' state.
  writeStage(session, METADATA_STATE);
  if (!wsId.endsWith("-nostate")) writeStage(session, FIXTURE_STATE);
  stageSessions.set(wsId, session);
  // Only if this stream is still the registered one: a reconnect under the
  // same wsId registers the new one first.
  res.on("close", () => {
    if (stageSessions.get(wsId) === session) stageSessions.delete(wsId);
  });
});

async function waitForStage(wsId: string): Promise<StageSession | null> {
  const deadline = Date.now() + WS_WAIT_MS;
  for (;;) {
    const session = stageSessions.get(wsId);
    if (session && !session.res.writableEnded && !session.res.destroyed) {
      return session;
    }
    if (Date.now() >= deadline) return null;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

// Put a stage message (metadata or state) on the page identified by wsId, or
// on every page without one. `sent` is how many streams it went to.
app.post("/test/send-stage", async (req, res) => {
  const { _wsId, ...message } = req.body;
  const wsId = (_wsId as string | undefined) ?? "";
  if (!wsId) {
    for (const session of stageSessions.values()) {
      writeStage(session, message as StageMessage);
    }
    res.json({ sent: stageSessions.size });
    return;
  }
  const session = await waitForStage(wsId);
  if (session) {
    writeStage(session, message as StageMessage);
    res.json({ sent: 1, wsId });
  } else {
    res.json({ sent: 0, wsId });
  }
});

// --- The dashboard's feed: the player's gRPC stream Watch ---
// The page reads its playback, waveform, log lines, reloads and hardware
// status from a gRPC-Web server stream, held open here per page like the
// stage's, keyed by the wsId of the page it came from, and written to as
// tests push (POST /test/send-watch, see e2e/support/watch.ts). A stream
// opens as the player's does: with the playback, the waveform and the
// hardware status (and the log lines the mock has), each the default unless
// the test set the page's own (POST /test/watch-opening).
/** A page's open Watch stream. */
type WatchSession = { res: express.Response };
const watchSessions = new Map<string, WatchSession>();
/** Every open Watch stream, including those of pages that share an id (or
 *  have none): a broadcast reaches all of them. */
const allWatchSessions = new Set<WatchSession>();

/** What a page's stream opens with in place of the defaults, by event. */
const watchOpenings = new Map<string, Map<WatchPush["type"], WatchPush>>();

/** Writes `message` into a page's stream. */
function writeWatch(session: WatchSession, message: WatchPush): void {
  session.res.write(watchFrame(message));
}

app.post("/player.v1.PlayerService/Watch", (req, res) => {
  const wsId = wsIdOfReferer(req.headers.referer);
  res.status(200).set({
    "content-type": "application/grpc-web+proto",
    "cache-control": "no-cache",
    "x-accel-buffering": "no",
  });
  res.flushHeaders();
  const session: WatchSession = { res };
  const own = watchOpenings.get(wsId);
  const opening = (message: WatchPush) => own?.get(message.type) ?? message;
  writeWatch(session, opening(PLAYBACK_STATE as WatchPush));
  writeWatch(session, opening(WAVEFORM_DATA as WatchPush));
  writeWatch(session, opening({ type: "hardware", ...STATUS }));
  writeWatch(session, opening(LOG_LINES as WatchPush));
  watchSessions.set(wsId, session);
  allWatchSessions.add(session);
  // Only if this stream is still the registered one: a reconnect under the
  // same wsId registers the new one first.
  res.on("close", () => {
    allWatchSessions.delete(session);
    if (watchSessions.get(wsId) === session) watchSessions.delete(wsId);
  });
});

async function waitForWatch(wsId: string): Promise<WatchSession | null> {
  const deadline = Date.now() + WS_WAIT_MS;
  for (;;) {
    const session = watchSessions.get(wsId);
    if (session && !session.res.writableEnded && !session.res.destroyed) {
      return session;
    }
    if (Date.now() >= deadline) return null;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

// Put a message on the page identified by wsId, or on every page without
// one. `sent` is how many streams it went to.
app.post("/test/send-watch", async (req, res) => {
  const { _wsId, ...message } = req.body;
  const wsId = (_wsId as string | undefined) ?? "";
  if (!wsId) {
    for (const session of allWatchSessions) {
      writeWatch(session, message as WatchPush);
    }
    res.json({ sent: allWatchSessions.size });
    return;
  }
  const session = await waitForWatch(wsId);
  if (session) {
    writeWatch(session, message as WatchPush);
    res.json({ sent: 1, wsId });
  } else {
    res.json({ sent: 0, wsId });
  }
});

// Set what the streams of the page identified by wsId open with.
app.post("/test/watch-opening", (req, res) => {
  const { _wsId, ...message } = req.body;
  const wsId = (_wsId as string | undefined) ?? "";
  if (!wsId || typeof message.type !== "string") {
    res.status(400).json({ error: "a wsId and a message with a type" });
    return;
  }
  const own = watchOpenings.get(wsId) ?? new Map();
  own.set(message.type, message as WatchPush);
  watchOpenings.set(wsId, own);
  res.json({ set: message.type, wsId });
});

// --- HTTP server ---

const server = createServer(app);

const PORT = 3111;
server.listen(PORT, "127.0.0.1", () => {
  console.log(`Mock server listening on http://127.0.0.1:${PORT}`);
});
