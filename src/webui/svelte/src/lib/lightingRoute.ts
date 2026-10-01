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

/** The Lighting area's sub-pages. */
export type LightingSub =
  | "overview"
  | "fixtures"
  | "venues"
  | "groups"
  | "fit"
  | "import";

export interface LightingRoute {
  sub: LightingSub;
  /** The `?profile=` value (Groups only). */
  profile: string | null;
  /** The `?group=` value (Fit shows only): the group to select. */
  group: string | null;
  /** The thing a page has open, from the path after the sub-page:
   *  `#/lighting/fixtures/<name>` (a fixture's page),
   *  `#/lighting/venues/<name>` (the venue selected for the stage card). */
  item: string | null;
  /** `?edit` on a venue: its form is open. `?as=text` on a fixture type:
   *  its file is open as text. */
  edit: boolean;
  as: string | null;
  /** `?new=<kind>`: a form for a new thing is open (`light`, `fixture`, or
   *  `venue`). */
  creating: string | null;
  /** `?view=3d` on Venues: the stage card shows the venue in 3D. */
  view: "plot" | "3d";
  /** `?mode=preview` (with `song` and `t`): the 3D view previews a song's
   *  show at a moment rather than drawing the live state. */
  mode: "live" | "preview";
  song: string | null;
  /** `?t=`, seconds; null when absent or not a time. */
  time: number | null;
}

const SUBS: readonly LightingSub[] = [
  "fixtures",
  "venues",
  "groups",
  "fit",
  "import",
];

/** Parses `#/lighting[/sub[/item]][?profile=name][&group=name][&edit][&as=text][&new=kind]
 *  [&view=3d][&mode=preview][&song=name][&t=seconds]`.
 *  An unknown sub-page is the overview. What a page has open is part of the
 *  address, so the section's links, the browser's Back and a reload all
 *  land where they say. */
export function lightingRoute(hash: string): LightingRoute {
  const rest = hash.replace(/^#\/?lighting\/?/, "");
  const [path, query = ""] = rest.split("?", 2);
  const [first, ...more] = path.split("/");
  const sub = (SUBS as readonly string[]).includes(first)
    ? (first as LightingSub)
    : "overview";
  const params = new URLSearchParams(query);
  let item: string | null = null;
  if (more.length > 0 && more.join("/")) {
    try {
      item = decodeURIComponent(more.join("/"));
    } catch {
      item = more.join("/");
    }
  }
  return {
    sub,
    profile: params.get("profile"),
    group: params.get("group"),
    item,
    edit: params.has("edit"),
    as: params.get("as"),
    creating: params.get("new"),
    view: params.get("view") === "3d" ? "3d" : "plot",
    mode: params.get("mode") === "preview" ? "preview" : "live",
    song: params.get("song"),
    time: timeParam(params.get("t")),
  };
}

function timeParam(raw: string | null): number | null {
  if (raw === null || raw === "") return null;
  const t = Number(raw);
  return Number.isFinite(t) && t >= 0 ? t : null;
}

/** The address of a page's open thing (`null` for the page itself). */
export function lightingHref(
  sub: "fixtures" | "venues",
  item: string | null = null,
  extra: Record<string, string> = {},
): string {
  const base = item
    ? `#/lighting/${sub}/${encodeURIComponent(item)}`
    : `#/lighting/${sub}`;
  const query = new URLSearchParams(extra).toString();
  return query ? `${base}?${query.replace(/=(&|$)/g, "$1")}` : base;
}

/** The same address with some query parameters set (a string) or removed
 *  (null); the rest of the address, and every other parameter, is kept.
 *  Empty values are written bare (`?edit`), as `lightingHref` writes them. */
export function withParams(
  hash: string,
  changes: Record<string, string | null>,
): string {
  const [path, query = ""] = hash.split("?", 2);
  const params = new URLSearchParams(query);
  for (const [key, value] of Object.entries(changes)) {
    if (value === null) params.delete(key);
    else params.set(key, value);
  }
  const out = params.toString().replace(/=(&|$)/g, "$1");
  return out ? `${path}?${out}` : path;
}

/** The venue card's 3D view of a venue (the current one when `venue` is
 *  null), optionally previewing a song at a moment. */
export function venue3dHref(
  venue: string | null,
  preview: { song: string; time?: number } | null = null,
): string {
  const extra: Record<string, string> = { view: "3d" };
  if (preview) {
    extra.mode = "preview";
    extra.song = preview.song;
    if (preview.time !== undefined)
      extra.t = String(Math.round(preview.time * 1000) / 1000);
  }
  return lightingHref("venues", venue, extra);
}

/** Stage 3D no longer has a page of its own: an old address for it
 *  (`#/stage…`, `#/lighting/stage…`) lands on the Venues page in 3D, on
 *  the current venue when one is known, keeping a preview's song and time.
 *  Any other hash is returned as it is. */
export function redirectRetiredHash(
  hash: string,
  currentVenue: string | null,
): string {
  const m = /^#\/(?:lighting\/)?stage(?=$|[/?])[^?]*(?:\?(.*))?$/.exec(hash);
  if (!m) return hash;
  const params = new URLSearchParams(m[1] ?? "");
  const extra: Record<string, string> = { view: "3d" };
  for (const key of ["mode", "song", "t"]) {
    const value = params.get(key);
    if (value !== null) extra[key] = value;
  }
  return lightingHref("venues", currentVenue, extra);
}
