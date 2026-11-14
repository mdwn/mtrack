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
  Fixture,
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
  MvrKept,
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
  Venue,
  VenueDetail,
  VenueError,
  VenueList,
  VenuePatch,
  VenueSaved,
  VenueScene,
} from "../../gen/rules/mtrack_rules";
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
import { mapFit, mapGroups, mapReadiness } from "../grpc/lighting";
import {
  callError,
  mapPlaylistDetail,
  mapPlaylistList,
  savePlaylistRequest,
  type PlaylistSettings,
} from "../grpc/library";
import { fixtureTypesChanged, venueChanged } from "../lighting/changes";
import {
  get,
  post,
  put,
  del,
  uploadFile,
  uploadFiles,
  putText,
  apiError,
  versionedError,
} from "./rest";
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

export async function uploadSampleFile(file: File): Promise<SampleUploaded> {
  const res = await uploadFile(
    `/samples/upload/${encodeURIComponent(file.name)}`,
    file,
  );
  if (!res.ok) throw await apiError(res, "Failed to upload sample");
  return res.json();
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
  const params = dirParams(dir, venuesDir);
  const res = await get(`/lighting/fixture-types${params}`);
  if (!res.ok) throw await apiError(res, "Failed to fetch fixture types");
  return res.json();
}

/** Imports a GDTF in one step: the fixture is usable in a venue in any of
 *  its modes at once. A `mode` makes one the default; none is the norm. */
export async function importGdtf(
  file: File,
  name?: string,
): Promise<GdtfImport> {
  const params = new URLSearchParams();
  if (name) params.set("name", name);
  const res = await uploadFiles(`/lighting/gdtf/import?${params}`, [file]);
  if (!res.ok) throw await apiError(res, "Failed to import GDTF");
  fixtureTypesChanged();
  return res.json();
}

// --- MVR import and export (lighting UI design section 11) ---

/** Where the lighting directories are, when a profile moves them. */
export interface MvrDirs {
  venuesDir?: string;
  fixtureTypesDir?: string;
}

function mvrForm(
  file: File,
  fields: Record<string, string | undefined>,
  dirs: MvrDirs,
): FormData {
  const form = new FormData();
  for (const [key, value] of Object.entries(fields)) {
    if (value) form.append(key, value);
  }
  if (dirs.venuesDir) form.append("venues_dir", dirs.venuesDir);
  if (dirs.fixtureTypesDir)
    form.append("fixture_types_dir", dirs.fixtureTypesDir);
  form.append("file", file, file.name);
  return form;
}

/** Reads an MVR: what an import would do and how the file looks from above.
 *  Writes nothing; refuses what is not an MVR with the parser's reason. */
export async function inspectMvr(
  file: File,
  name: string | undefined,
  dirs: MvrDirs = {},
): Promise<MvrInspection> {
  const res = await fetch("/api/lighting/mvr/inspect", {
    method: "POST",
    body: mvrForm(file, { name }, dirs),
  });
  if (!res.ok) throw await apiError(res, "Failed to read the MVR");
  return res.json();
}

/** The import's plan (`write` false) or its report (`write` true). */
export async function importMvr(
  file: File,
  options: { name: string; origin: Vec3; write: boolean; keep?: MvrKeep },
  dirs: MvrDirs = {},
): Promise<MvrImportAnswer> {
  const res = await fetch("/api/lighting/mvr/import", {
    method: "POST",
    body: mvrForm(
      file,
      {
        name: options.name,
        origin: options.origin.join(","),
        write: options.write ? "true" : "false",
        keep: options.keep ? JSON.stringify(options.keep) : undefined,
      },
      dirs,
    ),
  });
  if (!res.ok) throw await apiError(res, "Failed to import the MVR");
  if (options.write) {
    // A write adds types and writes (or merges into) a venue.
    fixtureTypesChanged();
    venueChanged(null);
  }
  return res.json();
}

function exportParams(
  venue: string,
  extra: Record<string, string | undefined>,
  dirs: MvrDirs,
): string {
  const params = new URLSearchParams({ venue });
  for (const [key, value] of Object.entries(extra)) {
    if (value !== undefined) params.set(key, value);
  }
  if (dirs.venuesDir) params.set("venues_dir", dirs.venuesDir);
  if (dirs.fixtureTypesDir)
    params.set("fixture_types_dir", dirs.fixtureTypesDir);
  return params.toString();
}

/** What an export would hold, without sending the archive. */
export async function fetchMvrExportSummary(
  venue: string,
  dirs: MvrDirs = {},
): Promise<MvrExportSummary> {
  const res = await get(
    `/lighting/mvr/export/summary?${exportParams(venue, {}, dirs)}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to summarize the export");
  return res.json();
}

/** Downloads a venue as an MVR: the archive as a blob. A GET, so it writes
 *  nothing; keeping a copy in the project is `keepMvrExport`. */
export async function downloadMvrExport(
  venue: string,
  options: { file: string; layersFromTags: boolean },
  dirs: MvrDirs = {},
): Promise<{ blob: Blob; fileName: string }> {
  const params = exportParams(
    venue,
    {
      file: options.file,
      layers_from_tags: String(options.layersFromTags),
    },
    dirs,
  );
  const res = await get(`/lighting/mvr/export?${params}`);
  if (!res.ok) throw await apiError(res, "Failed to export the venue");
  return { blob: await res.blob(), fileName: options.file };
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
  const params = exportParams(
    venue,
    {
      file: options.file,
      layers_from_tags: String(options.layersFromTags),
      overwrite: options.overwrite ? "true" : undefined,
    },
    dirs,
  );
  const res = await post(`/lighting/mvr/export/keep?${params}`);
  if (res.status === 409) {
    const body = await res.json().catch(() => ({}));
    return {
      status: "exists",
      existing:
        typeof body.existing === "string" ? body.existing : options.file,
    };
  }
  if (!res.ok) throw await apiError(res, "Failed to keep a copy of the export");
  const body: MvrKept = await res.json();
  return { status: "kept", path: body.kept };
}

/** Adds a focus point for each unlinked fixed fixture, where its rest beam
 *  meets the deck. */
export async function addAimPoints(
  venue: string,
  dirs: MvrDirs = {},
  version?: string,
): Promise<AimPoints> {
  const params = new URLSearchParams();
  if (dirs.venuesDir) params.set("venues_dir", dirs.venuesDir);
  if (dirs.fixtureTypesDir)
    params.set("fixture_types_dir", dirs.fixtureTypesDir);
  const query = params.toString();
  const res = await post(
    `/lighting/venues/${encodeURIComponent(venue)}/aim-points${query ? `?${query}` : ""}`,
    undefined,
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok) throw await versionedError(res, "Failed to add aim points");
  venueChanged(venue);
  return res.json();
}

export async function fetchFixtureType(
  name: string,
  dir?: string,
): Promise<FixtureTypeEntry> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to fetch fixture type");
  return res.json();
}

export async function fetchFixtureTypeGdtf(
  name: string,
  dir?: string,
  venuesDir?: string,
): Promise<FixtureTypeGdtf> {
  const params = dirParams(dir, venuesDir);
  const res = await get(
    `/lighting/fixture-types/${encodeURIComponent(name)}/gdtf${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to read the GDTF archive");
  return res.json();
}

export async function fetchFixtureSettings(
  name: string,
  dir?: string,
): Promise<FixtureSettingsData> {
  const params = dirParams(dir);
  const res = await get(
    `/lighting/fixture-types/${encodeURIComponent(name)}/settings${params}`,
  );
  if (!res.ok)
    throw await apiError(res, "Failed to read the fixture's settings");
  return res.json();
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
  const res = await post(
    `/lighting/fixture-types/${encodeURIComponent(name)}/settings${dirParams(dirs.dir, dirs.venuesDir)}`,
    JSON.stringify(body),
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok)
    throw await versionedError(res, "Failed to save the fixture's settings");
  if (body.write) {
    // A rename rewrites venue lines; a new default changes fixtures' modes.
    fixtureTypesChanged();
    venueChanged(null);
  }
  return res.json();
}

/** The query of a venue or fixture type save: its directory, and the old
 *  name when the save renames (the server moves or renames the one file). */
function saveParams(
  dir: string | undefined,
  from: string | null | undefined,
  extra: Record<string, string> = {},
): string {
  const params = new URLSearchParams(extra);
  if (dir) params.set("dir", dir);
  if (from) params.set("from", from);
  const query = params.toString();
  return query ? `?${query}` : "";
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
  const params = saveParams(dir, from, typeRenameParams(from, venuesDir));
  const res = await put(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
  typeSaved(from);
  return res.json();
}

/** The venues directory a type's rename retypes the venue lines in. */
function typeRenameParams(
  from: string | null | undefined,
  venuesDir: string | undefined,
): Record<string, string> {
  return from && venuesDir ? { venues_dir: venuesDir } : {};
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
  const params = saveParams(dir, from, {
    ext,
    ...typeRenameParams(from, venuesDir),
  });
  const res = await putText(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
    dsl,
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
  typeSaved(from);
  return res.json();
}

/** Deletes a fixture type. For a fixture from a GDTF, its archive goes too
 *  unless another fixture uses it. */
export async function deleteFixtureType(
  name: string,
  dir?: string,
): Promise<FixtureTypeDeleted> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete fixture type");
  fixtureTypesChanged();
  return res.json();
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
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/venues${params}`,
    AbortSignal.timeout(VENUES_TIMEOUT_MS),
  );
  if (!res.ok) throw await apiError(res, "Failed to fetch venues");
  return res.json();
}

export async function fetchVenue(
  name: string,
  dir?: string,
): Promise<VenueDetail> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to fetch venue");
  return res.json();
}

export async function saveVenue(
  name: string,
  data: {
    /** Each as the venue serves it, a `null` mode or beam angle for none
     *  (the server reads null and absent alike). */
    fixtures: (Omit<Fixture, "mode" | "beam_angle"> & {
      mode?: string | null;
      beam_angle?: number | null;
    })[];
    focus_points?: Venue["focus_points"];
    source?: Venue["source"];
  },
  dir?: string,
  version?: string,
  /** The venue's old name: the save renames it. */
  from?: string | null,
): Promise<SavedVenue> {
  const params = saveParams(dir, from);
  const res = await put(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok) throw await versionedError(res, "Failed to save venue");
  // A rename changes two venues' views: every one is stale.
  venueChanged(from && from !== name ? null : name);
  const body: Partial<VenueSaved> = await res.json().catch(() => ({}));
  return {
    version: body.version ?? null,
    venueError: body.venue_error ?? null,
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
  const params = new URLSearchParams();
  if (dir) params.set("dir", dir);
  if (fixtureTypesDir) params.set("fixture_types_dir", fixtureTypesDir);
  const query = params.toString();
  const res = await get(
    `/lighting/venues/${encodeURIComponent(name)}/patch${query ? `?${query}` : ""}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to read the venue's patch");
  return res.json();
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
  const params = new URLSearchParams();
  if (dir) params.set("dir", dir);
  if (fixtureTypesDir) params.set("fixture_types_dir", fixtureTypesDir);
  const query = params.toString();
  const res = await get(
    `/lighting/venues/${encodeURIComponent(name)}/scene${query ? `?${query}` : ""}`,
  );
  if (res.status === 404)
    throw new VenueNotFoundError((await apiError(res, "")).message);
  if (!res.ok) throw await apiError(res, "Failed to read the venue's scene");
  return res.json();
}

export async function deleteVenue(name: string, dir?: string): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete venue");
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
