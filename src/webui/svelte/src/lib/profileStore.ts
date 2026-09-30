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

/* eslint-disable @typescript-eslint/no-explicit-any */
/**
 * Hardware profile loading and saving, shared by Config (the profile editor)
 * and Lighting (the Groups page).
 *
 * A profile lives in one of two places: a file under `profiles_dir` when the
 * config sets one, or an inline entry in `mtrack.yaml`'s `profiles` list. The
 * two use different APIs; this module hides that so an editor asks to read or
 * write "a profile" and gets the same behaviour either way.
 */

import YAML from "yaml";
import {
  addProfile,
  fetchConfigStore,
  fetchProfileFile,
  fetchProfileFiles,
  saveProfileFile,
  updateProfile,
  type ConfigSnapshot,
  type ProfileFileInfo,
} from "./api/config";
import { get } from "./api/rest";

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
  inline: any[];
  /** Profile files (empty in inline mode). */
  files: ProfileFileInfo[];
  entries: ProfileEntry[];
}

/** Pulls the profile-related parts out of a config's YAML. */
export function parseProfileYaml(yaml: string): {
  profilesDir: string | null;
  inline: any[];
} {
  const parsed = YAML.parse(yaml);
  const profilesDir: string | null = parsed?.profiles_dir || null;
  return { profilesDir, inline: profilesDir ? [] : parsed?.profiles || [] };
}

export function inlineProfileName(profile: any, index: number): string {
  return profile?.hostname || `Profile #${index}`;
}

export function fileProfileName(filename: string): string {
  return filename.replace(/\.\w+$/, "");
}

function buildEntries(
  profilesDir: string | null,
  inline: any[],
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

/** Reads one profile for editing. Inline profiles come from the already
 *  loaded list (`inline`); file profiles are fetched. The result is a copy
 *  the caller may edit freely. */
export async function readProfile(
  ref: ProfileRef,
  inline: any[],
): Promise<any> {
  if (ref.kind === "file") {
    return (await fetchProfileFile(ref.filename)).profile;
  }
  return JSON.parse(JSON.stringify(inline[ref.index]));
}

/** Writes one profile. Returns the new config snapshot for an inline profile
 *  (whose checksum the next save needs), or null for a file, which has none. */
export async function writeProfile(
  ref: ProfileRef,
  profile: object,
  checksum: string,
  isNew = false,
): Promise<ConfigSnapshot | null> {
  if (ref.kind === "file") {
    await saveProfileFile(ref.filename, profile);
    return null;
  }
  return isNew
    ? addProfile(profile, checksum)
    : updateProfile(ref.index, profile, checksum);
}

/** The profile the player is running: the one whose `hostname` matches what
 *  `/api/status` reports as `hardware.profile`, else the first. */
export function pickRunningProfile(
  entries: ProfileEntry[],
  running: string | null | undefined,
): ProfileEntry | null {
  if (entries.length === 0) return null;
  return (
    (running ? entries.find((e) => e.hostname === running) : undefined) ??
    entries[0]
  );
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
  lighting: any;
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
 *  makes the engine reload. Returns the profile's name. A universe the
 *  profile already lists is left alone. Throws when there is no profile to
 *  write to. */
export async function addUniverseToRunningProfile(
  universe: number,
): Promise<string> {
  const [set, running] = await Promise.all([
    loadProfileSet(),
    fetchRunningProfile(),
  ]);
  const entry = pickRunningProfile(set.entries, running);
  if (!entry) throw new Error("No hardware profile to add the universe to");
  const profile = await readProfile(entry.ref, set.inline);
  const dmx = (profile.dmx ??= {});
  const universes: any[] = (dmx.universes ??= []);
  if (!universes.some((u) => u?.universe === universe)) {
    universes.push({ universe, name: `u${universe}` });
    await writeProfile(entry.ref, profile, set.checksum);
  }
  return entry.name;
}
