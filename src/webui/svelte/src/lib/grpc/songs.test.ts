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

import { describe, expect, it } from "vitest";
import { Code, ConnectError } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  BrowseDirectoryResponseSchema,
  BulkImportSongsResponseSchema,
  FileKind,
  GetSongConfigResponseSchema,
  GuessSongTempoResponseSchema,
  ListSongFilesResponseSchema,
  TempoGuessSource,
  WaveformSchema,
} from "../../gen/player/v1/player_pb";
import { ApiError, ConflictError } from "../api/errors";
import {
  mapBrowse,
  mapBulkImport,
  mapSongConfig,
  mapSongFiles,
  mapTempoGuess,
  SongNameTakenError,
  songCallError,
} from "./songs";
import { mapWaveform } from "./watch";

describe("a song's config", () => {
  it("is the file as written", () => {
    const yaml = "name: A\ntracks: []\n";
    expect(mapSongConfig(create(GetSongConfigResponseSchema, { yaml }))).toBe(
      yaml,
    );
  });

  it("is the summary as JSON for a song with no file now", () => {
    const text = mapSongConfig(
      create(GetSongConfigResponseSchema, {
        summary: {
          name: "NoConfig",
          durationMs: 1500n,
          durationDisplay: "0:01",
          numChannels: 2,
          sampleFormat: "int",
          tracks: ["a", "b"],
        },
      }),
    );
    expect(JSON.parse(text)).toEqual({
      name: "NoConfig",
      duration_ms: 1500,
      duration_display: "0:01",
      num_channels: 2,
      sample_format: "int",
      tracks: ["a", "b"],
      config_file: false,
    });
  });
});

describe("a song's files and the browser's listing", () => {
  it("name each file's kind as the page holds it", () => {
    const files = mapSongFiles(
      create(ListSongFilesResponseSchema, {
        files: [
          { name: "a.wav", kind: FileKind.AUDIO },
          { name: "b.mid", kind: FileKind.MIDI },
          { name: "c.light", kind: FileKind.LIGHTING },
          { name: "d.txt", kind: FileKind.OTHER },
        ],
      }),
    );
    expect(files).toEqual([
      { name: "a.wav", type: "audio" },
      { name: "b.mid", type: "midi" },
      { name: "c.light", type: "lighting" },
      { name: "d.txt", type: "other" },
    ]);
  });

  it("carry a folder's entries, directories marked", () => {
    const browse = mapBrowse(
      create(BrowseDirectoryResponseSchema, {
        path: "/songs",
        root: "/srv/project",
        entries: [
          {
            name: "Afar",
            path: "/songs/Afar",
            kind: FileKind.DIRECTORY,
            isDir: true,
          },
          {
            name: "x.wav",
            path: "/songs/x.wav",
            kind: FileKind.AUDIO,
            isDir: false,
          },
        ],
      }),
    );
    expect(browse).toEqual({
      path: "/songs",
      root: "/srv/project",
      entries: [
        { name: "Afar", path: "/songs/Afar", type: "directory", is_dir: true },
        { name: "x.wav", path: "/songs/x.wav", type: "audio", is_dir: false },
      ],
    });
  });
});

describe("a bulk import", () => {
  it("lists what was made, skipped and failed", () => {
    expect(
      mapBulkImport(
        create(BulkImportSongsResponseSchema, {
          created: ["A"],
          skipped: ["B"],
          failed: [{ name: "C", error: "no audio" }],
        }),
      ),
    ).toEqual({
      created: ["A"],
      skipped: ["B"],
      failed: [{ name: "C", error: "no audio" }],
    });
  });
});

describe("a waveform", () => {
  it("is the tracks' peaks as plain numbers", () => {
    const wave = mapWaveform(
      create(WaveformSchema, {
        songName: "S",
        tracks: [{ name: "t", peaks: [0.5, 1] }],
      }),
    );
    expect(wave).toEqual({
      song_name: "S",
      tracks: [{ name: "t", peaks: [0.5, 1] }],
    });
    expect(Array.isArray(wave.tracks[0].peaks)).toBe(true);
  });
});

describe("a tempo guess", () => {
  it("leaves out what the server leaves out", () => {
    const guess = mapTempoGuess(
      create(GuessSongTempoResponseSchema, {
        source: TempoGuessSource.BEAT_GRID,
        tempo: {
          startSeconds: 0.5,
          bpm: 120,
          timeSignatureNumerator: 4,
          timeSignatureDenominator: 4,
        },
        config: { bpm: 120, timeSignature: "4/4", start: 0.5 },
      }),
    );
    expect(guess).toEqual({
      source: "beat_grid",
      tempo: {
        start_seconds: 0.5,
        bpm: 120,
        time_signature: [4, 4],
        changes: [],
      },
      config: { bpm: 120, time_signature: "4/4", start: 0.5 },
    });
    expect("alignment_rms_ms" in guess.tempo).toBe(false);
    expect("changes" in guess.config).toBe(false);
  });

  it("carries the changes, the transitions and the alignment", () => {
    const guess = mapTempoGuess(
      create(GuessSongTempoResponseSchema, {
        source: TempoGuessSource.MIDI,
        tempo: {
          bpm: 100,
          timeSignatureNumerator: 6,
          timeSignatureDenominator: 8,
          alignmentRmsMs: 4.5,
          changes: [
            {
              measure: 5,
              beat: 2,
              bpm: 140,
              timeSignatureNumerator: 4,
              timeSignatureDenominator: 4,
              transitionBeats: 8,
            },
            {
              measure: 9,
              beat: 1,
              bpm: 90,
              timeSignatureNumerator: 4,
              timeSignatureDenominator: 4,
            },
          ],
        },
        config: {
          bpm: 100,
          timeSignature: "6/8",
          changes: [
            {
              measure: 5,
              beat: 2,
              bpm: 140,
              timeSignature: "4/4",
              transition: { ramp: { case: "beats", value: 8 } },
            },
            {
              measure: 9,
              transition: { ramp: { case: "measures", value: 2 } },
            },
            { measure: 12, bpm: 80 },
          ],
        },
      }),
    );
    expect(guess.source).toBe("midi");
    expect(guess.tempo.alignment_rms_ms).toBe(4.5);
    expect(guess.tempo.time_signature).toEqual([6, 8]);
    expect(guess.tempo.changes).toEqual([
      {
        measure: 5,
        beat: 2,
        bpm: 140,
        time_signature: [4, 4],
        transition_beats: 8,
      },
      { measure: 9, beat: 1, bpm: 90, time_signature: [4, 4] },
    ]);
    expect(guess.config.changes).toEqual([
      {
        measure: 5,
        beat: 2,
        bpm: 140,
        time_signature: "4/4",
        transition: { beats: 8 },
      },
      { measure: 9, transition: { measures: 2 } },
      { measure: 12, bpm: 80 },
    ]);
  });
});

describe("a failed song call", () => {
  it("shows the server's message, and says when something was there already", () => {
    const e = songCallError(
      new ConnectError("Song already exists: A", Code.AlreadyExists),
      "Failed",
    );
    expect(e).toBeInstanceOf(ApiError);
    expect(e).not.toBeInstanceOf(SongNameTakenError);
    expect(e.message).toBe("Song already exists: A");
    expect((e as ApiError).kind).toBe("exists");
  });

  it("carries the name another song holds, whatever its characters", () => {
    const metadata = new Headers();
    // "Café" as the bytes of a binary header, base64.
    const bytes = new TextEncoder().encode("Café");
    metadata.set("x-taken-name-bin", btoa(String.fromCharCode(...bytes)));
    const e = songCallError(
      new ConnectError(
        'a song named "Café" already exists; choose another name',
        Code.AlreadyExists,
        metadata,
      ),
      "Failed",
    );
    expect(e).toBeInstanceOf(SongNameTakenError);
    expect((e as SongNameTakenError).taken).toBe("Café");
    expect(e.message).toContain("choose another name");
  });

  it("shows a refused config's problems, and falls back to the code", () => {
    expect(
      songCallError(
        new ConnectError(
          "name must not be empty; no tracks",
          Code.InvalidArgument,
        ),
        "Save failed",
      ).message,
    ).toBe("name must not be empty; no tracks");
    expect(
      songCallError(new ConnectError("", Code.Internal), "Save failed").message,
    ).toBe("Save failed: Internal");
  });

  it("is a conflict for a change elsewhere, as every save's is", () => {
    expect(
      songCallError(new ConnectError("changed", Code.Aborted), "Failed"),
    ).toBeInstanceOf(ConflictError);
  });
});
