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

/**
 * Hardware profile loading and saving, shared by Config (the profile editor)
 * and Lighting (the Groups page).
 *
 * A profile lives in one of two places: a file under `profiles_dir` when the
 * config sets one, or an inline entry in `mtrack.yaml`'s `profiles` list. The
 * two use different APIs; this module hides that so an editor asks to read or
 * write "a profile" and gets the same behaviour either way.
 */

import type {
  ConfigReload,
  Lighting,
  StoreSnapshot,
} from "../gen/rules/mtrack_rules";
import YAML from "yaml";
import {
  addProfile,
  fetchConfigStore,
  fetchProfileFile,
  fetchProfileFiles,
  saveProfileFile,
  updateProfile,
  type ProfileDraft,
} from "./api/config";
import type { ProfileFileInfo } from "../gen/rules/mtrack_rules";
import { get } from "./api/rest";
import { reloadNotice, type ReloadNotice } from "./configReload";

/** Where one profile is stored. */
export type ProfileRef =
  | { kind: "file"; filename: string }
  | { kind: "inline"; index: number };

/** One profile in the listing. */
export interface ProfileEntry {
  /** The name used in URLs: a file's name without extension, or the
   *  hostname, or `Profile #<index>` for an inline profile with neither. */
  name: string;
  /** The profile's `hostname`, which decides which machine runs it. */
  hostname: string | null;
  ref: ProfileRef;
}

/** Everything read from the config in one go. */
export interface ProfileSet {
  yaml: string;
  checksum: string;
  profilesDir: string | null;
  /** Inline profiles (empty in file mode). */
  inline: ProfileDraft[];
  /** Profile files (empty in inline mode). */
  files: ProfileFileInfo[];
  entries: ProfileEntry[];
}

/** Pulls the profile-related parts out of a config's YAML. */
export function parseProfileYaml(yaml: string): {
  profilesDir: string | null;
  inline: ProfileDraft[];
} {
  const parsed = YAML.parse(yaml);
  const profilesDir: string | null = parsed?.profiles_dir || null;
  return { profilesDir, inline: profilesDir ? [] : parsed?.profiles || [] };
}

export function inlineProfileName(
  profile: ProfileDraft,
  index: number,
): string {
  return profile?.hostname || `Profile #${index}`;
}

export function fileProfileName(filename: string): string {
  return filename.replace(/\.\w+$/, "");
}

function buildEntries(
  profilesDir: string | null,
  inline: ProfileDraft[],
  files: ProfileFileInfo[],
): ProfileEntry[] {
  if (profilesDir) {
    return files.map((f) => ({
      name: fileProfileName(f.filename),
      hostname: f.hostname,
      ref: { kind: "file", filename: f.filename },
    }));
  }
  return inline.map((p, index) => ({
    name: inlineProfileName(p, index),
    hostname: p?.hostname ?? null,
    ref: { kind: "inline", index },
  }));
}

/** Lists the profile files in file mode. A failure is logged and reads as an
 *  empty list, as the profile list has always done. */
export async function loadProfileFileList(): Promise<ProfileFileInfo[]> {
  try {
    return await fetchProfileFiles();
  } catch (e) {
    console.error("Failed to load profile files:", e);
    return [];
  }
}

/** Reads the config and lists its profiles. */
export async function loadProfileSet(): Promise<ProfileSet> {
  const snapshot = await fetchConfigStore();
  const { profilesDir, inline } = parseProfileYaml(snapshot.yaml);
  const files = profilesDir ? await loadProfileFileList() : [];
  return {
    yaml: snapshot.yaml,
    checksum: snapshot.checksum,
    profilesDir,
    inline,
    files,
    entries: buildEntries(profilesDir, inline, files),
  };
}

/** Reads one profile for editing, with the version of the file it came from
 *  (file profiles only: an inline profile is guarded by the config checksum).
 *  Inline profiles come from the already loaded list (`inline`); file
 *  profiles are fetched. The profile is a copy the caller may edit freely. */
export async function readProfileVersioned(
  ref: ProfileRef,
  inline: ProfileDraft[],
): Promise<{ profile: ProfileDraft; version: string | null }> {
  if (ref.kind === "file") {
    const got = await fetchProfileFile(ref.filename);
    return { profile: got.profile, version: got.version ?? null };
  }
  return {
    profile: JSON.parse(JSON.stringify(inline[ref.index])),
    version: null,
  };
}

export async function readProfile(
  ref: ProfileRef,
  inline: ProfileDraft[],
): Promise<ProfileDraft> {
  return (await readProfileVersioned(ref, inline)).profile;
}

/** What a profile write hands back: the new config snapshot for an inline
 *  profile (whose checksum the next save needs), or the new file version for
 *  a file profile (which the next save must present), and what became of
 *  the hardware reload either way. */
export interface ProfileWrite {
  snapshot: StoreSnapshot | null;
  version: string | null;
  reload: Partial<ConfigReload>;
}

/** Writes one profile. Inline profiles are saved against the config
 *  `checksum`, file profiles against the file `version` they were read at;
 *  either one changed since is refused with a `ConflictError` (a file
 *  without a version, such as a new one, is written unconditionally). */
export async function writeProfile(
  ref: ProfileRef,
  profile: object,
  checksum: string,
  isNew = false,
  version: string | null = null,
): Promise<ProfileWrite> {
  if (ref.kind === "file") {
    const saved = await saveProfileFile(
      ref.filename,
      profile,
      version ?? undefined,
    );
    return { snapshot: null, version: saved.version, reload: saved.reload };
  }
  const snapshot = isNew
    ? await addProfile(profile, checksum)
    : await updateProfile(ref.index, profile, checksum);
  return { snapshot, version: null, reload: snapshot };
}

/** The profile the player is running: the one whose `hostname` matches what
 *  `/api/status` reports as `hardware.profile`. There is no fallback: when
 *  the status cannot be read or no profile names this host, the answer is
 *  null, and the caller has the user choose. Falling back to the first
 *  profile let one machine's editor write another machine's profile. */
export function pickRunningProfile(
  entries: ProfileEntry[],
  running: string | null | undefined,
): ProfileEntry | null {
  if (!running) return null;
  return entries.find((e) => e.hostname === running) ?? null;
}

/** The running profile's name as `/api/status` reports it, or null when the
 *  status cannot be read. */
export async function fetchRunningProfile(): Promise<string | null> {
  try {
    const res = await get("/status");
    if (!res.ok) return null;
    const data = await res.json();
    return data?.hardware?.profile ?? null;
  } catch {
    return null;
  }
}

/** The running profile's `dmx.lighting` block (empty when it has none), with
 *  the profile's name. Null when the config cannot be read or has no
 *  profiles. Lighting uses it for the directory overrides and the current
 *  venue. */
export async function loadRunningLighting(): Promise<{
  profileName: string;
  lighting: Lighting;
} | null> {
  try {
    const [set, running] = await Promise.all([
      loadProfileSet(),
      fetchRunningProfile(),
    ]);
    const entry = pickRunningProfile(set.entries, running);
    if (!entry) return null;
    const profile = await readProfile(entry.ref, set.inline);
    return { profileName: entry.name, lighting: profile?.dmx?.lighting ?? {} };
  } catch (e) {
    console.error("Failed to load the running profile:", e);
    return null;
  }
}

/** Adds an output for `universe` (`{universe, name: "u<N>"}`) to the running
 *  profile's `dmx.universes` and saves it the way Config does, which is what
 *  makes the engine reload. Returns the profile's name, and the save's
 *  reload notice (the reload waiting for playback to stop, or why it
 *  failed), if it calls for one. A universe the profile already lists is
 *  left alone. Throws when there is no profile to write to. */
export async function addUniverseToRunningProfile(
  universe: number,
): Promise<{ profile: string; notice: ReloadNotice | null }> {
  const [set, running] = await Promise.all([
    loadProfileSet(),
    fetchRunningProfile(),
  ]);
  const entry = pickRunningProfile(set.entries, running);
  if (!entry) {
    throw new Error(
      "No hardware profile matches this host; choose one on the Groups page",
    );
  }
  const { profile, version } = await readProfileVersioned(
    entry.ref,
    set.inline,
  );
  const universes = (profile.dmx ??= { universes: [] }).universes;
  let notice: ReloadNotice | null = null;
  if (!universes.some((u) => u?.universe === universe)) {
    universes.push({ universe, name: `u${universe}` });
    const written = await writeProfile(
      entry.ref,
      profile,
      set.checksum,
      false,
      version,
    );
    notice = reloadNotice(written.reload);
  }
  return { profile: entry.name, notice };
}
