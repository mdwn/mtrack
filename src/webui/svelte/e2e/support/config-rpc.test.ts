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

// The spec helpers that answer the config calls from the shapes the page
// holds are the inverse of the page's own mapping.
import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import {
  AddSampleFileResponseSchema,
  AddTrackFilesResponseSchema,
  BeginUploadResponseSchema,
  BrowseDirectoryResponseSchema,
  BulkImportSongsResponseSchema,
  GetConfigResponseSchema,
  GetProfileFileResponseSchema,
  GetSongConfigResponseSchema,
  GuessSongTempoResponseSchema,
  ListProfileFilesResponseSchema,
  ListSongFilesResponseSchema,
  ProfileFileSavedSchema,
  UpdateConfigResponseSchema,
  WaveformSchema,
} from "../../src/gen/player/v1/player_pb";
import {
  mapBrowse,
  mapBulkImport,
  mapSongConfig,
  mapSongFiles,
  mapTempoGuess,
} from "../../src/lib/grpc/songs";
import { mapWaveform } from "../../src/lib/grpc/watch";
import {
  mapSampleUploaded,
  mapTrackUploaded,
} from "../../src/lib/grpc/uploads";
import {
  mapConfigSaved,
  mapConfigStore,
  mapFileSaved,
  mapProfileFile,
  mapProfileFiles,
} from "../../src/lib/grpc/config";
import {
  addSampleFileReply,
  addTrackFilesReply,
  beginUploadReply,
  browseReply,
  bulkImportReply,
  getConfigReply,
  getProfileFileReply,
  getSongConfigReply,
  listProfileFilesReply,
  listSongFilesReply,
  profileFileSavedReply,
  savedReply,
  tempoGuessReply,
  waveformReply,
} from "./config-rpc";

describe("answers made from the page's own shapes", () => {
  it("a config read reads back as it was", () => {
    const snapshot = { yaml: "songs: s\n", checksum: "abc" };
    expect(
      mapConfigStore(create(GetConfigResponseSchema, getConfigReply(snapshot))),
    ).toEqual(snapshot);
  });

  it("an applied save reads back as one", () => {
    const snapshot = { yaml: "y", checksum: "c" };
    expect(
      mapConfigSaved(create(UpdateConfigResponseSchema, savedReply(snapshot))),
    ).toEqual({ ...snapshot, applied: true, reload: "applied" });
  });

  it("a deferred save reads back with what it waits for", () => {
    const snapshot = { yaml: "y", checksum: "c" };
    const reload = {
      applied: false,
      reload: "deferred",
      message: "waits",
      songs_error: "no songs",
      awaits_reload: 2,
      awaits_process: "99",
    };
    expect(
      mapConfigSaved(
        create(UpdateConfigResponseSchema, savedReply(snapshot, reload)),
      ),
    ).toEqual({ ...snapshot, ...reload });
  });
});

describe("answers made from the profile files' own shapes", () => {
  it("the list reads back as it was", () => {
    const files = [
      {
        file: "a.yaml",
        filename: "a",
        addressable: true,
        error: null,
        hostname: "host-a",
        has_audio: true,
        has_midi: false,
        has_dmx: false,
        has_trigger: true,
        has_controllers: false,
      },
      {
        file: "b.yml",
        filename: "b",
        addressable: false,
        error: "does not parse",
        hostname: null,
        has_audio: false,
        has_midi: false,
        has_dmx: false,
        has_trigger: false,
        has_controllers: false,
      },
    ];
    expect(
      mapProfileFiles(
        create(ListProfileFilesResponseSchema, listProfileFilesReply(files)),
      ),
    ).toEqual(files);
  });

  it("a file reads back as it was", () => {
    const file = {
      profile: { hostname: "h", audio: { device: "d" } },
      yaml: "hostname: h\n",
      version: "v1",
    };
    expect(
      mapProfileFile(
        create(GetProfileFileResponseSchema, getProfileFileReply(file)),
      ),
    ).toEqual(file);
  });

  it("a save reads back with its version and an applied reload", () => {
    expect(
      mapFileSaved(
        create(
          ProfileFileSavedSchema,
          profileFileSavedReply({ filename: "a", version: "v2" }),
        ),
      ),
    ).toEqual({
      status: "saved",
      filename: "a",
      version: "v2",
      applied: true,
      reload: "applied",
    });
  });

  it("a delete reads back with no version and a deferred reload", () => {
    const reload = {
      applied: false,
      reload: "deferred",
      message: "waits",
      awaits_reload: 1,
      awaits_process: "7",
    };
    expect(
      mapFileSaved(
        create(
          ProfileFileSavedSchema,
          profileFileSavedReply({ status: "deleted", filename: "a" }, reload),
        ),
      ),
    ).toEqual({ status: "deleted", filename: "a", ...reload });
  });

  it("a tempo guess reads back as it was", () => {
    const guess = {
      source: "midi" as const,
      tempo: {
        start_seconds: 0.25,
        bpm: 120,
        time_signature: [4, 4] as [number, number],
        alignment_rms_ms: 3.5,
        changes: [
          {
            measure: 5,
            beat: 2,
            bpm: 140,
            time_signature: [6, 8] as [number, number],
            transition_beats: 4,
          },
        ],
      },
      config: {
        bpm: 120,
        time_signature: "4/4",
        start: 0.25,
        changes: [
          {
            measure: 5,
            beat: 2,
            bpm: 140,
            time_signature: "6/8",
            transition: { measures: 2 },
          },
        ],
      },
    };
    expect(
      mapTempoGuess(
        create(GuessSongTempoResponseSchema, tempoGuessReply(guess)),
      ),
    ).toEqual(guess);
  });

  it("a folder, a file list, a bulk import and a waveform read back as they were", () => {
    const browse = {
      path: "/songs",
      root: "/p",
      entries: [
        {
          name: "A",
          path: "/songs/A",
          type: "directory" as const,
          is_dir: true,
        },
        {
          name: "a.mid",
          path: "/songs/a.mid",
          type: "midi" as const,
          is_dir: false,
        },
      ],
    };
    expect(
      mapBrowse(create(BrowseDirectoryResponseSchema, browseReply(browse))),
    ).toEqual(browse);
    const files = [
      { name: "a.wav", type: "audio" as const },
      { name: "b.light", type: "lighting" as const },
    ];
    expect(
      mapSongFiles(
        create(ListSongFilesResponseSchema, listSongFilesReply(files)),
      ),
    ).toEqual(files);
    const imported = {
      created: ["A"],
      skipped: ["B"],
      failed: [{ name: "C", error: "x" }],
    };
    expect(
      mapBulkImport(
        create(BulkImportSongsResponseSchema, bulkImportReply(imported)),
      ),
    ).toEqual(imported);
    const waveform = {
      song_name: "S",
      tracks: [{ name: "kick", peaks: [0.5, 1] }],
    };
    expect(
      mapWaveform(create(WaveformSchema, waveformReply(waveform))),
    ).toEqual(waveform);
    expect(
      mapSongConfig(
        create(GetSongConfigResponseSchema, getSongConfigReply("y")),
      ),
    ).toBe("y");
  });
});

describe("the uploads' answers", () => {
  it("an announced upload carries its id and the path of its bytes", () => {
    const begun = create(BeginUploadResponseSchema, beginUploadReply("abc123"));
    expect([begun.uploadId, begun.path]).toEqual(["abc123", "/uploads/abc123"]);
  });

  it("track files added read back through the page's mapping", () => {
    expect(
      mapTrackUploaded(
        create(
          AddTrackFilesResponseSchema,
          addTrackFilesReply({
            song: "Song",
            files: [{ file: "kick.wav", replaced: true }],
          }),
        ),
      ),
    ).toEqual({
      status: "replaced",
      replaced: true,
      song: "Song",
      file: "kick.wav",
    });
  });

  it("a stored sample reads back through the page's mapping", () => {
    expect(
      mapSampleUploaded(
        create(
          AddSampleFileResponseSchema,
          addSampleFileReply({ file: "k.wav", path: "samples/k.wav" }),
        ),
      ),
    ).toEqual({ status: "uploaded", file: "k.wav", path: "samples/k.wav" });
  });
});
