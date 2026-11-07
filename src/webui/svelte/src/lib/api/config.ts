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
  LightFileInfo,
  LightFiles,
  LightingGroup,
  LightingGroups,
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
  PlaylistBody,
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
import { readAnswer } from "../lighting/answer";
import { fixtureTypesChanged, venueChanged } from "../lighting/changes";
import {
  get,
  post,
  put,
  del,
  uploadFile,
  uploadFiles,
  putText,
  postText,
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
  const res = await fetch("/api/lock", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ locked }),
  });
  if (!res.ok) throw new Error(`Failed to set lock state: ${res.status}`);
}

export async function fetchConfigStore(): Promise<StoreSnapshot> {
  const res = await get("/config/store");
  if (!res.ok) throw new Error(`Failed to fetch config store: ${res.status}`);
  return res.json();
}

export async function fetchAudioDevices(): Promise<AudioDeviceInfo[]> {
  const res = await get("/devices/audio");
  if (!res.ok) throw new Error(`Failed to fetch audio devices: ${res.status}`);
  return res.json();
}

/**
 * Opens a device, confirms its output callback runs, and closes it again.
 *
 * Silent: no audio is played, so this is safe to run with the PA up. Takes up
 * to a couple of seconds, since it waits for a callback before giving up.
 */
export async function probeAudioDevice(
  req: ProbeAudioRequest,
): Promise<AudioProbe> {
  const res = await post("/devices/audio/probe", JSON.stringify(req));
  if (!res.ok) throw await apiError(res, "Device test failed");
  return res.json();
}

export async function fetchMidiDevices(): Promise<MidiDeviceInfo[]> {
  const res = await get("/devices/midi");
  if (!res.ok) throw new Error(`Failed to fetch MIDI devices: ${res.status}`);
  return res.json();
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
  const res = await post(
    "/config/profiles",
    JSON.stringify({ expected_checksum: checksum, profile }),
  );
  if (!res.ok) throw await apiError(res, "Failed to add profile");
  return res.json();
}

export async function updateProfile(
  index: number,
  profile: ProfileDraft,
  checksum: string,
): Promise<ConfigSaved> {
  const res = await put(
    `/config/profiles/${index}`,
    JSON.stringify({ expected_checksum: checksum, profile }),
  );
  if (!res.ok) throw await apiError(res, "Failed to update profile");
  return res.json();
}

export async function deleteProfile(
  index: number,
  checksum: string,
): Promise<ConfigSaved> {
  const res = await del(
    `/config/profiles/${index}?expected_checksum=${encodeURIComponent(checksum)}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete profile");
  return res.json();
}

// ---- File-based Profiles API (profiles_dir) ----

export async function fetchProfileFiles(): Promise<ProfileFileInfo[]> {
  const res = await get("/profiles");
  if (!res.ok) throw new Error(`Failed to fetch profiles: ${res.status}`);
  return res.json();
}

export async function fetchProfileFile(filename: string): Promise<ProfileFile> {
  const res = await get(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) throw await apiError(res, "Failed to fetch profile");
  return res.json();
}

/** Saves a profile file. `version` is the one the file was read at; a file
 *  that has changed since is refused with a `ConflictError`. With `create`,
 *  the save is only a create (`If-None-Match: *`): a name that reaches a
 *  file already there is refused, never merged into it. Returns the file's
 *  new version, and what became of the hardware reload. */
export async function saveProfileFile(
  filename: string,
  profile: ProfileDraft,
  version?: string,
  create = false,
): Promise<{ version: string | null; reload: Partial<ConfigReload> }> {
  const headers: Record<string, string> = {};
  if (version) headers["If-Match"] = version;
  if (create) headers["If-None-Match"] = "*";
  const res = await put(
    `/profiles/${encodeURIComponent(filename)}`,
    JSON.stringify(profile),
    Object.keys(headers).length ? headers : undefined,
  );
  if (!res.ok) throw await versionedError(res, "Failed to save profile");
  const body: Partial<FileSaved> = await res.json().catch(() => ({}));
  return { version: body.version ?? null, reload: body };
}

/** Deletes a profile file. Returns what became of the hardware reload. */
export async function deleteProfileFile(
  filename: string,
): Promise<Partial<FileSaved>> {
  const res = await del(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) throw await apiError(res, "Failed to delete profile");
  return res.json().catch(() => ({}));
}

// Samples API

export async function updateSamples(
  samples: Record<string, SampleDefinition>,
  checksum: string,
  maxSampleVoices?: number,
): Promise<ConfigSaved> {
  const body = {
    expected_checksum: checksum,
    samples,
    ...(maxSampleVoices !== undefined
      ? { max_sample_voices: maxSampleVoices }
      : {}),
  };
  const res = await put("/config/samples", JSON.stringify(body));
  if (!res.ok) throw await apiError(res, "Failed to update samples");
  return res.json();
}

export async function updateMetronomeDefaults(
  sounds: MetronomeSounds | null,
  enabled: boolean,
  volume: number | null,
  checksum: string,
): Promise<ConfigSaved> {
  const hasSounds = sounds && Object.keys(sounds).length > 0;
  const hasVolume = volume !== null && volume !== 1;
  const body = {
    expected_checksum: checksum,
    metronome:
      hasSounds || enabled || hasVolume
        ? {
            ...(enabled ? { enabled } : {}),
            ...(hasVolume ? { volume } : {}),
            ...(hasSounds ? { sounds } : {}),
          }
        : null,
  };
  const res = await put("/config/metronome", JSON.stringify(body));
  if (!res.ok) throw await apiError(res, "Failed to update metronome defaults");
  return res.json();
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

export async function startCalibration(
  device: string,
  channel: number,
  duration?: number,
): Promise<NoiseFloor> {
  const body: Record<string, unknown> = { device, channel };
  if (duration !== undefined) body.duration = duration;
  const res = await post("/calibrate/start", JSON.stringify(body));
  if (!res.ok) throw await apiError(res, "Calibration start failed");
  return res.json();
}

export async function startCapture(): Promise<void> {
  const res = await post("/calibrate/capture");
  if (!res.ok) throw await apiError(res, "Capture start failed");
}

export async function stopCapture(): Promise<ChannelCalibration> {
  const res = await post("/calibrate/stop");
  if (!res.ok) throw await apiError(res, "Capture stop failed");
  return res.json();
}

export async function cancelCalibration(): Promise<void> {
  await del("/calibrate");
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
): Promise<void> {
  const params = saveParams(dir, from);
  const res = await put(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
  fixtureTypesChanged();
}

/** Saves a fixture type as raw DSL. `ext: "fixture"` moves a `.light` to a
 *  `.fixture`; a new type is always a `.fixture`. `from` renames. */
export async function saveFixtureTypeText(
  name: string,
  dsl: string,
  ext: "light" | "fixture",
  dir?: string,
  from?: string | null,
): Promise<void> {
  const res = await putText(
    `/lighting/fixture-types/${encodeURIComponent(name)}${saveParams(dir, from, { ext })}`,
    dsl,
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
  fixtureTypesChanged();
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
  const res = await get("/lighting/groups");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting groups");
  const data: LightingGroups = await res.json();
  return data.groups;
}

/** The Lighting overview's five checks, and the facts behind them. */
export async function fetchLightingReadiness(): Promise<Readiness> {
  const res = await get("/lighting/readiness");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting readiness");
  return readAnswer<Readiness>(res, "the readiness checks", [
    "dmx",
    "venue",
    "fixture_types",
    "groups",
    "shows",
    "output",
    "checks",
  ]);
}

/** The facts and suggestions behind the Fit shows page. */
export async function fetchLightingFit(): Promise<FitReport> {
  const res = await get("/lighting/fit");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting fit");
  return readAnswer<FitReport>(res, "the fit", [
    "venue",
    "groups",
    "focus_points_wanted",
    "output",
  ]);
}

/** Venues that parsed, plus the files that did not.
 *
 *  A directory is a set of independent files, so one bad file no longer empties
 *  the list — but it must still be reported, or the only signal is a venue
 *  quietly missing. */
export async function fetchVenues(dir?: string): Promise<VenueList> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(`/lighting/venues${params}`);
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

/** What the 3D view draws for a venue, from its files: the websocket's
 *  metadata shape, for a venue that is not the current one. */
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

/** What a save sets beyond the songs; each left out is kept as it is. */
export type PlaylistSettings = Omit<PlaylistBody, "songs">;

export async function fetchPlaylists(): Promise<PlaylistInfo[]> {
  const res = await get("/playlists");
  if (!res.ok) throw new Error(`Failed to fetch playlists: ${res.status}`);
  return res.json();
}

export async function fetchPlaylist(name: string): Promise<PlaylistDetail> {
  const res = await get(`/playlists/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`Failed to fetch playlist: ${res.status}`);
  return res.json();
}

export async function savePlaylist(
  name: string,
  songs: string[],
  settings: PlaylistSettings = {},
): Promise<void> {
  const res = await put(
    `/playlists/${encodeURIComponent(name)}`,
    JSON.stringify({ songs, ...settings }),
  );
  if (!res.ok) throw await apiError(res, "Failed to save playlist");
}

export async function deletePlaylist(name: string): Promise<void> {
  const res = await del(`/playlists/${encodeURIComponent(name)}`);
  if (!res.ok) throw await apiError(res, "Failed to delete playlist");
}

export async function activatePlaylist(name: string): Promise<void> {
  const res = await post(`/playlists/${encodeURIComponent(name)}/activate`);
  if (!res.ok) throw await apiError(res, "Failed to activate playlist");
}

// ---- Lighting Show Files ----

export async function fetchLightingFiles(): Promise<LightFileInfo[]> {
  const res = await get("/lighting");
  if (!res.ok) throw new Error(`Failed to fetch lighting files: ${res.status}`);
  const data: LightFiles = await res.json();
  return data.files;
}

export async function fetchLightingFile(name: string): Promise<string> {
  const res = await get(`/lighting/${encodeURIComponent(name)}`);
  if (!res.ok) {
    throw new Error(`Failed to fetch lighting file: ${res.status}`);
  }
  return res.text();
}

export async function saveLightingFile(
  name: string,
  content: string,
): Promise<void> {
  const res = await putText(`/lighting/${encodeURIComponent(name)}`, content);
  if (!res.ok) throw await apiError(res, "Failed to save lighting file");
}

export async function deleteLightingFile(name: string): Promise<void> {
  const res = await del(`/lighting/${encodeURIComponent(name)}`);
  if (!res.ok) throw await apiError(res, "Failed to delete lighting file");
}

/** Validates `.light` DSL. Pass `song` when the source belongs to one —
 *  `@bar`/`beat` timing cannot be parsed without that song's tempo map —
 *  and `file`, the lighting file it is a draft of, so its `include`s resolve
 *  where saving it would. */
export async function validateLighting(
  content: string,
  song?: string,
  file?: string,
): Promise<LightingValidation> {
  const query = new URLSearchParams();
  if (song) query.set("song", song);
  if (file) query.set("file", file);
  const qs = query.toString();
  const res = await postText(
    qs ? `/lighting/validate?${qs}` : "/lighting/validate",
    content,
  );
  return res.json();
}
