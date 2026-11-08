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

/** The song detail page's tabs, as the address names them. */
export const SONG_TABS = [
  "tracks",
  "midi",
  "samples",
  "sections",
  "lighting",
  "notifications",
  "config",
] as const;

export type SongTab = (typeof SONG_TABS)[number];

export interface SongRoute {
  /** The song open, or null for the song list. */
  name: string | null;
  /** The tab the address names, if any. */
  tab: SongTab | undefined;
}

/** Parses `#/songs[/<name>[/<tab>]][?t=seconds]`. The name is one segment,
 *  encoded (`encodeURIComponent`), so the path is split before it is
 *  decoded: `#/songs/SongA%2Ftracks` is the song `SongA/tracks`, and
 *  `#/songs/SongA%2Ftracks/config` its Config tab. */
export function songRoute(hash: string): SongRoute {
  const path = hash.split("?", 1)[0];
  const prefix = "#/songs/";
  if (!path.startsWith(prefix)) return { name: null, tab: undefined };
  const [first, second] = path.slice(prefix.length).split("/");
  if (!first) return { name: null, tab: undefined };
  let name: string;
  try {
    name = decodeURIComponent(first);
  } catch {
    name = first;
  }
  const tab = (SONG_TABS as readonly string[]).includes(second)
    ? (second as SongTab)
    : undefined;
  return { name, tab };
}
