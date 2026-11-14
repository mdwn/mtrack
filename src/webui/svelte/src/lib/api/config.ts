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

import type {
  AimPoints,
  AudioDeviceInfo,
  AudioProbe,
  ChannelCalibration,
  ConfigReload,
  ConfigSaved,
  Controller,
  FileSaved,
  FitReport,
  FixtureSettingsData,
  FixtureType,
  FixtureTypeDeleted,
  FixtureTypeEntry,
  FixtureTypeGdtf,
  FixtureTypeList,
  GdtfImport,
  LightingGroup,
  LightingValidation,
  MetronomeSounds,
  MidiDeviceInfo,
  MovementLimits,
  MvrExportSummary,
  MvrImportAnswer,
  MvrInspection,
  MvrKeep,
  NoiseFloor,
  PlaylistDetail,
  PlaylistInfo,
  ProbeAudioRequest,
  Profile,
  ProfileFile,
  ProfileFileInfo,
  Readiness,
  SampleDefinition,
  SampleUploaded,
  SettingsSaved,
  FixtureTypeSaved,
  StoreSnapshot,
  Vec3,
  VenueDetail,
  VenueError,
  VenueList,
  VenuePatch,
  VenueSaved,
  VenueScene,
} from "../../gen/rules/mtrack_rules";
import { Code, ConnectError } from "@connectrpc/connect";
import { configClient, lightingClient, playerClient } from "../grpc/client";
import {
  mapAudioDevices,
  mapAudioProbe,
  mapCalibration,
  mapConfigSaved,
  mapConfigStore,
  mapFileSaved,
  mapMidiDevices,
  mapNoiseFloor,
  mapProfileFile,
  mapProfileFiles,
  metronomeJson,
  probeAudioRequest,
  startCalibrationRequest,
} from "../grpc/config";
import {
  mapFit,
  mapFixtureSettings,
  mapFixtureTypeDeleted,
  mapFixtureTypeEntry,
  mapFixtureTypeGdtf,
  mapFixtureTypeList,
  mapFixtureTypeSaved,
  mapAimPoints,
  mapGroups,
  mapMvrExportSummary,
  mapReadiness,
  mapSettingsSaved,
  mapVenueDetail,
  mapVenueList,
  mapVenuePatch,
  mapVenueSaved,
  mapVenueScene,
  saveFixtureSettingsRequest,
  saveFixtureTypeRequest,
  saveVenueRequest,
  type VenueBody,
} from "../grpc/lighting";
import {
  importCallError,
  mapGdtfImport,
  mapMvrImportAnswer,
  mapMvrInspection,
  mapSampleUploaded,
  mvrOptions,
} from "../grpc/uploads";
import { sendUpload } from "./upload";
import { UploadPurpose } from "../../gen/player/v1/player_pb";
import { BadAnswerError } from "../lighting/answer";
import {
  callError,
  mapPlaylistDetail,
  mapPlaylistList,
  savePlaylistRequest,
  type PlaylistSettings,
} from "../grpc/library";
import { fixtureTypesChanged, venueChanged } from "../lighting/changes";

export { ConflictError } from "./rest";

/** Describes a device's channel count without overstating what is known. */
export function channelsLabel(device: AudioDeviceInfo): string {
  if (device.channels_known) return `${device.max_channels}ch`;
  return device.virtual_node ? "virtual" : `${device.max_channels}ch+`;
}

export async function setLocked(locked: boolean): Promise<void> {
  try {
    await playerClient.setLock({ locked });
  } catch (e) {
    throw callError(e, "Failed to set lock state");
  }
}

export async function fetchConfigStore(): Promise<StoreSnapshot> {
  try {
    return mapConfigStore(await playerClient.getConfig({}));
  } catch (e) {
    throw callError(e, "Failed to fetch config store");
  }
}

export async function fetchAudioDevices(): Promise<AudioDeviceInfo[]> {
  try {
    return mapAudioDevices(await configClient.listAudioDevices({}));
  } catch (e) {
    throw callError(e, "Failed to fetch audio devices");
  }
}

/**
 * Opens a device, confirms its output callback runs, and closes it again.
 *
 * Silent: no audio is played, so this is safe to run with the PA up. Takes up
 * to a couple of seconds, since it waits for a callback before giving up
 * (longer when another test is running: they run one at a time). Nothing in
 * the client times the call out.
 */
export async function probeAudioDevice(
  req: ProbeAudioRequest,
): Promise<AudioProbe> {
  try {
    return mapAudioProbe(
      await configClient.probeAudioDevice(probeAudioRequest(req)),
    );
  } catch (e) {
    throw callError(e, "Device test failed");
  }
}

export async function fetchMidiDevices(): Promise<MidiDeviceInfo[]> {
  try {
    return mapMidiDevices(await configClient.listMidiDevices({}));
  } catch (e) {
    throw callError(e, "Failed to fetch MIDI devices");
  }
}

/** A profile as the editor holds it: the generated `Profile`, a section
 *  set to `null` to remove it on save (the server drops a section sent as
 *  null, and keeps one left out). */
export type ProfileDraft = Omit<Profile, "controllers"> & {
  controllers?: Controller[] | null;
};

export async function addProfile(
  profile: ProfileDraft,
  checksum: string,
): Promise<ConfigSaved> {
  try {
    return mapConfigSaved(
      await playerClient.addProfile({
        profileJson: JSON.stringify(profile),
        expectedChecksum: checksum,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to add profile");
  }
}

export async function updateProfile(
  index: number,
  profile: ProfileDraft,
  checksum: string,
): Promise<ConfigSaved> {
  try {
    return mapConfigSaved(
      await playerClient.updateProfile({
        index,
        profileJson: JSON.stringify(profile),
        expectedChecksum: checksum,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to update profile");
  }
}

export async function deleteProfile(
  index: number,
  checksum: string,
): Promise<ConfigSaved> {
  try {
    return mapConfigSaved(
      await playerClient.removeProfile({
        index,
        expectedChecksum: checksum,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to delete profile");
  }
}

// ---- File-based Profiles API (profiles_dir) ----

export async function fetchProfileFiles(): Promise<ProfileFileInfo[]> {
  try {
    return mapProfileFiles(await configClient.listProfileFiles({}));
  } catch (e) {
    throw callError(e, "Failed to fetch profiles");
  }
}

export async function fetchProfileFile(filename: string): Promise<ProfileFile> {
  try {
    return mapProfileFile(await configClient.getProfileFile({ filename }));
  } catch (e) {
    throw callError(e, "Failed to fetch profile");
  }
}

/** Saves a profile file. `version` is the one the file was read at; a file
 *  that has changed since is refused with a `ConflictError`. With `create`,
 *  the save is only a create: a name that reaches a file already there is
 *  refused (also a `ConflictError`), never merged into it. Returns the file's
 *  new version, and what became of the hardware reload. */
export async function saveProfileFile(
  filename: string,
  profile: ProfileDraft,
  version?: string,
  create = false,
): Promise<{ version: string | null; reload: Partial<ConfigReload> }> {
  try {
    const saved = mapFileSaved(
      await configClient.saveProfileFile({
        filename,
        profileJson: JSON.stringify(profile),
        expectedVersion: version || undefined,
        create,
      }),
    );
    return { version: saved.version ?? null, reload: saved };
  } catch (e) {
    throw callError(e, "Failed to save profile");
  }
}

/** Deletes a profile file. Returns what became of the hardware reload. */
export async function deleteProfileFile(
  filename: string,
): Promise<Partial<FileSaved>> {
  try {
    return mapFileSaved(await configClient.deleteProfileFile({ filename }));
  } catch (e) {
    throw callError(e, "Failed to delete profile");
  }
}

// Samples API

export async function updateSamples(
  samples: Record<string, SampleDefinition>,
  checksum: string,
  maxSampleVoices?: number,
): Promise<ConfigSaved> {
  try {
    return mapConfigSaved(
      await configClient.updateSamples({
        samplesJson: JSON.stringify(samples),
        maxSampleVoices,
        expectedChecksum: checksum,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to update samples");
  }
}

export async function updateMetronomeDefaults(
  sounds: MetronomeSounds | null,
  enabled: boolean,
  volume: number | null,
  checksum: string,
): Promise<ConfigSaved> {
  try {
    return mapConfigSaved(
      await configClient.updateMetronome({
        metronomeJson: metronomeJson(sounds, enabled, volume),
        expectedChecksum: checksum,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to update metronome defaults");
  }
}

/** Uploads a sample file into the project's `samples/` directory: announced,
 *  its bytes sent to the web server, then stored. Answers the path a sample
 *  definition names it by. */
export async function uploadSampleFile(file: File): Promise<SampleUploaded> {
  const fail = (e: unknown) => callError(e, "Failed to upload sample");
  const id = await sendUpload({ purpose: UploadPurpose.SAMPLE }, file, fail);
  try {
    return mapSampleUploaded(
      await configClient.addSampleFile({ uploadId: id }),
    );
  } catch (e) {
    throw fail(e);
  }
}

// Calibration API

/** Records the channel's noise for `duration` seconds (3 by default, up to
 *  30), so the call takes that long: nothing in the client times it out. */
export async function startCalibration(
  device: string,
  channel: number,
  duration?: number,
): Promise<NoiseFloor> {
  try {
    return mapNoiseFloor(
      await configClient.startCalibration(
        startCalibrationRequest(device, channel, duration),
      ),
    );
  } catch (e) {
    throw callError(e, "Calibration start failed");
  }
}

export async function startCapture(): Promise<void> {
  try {
    await configClient.captureCalibration({});
  } catch (e) {
    throw callError(e, "Capture start failed");
  }
}

export async function stopCapture(): Promise<ChannelCalibration> {
  try {
    return mapCalibration(await configClient.stopCalibration({}));
  } catch (e) {
    throw callError(e, "Capture stop failed");
  }
}

export async function cancelCalibration(): Promise<void> {
  await configClient.cancelCalibration({});
}

/** Restarts the controllers from the current config. The new status arrives
 *  on the `Watch` stream. Refused during playback. */
export async function restartControllers(): Promise<void> {
  try {
    await configClient.restartControllers({});
  } catch (e) {
    throw callError(e, "Restart failed");
  }
}

/** The sample rates and bit depths offered when the device does not say
 *  which it takes. Suggestions, not a rule: the player takes any rate or
 *  depth above zero (`Audio`'s validation) and the device decides. */
export const COMMON_SAMPLE_RATES = [44100, 48000, 96000];
export const COMMON_BIT_DEPTHS = [16, 24, 32];

// Lighting fixture type & venue API

/** `?dir=…&venues_dir=…`, each only when given, then `more`. */
export function dirParams(
  dir?: string,
  venuesDir?: string,
  more: Record<string, string> = {},
): string {
  const params = new URLSearchParams();
  if (dir) params.set("dir", dir);
  if (venuesDir) params.set("venues_dir", venuesDir);
  for (const [key, value] of Object.entries(more)) params.set(key, value);
  const query = params.toString();
  return query ? `?${query}` : "";
}

export async function fetchFixtureTypes(
  dir?: string,
  venuesDir?: string,
): Promise<FixtureTypeList> {
  try {
    return mapFixtureTypeList(
      await lightingClient.listFixtureTypes({
        dir: dir || undefined,
        venuesDir: venuesDir || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to fetch fixture types");
  }
}

/** Imports a GDTF in one step: the fixture is usable in a venue in any of
 *  its modes at once. A `name` pins the fixture's name in a record; none is
 *  the norm. */
export async function importGdtf(
  file: File,
  name?: string,
): Promise<GdtfImport> {
  const fail = (e: unknown) => importCallError(e, "Failed to import GDTF");
  const id = await sendUpload(
    { purpose: UploadPurpose.GDTF_IMPORT },
    file,
    fail,
  );
  let report: GdtfImport;
  try {
    report = mapGdtfImport(
      await lightingClient.importGdtf({
        uploadId: id,
        name: name || undefined,
      }),
    );
  } catch (e) {
    throw fail(e);
  }
  fixtureTypesChanged();
  return report;
}

// --- MVR import and export (lighting UI design section 11) ---

/** Where the lighting directories are, when a profile moves them. */
export interface MvrDirs {
  venuesDir?: string;
  fixtureTypesDir?: string;
}

/** Reads an MVR: what an import would do and how the file looks from above.
 *  Writes nothing; refuses what is not an MVR with the parser's reason. The
 *  file is sent again at each step (the server keeps nothing between them). */
export async function inspectMvr(
  file: File,
  name: string | undefined,
  dirs: MvrDirs = {},
): Promise<MvrInspection> {
  const fail = (e: unknown) => importCallError(e, "Failed to read the MVR");
  const id = await sendUpload(
    { purpose: UploadPurpose.MVR_INSPECT },
    file,
    fail,
  );
  try {
    return mapMvrInspection(
      await lightingClient.inspectMvr({
        uploadId: id,
        options: mvrOptions({ name, ...dirs }),
      }),
    );
  } catch (e) {
    throw fail(e);
  }
}

/** The import's plan (`write` false) or its report (`write` true). */
export async function importMvr(
  file: File,
  options: { name: string; origin: Vec3; write: boolean; keep?: MvrKeep },
  dirs: MvrDirs = {},
): Promise<MvrImportAnswer> {
  const fail = (e: unknown) => importCallError(e, "Failed to import the MVR");
  const id = await sendUpload(
    { purpose: UploadPurpose.MVR_IMPORT },
    file,
    fail,
  );
  let answer: MvrImportAnswer;
  try {
    answer = mapMvrImportAnswer(
      await lightingClient.importMvr({
        uploadId: id,
        write: options.write,
        options: mvrOptions({
          name: options.name,
          origin: options.origin,
          keep: options.keep,
          ...dirs,
        }),
      }),
    );
  } catch (e) {
    throw fail(e);
  }
  if (options.write) {
    // A write adds types and writes (or merges into) a venue.
    fixtureTypesChanged();
    venueChanged(null);
  }
  return answer;
}

/** What an export would hold, without sending the archive. */
export async function fetchMvrExportSummary(
  venue: string,
  dirs: MvrDirs = {},
): Promise<MvrExportSummary> {
  try {
    return mapMvrExportSummary(
      await lightingClient.getMvrExportSummary({
        venue,
        venuesDir: dirs.venuesDir || undefined,
        fixtureTypesDir: dirs.fixtureTypesDir || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to summarize the export");
  }
}

/** Stages a venue's MVR export for download: answers the path on the web
 *  server to fetch it from (any number of times, for ten minutes) and the
 *  name it carries. Nothing is written to the project; keeping a copy there
 *  is `keepMvrExport`. */
export async function downloadMvrExport(
  venue: string,
  options: { file: string; layersFromTags: boolean },
  dirs: MvrDirs = {},
): Promise<{ path: string; fileName: string }> {
  try {
    const staged = await lightingClient.exportMvr({
      venue,
      file: options.file || undefined,
      layersFromTags: options.layersFromTags,
      venuesDir: dirs.venuesDir || undefined,
      fixtureTypesDir: dirs.fixtureTypesDir || undefined,
    });
    return { path: staged.path, fileName: staged.fileName };
  } catch (e) {
    throw callError(e, "Failed to export the venue");
  }
}

/** The project path of a kept copy, or the name of the file a keep would
 *  have replaced (the server's 409). */
export type KeepResult =
  | { status: "kept"; path: string }
  | { status: "exists"; existing: string };

/** Writes the export under `lighting/export/`. An existing file is left
 *  alone (`exists`) unless `overwrite` says to replace it. */
export async function keepMvrExport(
  venue: string,
  options: { file: string; layersFromTags: boolean; overwrite?: boolean },
  dirs: MvrDirs = {},
): Promise<KeepResult> {
  try {
    const kept = await lightingClient.keepMvrExport({
      venue,
      file: options.file,
      layersFromTags: options.layersFromTags,
      overwrite: options.overwrite ?? false,
      venuesDir: dirs.venuesDir || undefined,
      fixtureTypesDir: dirs.fixtureTypesDir || undefined,
    });
    switch (kept.outcome.case) {
      case "kept":
        return { status: "kept", path: kept.outcome.value };
      case "existing":
        return { status: "exists", existing: kept.outcome.value };
      default:
        throw new BadAnswerError("a kept export");
    }
  } catch (e) {
    if (e instanceof BadAnswerError) throw e;
    throw callError(e, "Failed to keep a copy of the export");
  }
}

/** Adds a focus point for each unlinked fixed fixture, where its rest beam
 *  meets the deck. */
export async function addAimPoints(
  venue: string,
  dirs: MvrDirs = {},
  version?: string,
): Promise<AimPoints> {
  let added: AimPoints;
  try {
    added = mapAimPoints(
      await lightingClient.addVenueAimPoints({
        name: venue,
        venuesDir: dirs.venuesDir || undefined,
        fixtureTypesDir: dirs.fixtureTypesDir || undefined,
        expectedRevision: version || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to add aim points");
  }
  venueChanged(venue);
  return added;
}

export async function fetchFixtureType(
  name: string,
  dir?: string,
): Promise<FixtureTypeEntry> {
  try {
    return mapFixtureTypeEntry(
      await lightingClient.getFixtureType({ name, dir: dir || undefined }),
      false,
    );
  } catch (e) {
    throw callError(e, "Failed to fetch fixture type");
  }
}

export async function fetchFixtureTypeGdtf(
  name: string,
  dir?: string,
  venuesDir?: string,
): Promise<FixtureTypeGdtf> {
  try {
    return mapFixtureTypeGdtf(
      await lightingClient.getFixtureTypeGdtfDetails({
        name,
        dir: dir || undefined,
        venuesDir: venuesDir || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to read the GDTF archive");
  }
}

export async function fetchFixtureSettings(
  name: string,
  dir?: string,
): Promise<FixtureSettingsData> {
  try {
    return mapFixtureSettings(
      await lightingClient.getFixtureTypeSettings({
        name,
        dir: dir || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to read the fixture's settings");
  }
}

/** Plans (`write: false`) or saves a GDTF type's settings. A save sends the
 *  file's version and the venue files' versions from the plan; either
 *  having changed is a `ConflictError`. */
export async function postFixtureSettings(
  name: string,
  body: {
    name: string;
    movement: MovementLimits;
    write: boolean;
    venue_versions?: Record<string, string>;
  },
  dirs: { dir?: string; venuesDir?: string },
  version?: string,
): Promise<SettingsSaved> {
  let saved: SettingsSaved;
  try {
    saved = mapSettingsSaved(
      await lightingClient.saveFixtureTypeSettings(
        saveFixtureSettingsRequest(name, body, dirs, version),
      ),
    );
  } catch (e) {
    throw callError(e, "Failed to save the fixture's settings");
  }
  if (body.write) {
    // A rename rewrites venue lines; a new default changes fixtures' modes.
    fixtureTypesChanged();
    venueChanged(null);
  }
  return saved;
}

export async function saveFixtureType(
  name: string,
  data: Pick<FixtureType, "channels"> &
    Partial<
      Pick<
        FixtureType,
        "max_strobe_frequency" | "min_strobe_frequency" | "strobe_dmx_offset"
      >
    >,
  dir?: string,
  from?: string | null,
  venuesDir?: string,
): Promise<FixtureTypeSaved> {
  let saved: FixtureTypeSaved;
  try {
    saved = mapFixtureTypeSaved(
      await lightingClient.saveFixtureType(
        saveFixtureTypeRequest(name, data, { dir, from, venuesDir }),
      ),
    );
  } catch (e) {
    throw callError(e, "Failed to save fixture type");
  }
  typeSaved(from);
  return saved;
}

/** After a type's save: a rename rewrote the venue lines naming it too. */
function typeSaved(from: string | null | undefined): void {
  fixtureTypesChanged();
  if (from) venueChanged(null);
}

/** Saves a fixture type as raw DSL. `ext: "fixture"` moves a `.light` to a
 *  `.fixture`; a new type is always a `.fixture`. `from` renames, and the
 *  venue lines naming the old name in `venuesDir` are retyped with it. */
export async function saveFixtureTypeText(
  name: string,
  dsl: string,
  ext: "light" | "fixture",
  dir?: string,
  from?: string | null,
  venuesDir?: string,
): Promise<FixtureTypeSaved> {
  let saved: FixtureTypeSaved;
  try {
    saved = mapFixtureTypeSaved(
      await lightingClient.saveFixtureType(
        saveFixtureTypeRequest(
          name,
          { text: dsl },
          { ext, dir, from, venuesDir },
        ),
      ),
    );
  } catch (e) {
    throw callError(e, "Failed to save fixture type");
  }
  typeSaved(from);
  return saved;
}

/** Deletes a fixture type. For a fixture from a GDTF, its archive goes too
 *  unless another fixture uses it. */
export async function deleteFixtureType(
  name: string,
  dir?: string,
): Promise<FixtureTypeDeleted> {
  let deleted: FixtureTypeDeleted;
  try {
    deleted = mapFixtureTypeDeleted(
      await lightingClient.deleteFixtureType({ name, dir: dir || undefined }),
    );
  } catch (e) {
    throw callError(e, "Failed to delete fixture type");
  }
  fixtureTypesChanged();
  return deleted;
}

/// Group names a cue can target, with the fixtures each currently resolves to.
/// These are the logical groups from `dmx.lighting.groups`; venues no longer
/// define groups of their own.
export async function fetchLightingGroups(): Promise<LightingGroup[]> {
  try {
    return mapGroups(await lightingClient.listLightingGroups({}));
  } catch (e) {
    throw callError(e, "Failed to fetch lighting groups");
  }
}

/** The Lighting overview's five checks, and the facts behind them. */
export async function fetchLightingReadiness(): Promise<Readiness> {
  try {
    return mapReadiness(await lightingClient.getLightingReadiness({}));
  } catch (e) {
    throw callError(e, "Failed to fetch lighting readiness");
  }
}

/** The facts and suggestions behind the Fit shows page. */
export async function fetchLightingFit(): Promise<FitReport> {
  try {
    return mapFit(await lightingClient.getLightingFit({}));
  } catch (e) {
    throw callError(e, "Failed to fetch lighting fit");
  }
}

/** How long a venue list is waited for before it is given up on (a
 *  `TimeoutError`): a server that never answers would leave a field
 *  disabled, or a spinner turning, for good. */
const VENUES_TIMEOUT_MS = 10_000;

/** Venues that parsed, plus the files that did not.
 *
 *  A directory is a set of independent files, so one bad file no longer empties
 *  the list — but it must still be reported, or the only signal is a venue
 *  quietly missing. Gives up after `VENUES_TIMEOUT_MS`. */
export async function fetchVenues(dir?: string): Promise<VenueList> {
  const signal = AbortSignal.timeout(VENUES_TIMEOUT_MS);
  try {
    return mapVenueList(
      await lightingClient.listVenues({ dir: dir || undefined }, { signal }),
    );
  } catch (e) {
    // The caller tells a list that never came from one that was refused.
    if (signal.aborted) {
      throw new DOMException("The venue list did not arrive", "TimeoutError");
    }
    throw callError(e, "Failed to fetch venues");
  }
}

export async function fetchVenue(
  name: string,
  dir?: string,
): Promise<VenueDetail> {
  try {
    return mapVenueDetail(
      await lightingClient.getVenue({ name, dir: dir || undefined }),
    );
  } catch (e) {
    throw callError(e, "Failed to fetch venue");
  }
}

export async function saveVenue(
  name: string,
  data: VenueBody,
  dir?: string,
  version?: string,
  /** The venue's old name: the save renames it. */
  from?: string | null,
): Promise<SavedVenue> {
  let saved: VenueSaved;
  try {
    saved = mapVenueSaved(
      await lightingClient.saveVenue(
        saveVenueRequest(name, data, { dir, from, version }),
      ),
    );
  } catch (e) {
    throw callError(e, "Failed to save venue");
  }
  // A rename changes two venues' views: every one is stale.
  venueChanged(from && from !== name ? null : name);
  return {
    version: saved.version ?? null,
    venueError: saved.venue_error ?? null,
  };
}

/** What a venue save answers: the file's new version, and the venue's
 *  load failure when the save left the current venue failing. */
export interface SavedVenue {
  version: string | null;
  venueError: VenueError | null;
}

/** The addresses a venue's fixtures occupy, from the files (no engine). */
export async function fetchVenuePatch(
  name: string,
  dir?: string,
  fixtureTypesDir?: string,
): Promise<VenuePatch> {
  try {
    return mapVenuePatch(
      await lightingClient.getVenuePatch({
        name,
        dir: dir || undefined,
        fixtureTypesDir: fixtureTypesDir || undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to read the venue's patch");
  }
}

/** What the 3D view draws for a venue, from its files: the stage
 *  metadata's shape, for a venue that is not the current one. */
/** A venue the server does not have (404), told apart from a failed read. */
export class VenueNotFoundError extends Error {}

export async function fetchVenueScene(
  name: string,
  dir?: string,
  fixtureTypesDir?: string,
): Promise<VenueScene> {
  try {
    return mapVenueScene(
      await lightingClient.getVenueScene({
        name,
        dir: dir || undefined,
        fixtureTypesDir: fixtureTypesDir || undefined,
      }),
    );
  } catch (e) {
    const err = callError(e, "Failed to read the venue's scene");
    if (e instanceof ConnectError && e.code === Code.NotFound) {
      throw new VenueNotFoundError(err.message);
    }
    throw err;
  }
}

export async function deleteVenue(name: string, dir?: string): Promise<void> {
  try {
    await lightingClient.deleteVenue({ name, dir: dir || undefined });
  } catch (e) {
    throw callError(e, "Failed to delete venue");
  }
  venueChanged(name);
}

// ---- Playlist CRUD ----

/** A playlist's (or an entry's) ambient light: `null` for the profile's (an
 *  entry: the playlist's), `false` for none, or settings of its own. */
export type PlaylistAmbient = PlaylistDetail["ambient"];

/** An entry's auto play: `true` for at once, a gap (`"2s"`), or `null`. */
export type AutoPlay = PlaylistDetail["auto_play"][number];

export type { PlaylistSettings };

export async function fetchPlaylists(): Promise<PlaylistInfo[]> {
  try {
    return mapPlaylistList(
      await playerClient.listPlaylists({ detailed: true }),
    );
  } catch (e) {
    throw callError(e, "Failed to fetch playlists");
  }
}

export async function fetchPlaylist(name: string): Promise<PlaylistDetail> {
  try {
    return mapPlaylistDetail(
      await playerClient.getPlaylist({ name, withLibrary: true }),
    );
  } catch (e) {
    throw callError(e, "Failed to fetch playlist");
  }
}

/** Saves `songs` (and `settings`) as the playlist `name`, which must exist;
 *  with `create`, makes it and refuses a name that is taken. */
export async function savePlaylist(
  name: string,
  songs: string[],
  settings: PlaylistSettings = {},
  create = false,
): Promise<void> {
  try {
    await playerClient.savePlaylist(
      savePlaylistRequest(name, songs, settings, create),
    );
  } catch (e) {
    throw callError(e, "Failed to save playlist");
  }
}

export async function deletePlaylist(name: string): Promise<void> {
  try {
    await playerClient.deletePlaylist({ name });
  } catch (e) {
    throw callError(e, "Failed to delete playlist");
  }
}

export async function activatePlaylist(name: string): Promise<void> {
  try {
    await playerClient.switchToPlaylist({ playlistName: name });
  } catch (e) {
    throw callError(e, "Failed to activate playlist");
  }
}

// ---- Lighting Show Files ----

export async function fetchLightingFile(name: string): Promise<string> {
  try {
    return (await lightingClient.getLightShow({ name })).source;
  } catch (e) {
    throw callError(e, "Failed to fetch lighting file");
  }
}

export async function saveLightingFile(
  name: string,
  content: string,
): Promise<void> {
  try {
    await lightingClient.saveLightShow({ name, source: content });
  } catch (e) {
    throw callError(e, "Failed to save lighting file");
  }
}

export async function deleteLightingFile(name: string): Promise<void> {
  try {
    await lightingClient.deleteLightShow({ name });
  } catch (e) {
    throw callError(e, "Failed to delete lighting file");
  }
}

/** Validates `.light` DSL. Pass `song` when the source belongs to one —
 *  `@bar`/`beat` timing cannot be parsed without that song's tempo map —
 *  and `file`, the lighting file it is a draft of, so its `include`s resolve
 *  where saving it would. The problems are the answer; a call that cannot be
 *  checked (a `file` outside the songs directory) throws. */
export async function validateLighting(
  content: string,
  song?: string,
  file?: string,
): Promise<LightingValidation> {
  try {
    const r = await lightingClient.validateLightShow({
      source: content,
      song: song || undefined,
      file: file || undefined,
    });
    return r.valid ? { valid: true } : { valid: false, errors: [...r.errors] };
  } catch (e) {
    throw callError(e, "Failed to validate lighting");
  }
}
