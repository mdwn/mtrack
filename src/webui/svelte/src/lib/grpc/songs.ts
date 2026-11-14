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

// A song's files over gRPC: the calls the song pages and the import pages
// make on `ConfigService` (`GetSongConfig`, `CreateSong`, `SaveSongConfig`,
// `DeleteSong`, `GetSongWaveform`, `GuessSongTempo`, `ListSongFiles`,
// `ImportSongFile`, `BrowseDirectory`, `CreateSongInDirectory`,
// `BulkImportSongs`), mapped into the shapes the components hold (`Browse`,
// `BulkImport`, `SongFile`, `TempoGuess`) and the errors they tell apart. Only
// the mapping lives here, so a unit test can run it without a page; the calls
// are in src/lib/api/songs.ts.

import { Code, ConnectError } from "@connectrpc/connect";
import { ApiError } from "../api/errors";
import {
  FileKind,
  TempoGuessSource,
  type BrowseDirectoryResponse,
  type BulkImportSongsResponse,
  type GetSongConfigResponse,
  type GuessSongTempoResponse,
  type ListSongFilesResponse,
  type TempoTransition,
} from "../../gen/player/v1/player_pb";
import type {
  Browse,
  BulkImport,
  FileKind as WireFileKind,
  GuessedTempo,
  SongFile,
  TempoChangeConfig,
  TempoConfig,
  TempoGuess,
  TransitionConfig,
} from "../../gen/rules/mtrack_rules";
import { callError } from "./library";

/** The metadata key of an `ALREADY_EXISTS` refusal of a song name another
 *  song holds: the name, as bytes (src/controller/grpc/config_songs.rs). */
const TAKEN_NAME_KEY = "x-taken-name-bin";

/** A song name another song holds, refused to a create or a rename. `taken`
 *  is the name. */
export class SongNameTakenError extends ApiError {
  taken: string;
  constructor(message: string, taken: string) {
    super(message, "exists");
    this.name = "SongNameTakenError";
    this.taken = taken;
  }
}

/** The error a failed song call shows ({@link callError}), with a refusal of
 *  a name another song holds as a {@link SongNameTakenError}. A refusal of
 *  something that is there already has `kind` "exists". */
export function songCallError(e: unknown, fallback: string): Error {
  const err = callError(e, fallback);
  if (e instanceof ConnectError && e.code === Code.AlreadyExists) {
    const taken = e.metadata.get(TAKEN_NAME_KEY);
    if (taken !== null) {
      return new SongNameTakenError(err.message, decodeBase64(taken));
    }
  }
  return err;
}

/** A binary metadata value (base64 on the wire) as the UTF-8 text it holds. */
function decodeBase64(value: string): string {
  const binary = atob(value);
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/** A song's config as the editor holds it: the file as written. A song that
 *  is loaded but has no config file now is its summary, as JSON, which is
 *  what the page has always shown for it. */
export function mapSongConfig(r: GetSongConfigResponse): string {
  const s = r.summary;
  if (!s) return r.yaml;
  return JSON.stringify({
    name: s.name,
    duration_ms: Number(s.durationMs),
    duration_display: s.durationDisplay,
    num_channels: s.numChannels,
    sample_format: s.sampleFormat,
    tracks: s.tracks,
    config_file: false,
  });
}

const FILE_KINDS: Partial<Record<FileKind, WireFileKind>> = {
  [FileKind.DIRECTORY]: "directory",
  [FileKind.AUDIO]: "audio",
  [FileKind.MIDI]: "midi",
  [FileKind.LIGHTING]: "lighting",
  [FileKind.OTHER]: "other",
};

function fileKind(kind: FileKind): WireFileKind {
  return FILE_KINDS[kind] ?? "other";
}

/** The files in a song's directory, as the song page lists them. */
export function mapSongFiles(r: ListSongFilesResponse): SongFile[] {
  return r.files.map((f) => ({ name: f.name, type: fileKind(f.kind) }));
}

/** A folder of the project, as the file browser shows it. */
export function mapBrowse(r: BrowseDirectoryResponse): Browse {
  return {
    path: r.path,
    root: r.root,
    entries: r.entries.map((e) => ({
      name: e.name,
      path: e.path,
      type: fileKind(e.kind),
      is_dir: e.isDir,
    })),
  };
}

/** What a bulk import made of each folder. */
export function mapBulkImport(r: BulkImportSongsResponse): BulkImport {
  return {
    created: r.created,
    skipped: r.skipped,
    failed: r.failed.map((f) => ({ name: f.name, error: f.error })),
  };
}

function transition(
  t: TempoTransition | undefined,
): TransitionConfig | undefined {
  switch (t?.ramp.case) {
    case "beats":
      return { beats: t.ramp.value };
    case "measures":
      return { measures: t.ramp.value };
    default:
      return undefined;
  }
}

/** A tempo guess, as the tempo editors take one: what the server leaves out
 *  (a start, a beat at its default, the alignment) is absent, as the song's
 *  `tempo:` block leaves it. */
export function mapTempoGuess(r: GuessSongTempoResponse): TempoGuess {
  const t = r.tempo;
  const c = r.config;
  const tempo: GuessedTempo = {
    start_seconds: t?.startSeconds ?? 0,
    bpm: t?.bpm ?? 0,
    time_signature: [
      t?.timeSignatureNumerator ?? 4,
      t?.timeSignatureDenominator ?? 4,
    ],
    changes: (t?.changes ?? []).map((ch) => {
      const change: GuessedTempo["changes"][number] = {
        measure: ch.measure,
        beat: ch.beat,
        bpm: ch.bpm,
        time_signature: [
          ch.timeSignatureNumerator,
          ch.timeSignatureDenominator,
        ],
      };
      if (ch.transitionBeats !== undefined) {
        change.transition_beats = ch.transitionBeats;
      }
      return change;
    }),
  };
  if (t?.alignmentRmsMs !== undefined)
    tempo.alignment_rms_ms = t.alignmentRmsMs;

  const config: TempoConfig = {
    bpm: c?.bpm ?? 0,
    time_signature: c?.timeSignature ?? "4/4",
  };
  if (c?.start !== undefined) config.start = c.start;
  if (c && c.changes.length > 0) {
    config.changes = c.changes.map((ch) => {
      const change: TempoChangeConfig = { measure: ch.measure };
      if (ch.beat !== undefined) change.beat = ch.beat;
      if (ch.bpm !== undefined) change.bpm = ch.bpm;
      if (ch.timeSignature !== undefined) {
        change.time_signature = ch.timeSignature;
      }
      const ramp = transition(ch.transition);
      if (ramp) change.transition = ramp;
      return change;
    });
  }

  return {
    source: r.source === TempoGuessSource.MIDI ? "midi" : "beat_grid",
    tempo,
    config,
  };
}
