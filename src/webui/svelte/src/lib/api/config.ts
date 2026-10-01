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

import { BadAnswerError, readJson } from "../lighting/answer";
import { parseFit, type Fit } from "../lighting/fit";
import { parseReadiness, type Readiness } from "../lighting/readiness";
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

// Calibration types
export interface NoiseFloorResult {
  peak: number;
  rms: number;
  low_freq_energy: number;
  channel: number;
  sample_rate: number;
  device_channels: number;
}

export interface ChannelCalibration {
  channel: number;
  threshold: number;
  gain: number;
  scan_time_ms: number;
  retrigger_time_ms: number;
  highpass_freq?: number;
  dynamic_threshold_decay_ms?: number;
  num_hits_detected: number;
  noise_floor_peak: number;
  max_hit_amplitude: number;
}

export interface SupportedFormat {
  sample_format: string;
  bits_per_sample: number;
}

export interface AudioDeviceInfo {
  name: string;
  max_channels: number;
  host_name: string;
  supported_sample_rates: number[];
  supported_formats: SupportedFormat[];
  /** An ALSA plug/virtual node. Still selectable — some rigs need one. */
  virtual_node: boolean;
  /** False when max_channels is a floor rather than the real maximum. */
  channels_known: boolean;
}

/** Describes a device's channel count without overstating what is known. */
export function channelsLabel(device: AudioDeviceInfo): string {
  if (device.channels_known) return `${device.max_channels}ch`;
  return device.virtual_node ? "virtual" : `${device.max_channels}ch+`;
}

export interface MidiDeviceInfo {
  name: string;
  has_input: boolean;
  has_output: boolean;
}

export interface ConfigSnapshot {
  yaml: string;
  checksum: string;
}

export async function setLocked(locked: boolean): Promise<void> {
  const res = await fetch("/api/lock", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ locked }),
  });
  if (!res.ok) throw new Error(`Failed to set lock state: ${res.status}`);
}

export async function fetchConfigStore(): Promise<ConfigSnapshot> {
  const res = await get("/config/store");
  if (!res.ok) throw new Error(`Failed to fetch config store: ${res.status}`);
  return res.json();
}

export async function fetchAudioDevices(): Promise<AudioDeviceInfo[]> {
  const res = await get("/devices/audio");
  if (!res.ok) throw new Error(`Failed to fetch audio devices: ${res.status}`);
  return res.json();
}

/** What a device did when asked to actually stream. */
export type ProbeOutcome =
  | "streaming"
  | "opened_but_silent"
  | "could_not_open"
  | "opened_cannot_report"
  /** Already open by this player, so it was reported rather than probed. */
  | "in_use";

export interface AudioProbeResult {
  outcome: ProbeOutcome;
  /** Whether the device proved it can stream. */
  ok: boolean;
  /** Why it could not be opened. Only ever set for `could_not_open`. */
  reason?: string | null;
  callbacks?: number | null;
}

/** The audio settings a probe honours, mirroring the profile's audio block. */
export interface AudioProbeRequest {
  device: string;
  sample_rate?: number;
  sample_format?: string;
  bits_per_sample?: number;
  buffer_size?: number;
  stream_buffer_size?: string | number;
}

/**
 * Opens a device, confirms its output callback runs, and closes it again.
 *
 * Silent: no audio is played, so this is safe to run with the PA up. Takes up
 * to a couple of seconds, since it waits for a callback before giving up.
 */
export async function probeAudioDevice(
  req: AudioProbeRequest,
): Promise<AudioProbeResult> {
  const res = await post("/devices/audio/probe", JSON.stringify(req));
  if (!res.ok) throw await apiError(res, "Device test failed");
  return res.json();
}

export async function fetchMidiDevices(): Promise<MidiDeviceInfo[]> {
  const res = await get("/devices/midi");
  if (!res.ok) throw new Error(`Failed to fetch MIDI devices: ${res.status}`);
  return res.json();
}

export async function addProfile(
  profile: object,
  checksum: string,
): Promise<ConfigSnapshot> {
  const res = await post(
    "/config/profiles",
    JSON.stringify({ expected_checksum: checksum, profile }),
  );
  if (!res.ok) throw await apiError(res, "Failed to add profile");
  return res.json();
}

export async function updateProfile(
  index: number,
  profile: object,
  checksum: string,
): Promise<ConfigSnapshot> {
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
): Promise<ConfigSnapshot> {
  const res = await del(
    `/config/profiles/${index}?expected_checksum=${encodeURIComponent(checksum)}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete profile");
  return res.json();
}

// ---- File-based Profiles API (profiles_dir) ----

export interface ProfileFileInfo {
  filename: string;
  hostname: string | null;
  has_audio: boolean;
  has_midi: boolean;
  has_dmx: boolean;
  has_trigger: boolean;
  has_controllers: boolean;
}

export async function fetchProfileFiles(): Promise<ProfileFileInfo[]> {
  const res = await get("/profiles");
  if (!res.ok) throw new Error(`Failed to fetch profiles: ${res.status}`);
  return res.json();
}

export async function fetchProfileFile(
  filename: string,
): Promise<{ profile: object; yaml: string; version?: string }> {
  const res = await get(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) throw await apiError(res, "Failed to fetch profile");
  return res.json();
}

/** Saves a profile file. `version` is the one the file was read at; a file
 *  that has changed since is refused with a `ConflictError`. Returns the
 *  file's new version. */
export async function saveProfileFile(
  filename: string,
  profile: object,
  version?: string,
): Promise<string | null> {
  const res = await put(
    `/profiles/${encodeURIComponent(filename)}`,
    JSON.stringify(profile),
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok) throw await versionedError(res, "Failed to save profile");
  const body = await res.json().catch(() => ({}));
  return body?.version ?? null;
}

export async function deleteProfileFile(filename: string): Promise<void> {
  const res = await del(`/profiles/${encodeURIComponent(filename)}`);
  if (!res.ok) throw await apiError(res, "Failed to delete profile");
}

// Samples API

export async function updateSamples(
  samples: Record<string, unknown>,
  checksum: string,
  maxSampleVoices?: number,
): Promise<ConfigSnapshot> {
  const body: Record<string, unknown> = {
    expected_checksum: checksum,
    samples,
  };
  if (maxSampleVoices !== undefined) {
    body.max_sample_voices = maxSampleVoices;
  }
  const res = await put("/config/samples", JSON.stringify(body));
  if (!res.ok) throw await apiError(res, "Failed to update samples");
  return res.json();
}

export async function updateMetronomeDefaults(
  sounds: Record<string, unknown> | null,
  enabled: boolean,
  volume: number | null,
  checksum: string,
): Promise<ConfigSnapshot> {
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

export interface SampleUploadResult {
  status: string;
  file: string;
  path: string;
}

export async function uploadSampleFile(
  file: File,
): Promise<SampleUploadResult> {
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
): Promise<NoiseFloorResult> {
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

// Lighting fixture type & venue API

export interface FixtureTypeData {
  name: string;
  channels: Record<string, number>;
  max_strobe_frequency: number | null;
  min_strobe_frequency: number | null;
  strobe_dmx_offset: number | null;
}

/** A stage triple: meters (or degrees, for a rotation). */
export type Vec3 = [number, number, number];

export interface FixtureData {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
  /** Stage position, when the venue places the fixture. */
  position?: Vec3 | null;
  /** Mounting rotation in degrees about X, Y, Z. */
  rotation?: Vec3 | null;
  /** The fixture's own GDTF mode (`mode "…"` on its line); absent or null
   *  is its type's default. Must survive every save untouched. */
  mode?: string | null;
}

/** Where a venue was seeded from (an MVR import). */
export interface VenueSource {
  mvr: string;
  origin: Vec3;
}

export interface VenueData {
  name: string;
  fixtures: Record<string, FixtureData>;
  /** Named stage points, the positional analog of tags. */
  focus_points?: Record<string, Vec3>;
  source?: VenueSource | null;
}

/** A fixture type as the directory listing carries it: the parsed type plus
 *  the file it came from. The form decides how it may be edited — a
 *  `.fixture` type is text-only, and a referential one has no channels of
 *  its own until the lighting system expands its GDTF archive. */
export interface FixtureTypeEntry {
  fixture_type: FixtureTypeData;
  file: string;
  extension: string;
  referential: boolean;
  /** A GDTF type's default mode; null on a native type and on a GDTF type
   *  with none, whose fixtures each name their own. */
  default_mode?: string | null;
  /** The addresses a fixture of the type occupies (a GDTF type's in its
   *  default mode); null when not known. */
  footprint?: number | null;
  rich: boolean;
  /** A referential type's archive in brief, for its card; null when the
   *  archive is missing or does not parse. Absent on a native type. */
  gdtf?: GdtfSummary | null;
}

/** What a referential type's card says about its archive. */
export interface GdtfSummary {
  fixture: string;
  manufacturer: string;
  /** How many modes the archive has. */
  modes: number;
  /** The type's default mode, as the `.fixture` writes it; null when the
   *  type has none (every fixture of it names its own). */
  mode: string | null;
  /** The modes venue fixtures use, most-used first. */
  in_use: { mode: string; count: number }[];
  /** The pinned mode's first beam, as far as the archive states it. */
  beam: { type: string | null; angle: number | null } | null;
  /** In the asset store, once a rig has been made for the type. */
  thumbnail: string | null;
  /** Venue fixtures of this type. */
  used_by: number;
}

/** `?dir=…&venues_dir=…`, each only when given. */
function dirParams(dir?: string, venuesDir?: string): string {
  const params = new URLSearchParams();
  if (dir) params.set("dir", dir);
  if (venuesDir) params.set("venues_dir", venuesDir);
  const query = params.toString();
  return query ? `?${query}` : "";
}

export async function fetchFixtureTypes(
  dir?: string,
  venuesDir?: string,
): Promise<{
  fixtureTypes: Record<string, FixtureTypeEntry>;
  errors: LightingFileError[];
}> {
  const params = dirParams(dir, venuesDir);
  const res = await get(`/lighting/fixture-types${params}`);
  if (!res.ok) throw await apiError(res, "Failed to fetch fixture types");
  const data = await res.json();
  return {
    fixtureTypes: data.fixture_types ?? {},
    errors: data.errors ?? [],
  };
}

/** One DMX mode of a GDTF archive, as the inspect endpoint describes it. */
export interface GdtfMode {
  name: string;
  channel_count: number;
  /** DMX addresses the mode occupies. */
  footprint: number;
  /** What a show can do in the mode, named as the fit endpoint names
   *  capabilities: `color`, `dimmer`, `strobe`, `pan_tilt`, `white`, `zoom`,
   *  `focus`, `gobo`, `color_wheel`. Absent on a refused mode. */
  capabilities?: string[];
  /** Cells of a pixel mode (0 for a one-colour fixture). */
  cells?: number;
  /** The strobe channel's rate, when it carries one. */
  strobe_range?: { min_hz: number | null; max_hz: number };
  /** The channel map: [address offset, name], as the type would have it. */
  channels?: [number, string][];
  /** What the distiller skipped or guessed. */
  warnings?: string[];
  /** Why the mode cannot be imported; present only when it cannot. */
  refused?: string;
}

export interface GdtfInspection {
  fixture: string;
  manufacturer: string;
  /** The archive's fixture name as a safe type name. */
  suggested_name?: string;
  /** Where an import writes the `.fixture` file. */
  fixture_types_dir?: string;
  modes: GdtfMode[];
}

export interface GdtfImportReport {
  type_name: string;
  mode: string;
  archive: string;
  replaced_archive: boolean;
  fixture_file: string;
  channels: [number, string][];
  warnings: string[];
}

/** Parses an uploaded GDTF archive and returns its modes. Writes nothing. */
export async function inspectGdtf(file: File): Promise<GdtfInspection> {
  const res = await uploadFiles("/lighting/gdtf/inspect", [file]);
  if (!res.ok) throw await apiError(res, "Failed to inspect GDTF");
  return res.json();
}

/** Imports one mode of a GDTF archive: the archive lands in
 * lighting/library/, a referential .fixture definition is written, and the
 * expansion cache is warmed. Returns the import report. */
export async function importGdtf(
  file: File,
  mode: string,
  name?: string,
): Promise<GdtfImportReport> {
  const params = new URLSearchParams({ mode });
  if (name) params.set("name", name);
  const res = await uploadFiles(`/lighting/gdtf/import?${params}`, [file]);
  if (!res.ok) throw await apiError(res, "Failed to import GDTF");
  return res.json();
}

// --- MVR import and export (lighting UI design section 11) ---

export type Vec3Mm = [number, number, number];

export interface MvrBoundsMm {
  min: Vec3Mm;
  max: Vec3Mm;
}

/** The file seen from above, in the file's own millimeters. */
export interface MvrSceneView {
  fixtures: {
    name: string;
    layer: string;
    position_mm: Vec3Mm | null;
  }[];
  focus_points: { name: string; position_mm: Vec3Mm | null }[];
  scenery: {
    name: string;
    kind: string;
    deck: boolean;
    bounds_mm: MvrBoundsMm | null;
  }[];
  /** The stage floor's bounds, when the scenery carries one. */
  deck_mm: MvrBoundsMm | null;
}

export interface MvrPlannedType {
  name: string;
  archive: string;
  /** The type's default mode, as the GDTF spells it: the mode most of the
   *  file's fixtures use. Null for an existing type with no default, whose
   *  fixtures all name their modes. */
  mode: string | null;
  /** Every mode the file patches the type's fixtures in, sorted. */
  modes: string[];
  existing: boolean;
  fixture_file: string;
}

/** A hand-edited field a merge would overwrite with the MVR's value. */
export interface MvrHandEdit {
  /** "position", "rotation", "patch", "mode" or "type". */
  field: string;
  mine: string;
  mvr: string;
}

/** The hand edits a merge keeps instead of overwriting. */
export interface MvrKeep {
  fixtures: Record<string, string[]>;
  focus_points: string[];
}

export interface MvrPlannedFixture {
  name: string;
  layer: string;
  fixture_type: string | null;
  /** The mode the venue line names: null when the fixture is in its type's
   *  default mode (or is a TODO). */
  mode?: string | null;
  patch: [number, number] | null;
  position: Vec3 | null;
  rotation: Vec3 | null;
  tags: string[];
  todo: string | null;
  change: string | null;
  /** On a merge: hand-edited fields the import would overwrite. */
  overwrites?: MvrHandEdit[];
  /** Hand-edited fields the request keeps. */
  kept_edits?: string[];
}

/** What an import would do (and, after a write, did): the MCP plan. */
export interface MvrPlan {
  venue_name: string;
  venue_file: string;
  archive: string;
  merge: boolean;
  origin: Vec3;
  fixture_types: MvrPlannedType[];
  fixtures: MvrPlannedFixture[];
  removed_fixtures: { name: string; tags: string[] }[];
  kept_fixtures: string[];
  focus_points: {
    name: string;
    point: Vec3;
    change: string | null;
    overwrites?: MvrHandEdit[];
    kept_edit?: boolean;
  }[];
  removed_focus_points: string[];
  kept_focus_points: string[];
  scenery_objects: number;
  scenery_meshes_undrawn: number;
  warnings: string[];
}

export interface MvrImportReport extends MvrPlan {
  written: string[];
  distillation_warnings: Record<string, string[]>;
}

export interface MvrInspection {
  file_name: string;
  report: MvrPlan;
  scene: MvrSceneView;
}

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

/** A venue that will not load: the fixture that fails it, and why. While it
 *  stands, no fixture of the venue lights. */
export interface MvrVenueError {
  venue: string;
  fixture: string;
  reason: string;
}

/** The import's plan (`write` false) or its report (`write` true). */
export async function importMvr(
  file: File,
  options: { name: string; origin: Vec3Mm; write: boolean; keep?: MvrKeep },
  dirs: MvrDirs = {},
): Promise<{
  write: boolean;
  plan?: MvrPlan;
  report?: MvrImportReport;
  reloaded?: boolean;
  /** After a write: the current venue no longer loads, and why. */
  venue_error?: MvrVenueError | null;
}> {
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
  return res.json();
}

export interface MvrExportSummary {
  venue: string;
  /** The name a download would carry. */
  file_name: string;
  fixtures: number;
  positioned_fixtures: number;
  focus_points: number;
  embedded_gdtfs: string[];
  generated_gdtfs: Record<string, string>;
  /** Positioned fixed fixtures, each with the focus point it links to. */
  fixed_fixtures: { name: string; focus: string | null }[];
  warnings: string[];
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
  const body = await res.json();
  return { status: "kept", path: String(body.kept) };
}

export interface AimPointsResult {
  created: { name: string; fixture: string; point: Vec3 }[];
  reloaded: boolean;
  /** The venue file's version after the change. */
  version?: string;
  /** The current venue no longer loads after the change. */
  venue_error?: VenueError | null;
}

/** Adds a focus point for each unlinked fixed fixture, where its rest beam
 *  meets the deck. */
export async function addAimPoints(
  venue: string,
  dirs: MvrDirs = {},
  version?: string,
): Promise<AimPointsResult> {
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
  return res.json();
}

export async function fetchFixtureType(
  name: string,
  dir?: string,
): Promise<FixtureTypeEntry & { dsl: string }> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to fetch fixture type");
  return res.json();
}

/** What a referential fixture type's GDTF archive holds, for the type's
 *  details view. Paths are in the asset store (`/api/lighting/assets/`). */
export interface FixtureTypeGdtf {
  /** The archive as the `.fixture` names it, project-relative. */
  archive: string;
  /** The type's default mode as the `.fixture` writes it; null when it has
   *  none. */
  mode: string | null;
  /** The archive mode that pin resolves to, by its own name; null when it
   *  resolves to none. */
  matched_mode: string | null;
  /** The rig model for the 3D view; null when none could be made. */
  rig: string | null;
  thumbnail: string | null;
  /** The pinned mode's first beam; each figure null when the archive does
   *  not state it. */
  beam: {
    type: string | null;
    beam_angle: number | null;
    field_angle: number | null;
    luminous_flux: number | null;
    color_temperature: number | null;
    /** Watts. */
    power: number | null;
  } | null;
  /** The archive's own description of the fixture: a stranger's text. */
  about: string | null;
  /** The venues with fixtures of this type, each fixture in patch order
   *  with the mode it is driven in (its own, else the default), spelled as
   *  the archive spells it; null when that names no mode of the archive. */
  venues: { name: string; fixtures: { name: string; mode: string | null }[] }[];
  inspection: GdtfInspection;
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

/** Movement limits in degrees per second; null where none is set. */
export interface MovementLimits {
  max_pan_speed: number | null;
  max_tilt_speed: number | null;
}

/** What a GDTF type's file holds that is the user's: the fixture page's
 *  settings form. */
export interface FixtureSettingsData {
  name: string;
  default_mode: string | null;
  movement: MovementLimits;
  /** The file they are saved in. */
  file: string;
  /** The file's version, sent back as `If-Match`. */
  version: string;
}

/** What a settings save does (or, planned, would do). */
export interface FixtureSettingsResult {
  write: boolean;
  file: string;
  version: string;
  /** The file's text after the save. */
  dsl: string;
  /** A rename: every venue line that names the type, by venue. */
  rename: {
    from: string;
    to: string;
    lines: number;
    venues: { venue: string; file: string; lines: number }[];
  } | null;
  /** A new default: the venue fixtures that take it (they name no mode). */
  default_change: {
    from: string | null;
    to: string | null;
    /** The new default's addresses; null when unknown or none. */
    footprint: number | null;
    count: number;
    venues: { venue: string; fixtures: string[] }[];
  } | null;
  /** Overlaps the new default would cause, not there before. */
  overlaps: {
    venue: string;
    a: string;
    b: string;
    a_gang: string[];
    b_gang: string[];
    universe: number;
    from: number;
    to: number;
    message: string;
  }[];
  overruns: { venue: string; fixture: string; message: string }[];
  /** The venue files read, by name, with their versions. */
  venue_versions: Record<string, string>;
  /** Inline fixtures in the player config that name the type (a rename
   *  leaves them for the user to change). */
  config_references: string[];
  reloaded: boolean;
  venue_error: VenueError | null;
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
    default_mode: string | null;
    movement: MovementLimits;
    write: boolean;
    venue_versions?: Record<string, string>;
  },
  dirs: { dir?: string; venuesDir?: string },
  version?: string,
): Promise<FixtureSettingsResult> {
  const res = await post(
    `/lighting/fixture-types/${encodeURIComponent(name)}/settings${dirParams(dirs.dir, dirs.venuesDir)}`,
    JSON.stringify(body),
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok)
    throw await versionedError(res, "Failed to save the fixture's settings");
  return res.json();
}

export async function saveFixtureType(
  name: string,
  data: {
    channels: Record<string, number>;
    max_strobe_frequency?: number | null;
    min_strobe_frequency?: number | null;
    strobe_dmx_offset?: number | null;
  },
  dir?: string,
): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await put(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
}

/** Saves a fixture type as raw DSL. `ext` decides the form a *new* type is
 *  born in; an existing file keeps its own extension. */
export async function saveFixtureTypeText(
  name: string,
  dsl: string,
  ext: "light" | "fixture",
  dir?: string,
): Promise<void> {
  const params = new URLSearchParams({ ext });
  if (dir) params.set("dir", dir);
  const res = await putText(
    `/lighting/fixture-types/${encodeURIComponent(name)}?${params}`,
    dsl,
  );
  if (!res.ok) throw await apiError(res, "Failed to save fixture type");
}

export async function deleteFixtureType(
  name: string,
  dir?: string,
): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/fixture-types/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete fixture type");
}

/// Group names a cue can target, with the fixtures each currently resolves to.
/// These are the logical groups from `dmx.lighting.groups`; venues no longer
/// define groups of their own.
export async function fetchLightingGroups(): Promise<
  { name: string; fixtures: string[] }[]
> {
  const res = await get("/lighting/groups");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting groups");
  const data = await res.json();
  return data.groups ?? [];
}

/** The facts behind the Lighting overview's five checks. */
export async function fetchLightingReadiness(): Promise<Readiness> {
  const res = await get("/lighting/readiness");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting readiness");
  const readiness = parseReadiness(await readJson(res, "the readiness checks"));
  if (!readiness) throw new BadAnswerError("the readiness checks");
  return readiness;
}

/** The facts and suggestions behind the Fit shows page. */
export async function fetchLightingFit(): Promise<Fit> {
  const res = await get("/lighting/fit");
  if (!res.ok) throw await apiError(res, "Failed to fetch lighting fit");
  const fit = parseFit(await readJson(res, "the fit"));
  if (!fit) throw new BadAnswerError("the fit");
  return fit;
}

/** A `.light` file in a lighting directory that could not be parsed. */
export interface LightingFileError {
  file: string;
  error: string;
}

/** Venues that parsed, plus the files that did not.
 *
 *  A directory is a set of independent files, so one bad file no longer empties
 *  the list — but it must still be reported, or the only signal is a venue
 *  quietly missing. */
export async function fetchVenues(dir?: string): Promise<{
  venues: Record<string, VenueData>;
  errors: LightingFileError[];
  /** Each venue file's version, to save against. */
  versions: Record<string, string>;
}> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await get(`/lighting/venues${params}`);
  if (!res.ok) throw await apiError(res, "Failed to fetch venues");
  const data = await res.json();
  return {
    venues: data.venues ?? {},
    errors: data.errors ?? [],
    versions: data.versions ?? {},
  };
}

export async function fetchVenue(
  name: string,
  dir?: string,
): Promise<{ venue: VenueData; dsl: string; version?: string }> {
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
    fixtures: {
      name: string;
      fixture_type: string;
      universe: number;
      start_channel: number;
      tags: string[];
      position?: Vec3 | null;
      rotation?: Vec3 | null;
      mode?: string | null;
    }[];
    focus_points?: Record<string, Vec3>;
    source?: VenueSource | null;
  },
  dir?: string,
  version?: string,
): Promise<SavedVenue> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await put(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
    JSON.stringify(data),
    version ? { "If-Match": version } : undefined,
  );
  if (!res.ok) throw await versionedError(res, "Failed to save venue");
  const body = await res.json().catch(() => ({}));
  return {
    version: body?.version ?? null,
    venueError: body?.venue_error ?? null,
  };
}

/** Why the current venue does not load after a save: the first fixture
 *  that cannot be driven. A venue that does not load lights nothing. */
export interface VenueError {
  venue: string;
  fixture: string;
  reason: string;
}

/** What a venue save answers: the file's new version, and the venue's
 *  load failure when the save left the current venue failing. */
export interface SavedVenue {
  version: string | null;
  venueError: VenueError | null;
}

/** One fixture's addresses, as `GET /api/lighting/venues/{name}/patch`
 *  reports them; `footprint` is null when its type or mode does not load. */
export interface PatchSpan {
  fixture: string;
  universe: number;
  address: number;
  footprint: number | null;
  type: string;
  /** The mode the fixture line names; null is its type's default. */
  mode: string | null;
}

export interface PatchOverlapInfo {
  a: string;
  b: string;
  a_gang: string[];
  b_gang: string[];
  universe: number;
  from: number;
  to: number;
}

export interface VenuePatch {
  spans: PatchSpan[];
  overlaps: PatchOverlapInfo[];
  overruns: {
    fixture: string;
    universe: number;
    address: number;
    footprint: number;
    last: number;
  }[];
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

export async function deleteVenue(name: string, dir?: string): Promise<void> {
  const params = dir ? `?dir=${encodeURIComponent(dir)}` : "";
  const res = await del(
    `/lighting/venues/${encodeURIComponent(name)}${params}`,
  );
  if (!res.ok) throw await apiError(res, "Failed to delete venue");
}

// ---- Playlist CRUD ----

export interface PlaylistInfo {
  name: string;
  song_count: number;
  is_active: boolean;
}

export interface PlaylistData {
  name: string;
  songs: string[];
  available_songs: string[];
}

export async function fetchPlaylists(): Promise<PlaylistInfo[]> {
  const res = await get("/playlists");
  if (!res.ok) throw new Error(`Failed to fetch playlists: ${res.status}`);
  return res.json();
}

export async function fetchPlaylist(name: string): Promise<PlaylistData> {
  const res = await get(`/playlists/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`Failed to fetch playlist: ${res.status}`);
  return res.json();
}

export async function savePlaylist(
  name: string,
  songs: string[],
): Promise<void> {
  const res = await put(
    `/playlists/${encodeURIComponent(name)}`,
    JSON.stringify({ songs }),
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

export interface LightFileInfo {
  path: string;
  name: string;
}

export async function fetchLightingFiles(): Promise<LightFileInfo[]> {
  const res = await get("/lighting");
  if (!res.ok) throw new Error(`Failed to fetch lighting files: ${res.status}`);
  const data = await res.json();
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
 *  `@bar`/`beat` timing cannot be parsed without that song's tempo map. */
export async function validateLighting(
  content: string,
  song?: string,
): Promise<{ valid: boolean; errors?: string[] }> {
  const path = song
    ? `/lighting/validate?song=${encodeURIComponent(song)}`
    : "/lighting/validate";
  const res = await postText(path, content);
  return res.json();
}
