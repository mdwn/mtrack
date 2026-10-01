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
  | "import"
  | "stage";

export interface LightingRoute {
  sub: LightingSub;
  /** The `?profile=` value (Groups only). */
  profile: string | null;
  /** The `?group=` value (Fit shows only): the group to select. */
  group: string | null;
  /** The thing a page has open, from the path after the sub-page:
   *  `#/lighting/fixtures/<name>` (a fixture's page),
   *  `#/lighting/venues/<name>` (the venue selected for the plot),
   *  `#/lighting/stage/<name>` (the venue Stage 3D shows). */
  item: string | null;
  /** `?edit` on a venue: its form is open. `?as=text` on a fixture type:
   *  its file is open as text. */
  edit: boolean;
  as: string | null;
  /** `?new=<kind>`: a form for a new thing is open (`light`, `fixture`, or
   *  `venue`). */
  creating: string | null;
}

const SUBS: readonly LightingSub[] = [
  "fixtures",
  "venues",
  "groups",
  "fit",
  "import",
  "stage",
];

/** Parses `#/lighting[/sub[/item]][?profile=name][&group=name][&edit][&as=text][&new=kind]`.
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
  };
}

/** The address of a page's open thing (`null` for the page itself). */
export function lightingHref(
  sub: "fixtures" | "venues" | "stage",
  item: string | null = null,
  extra: Record<string, string> = {},
): string {
  const base = item
    ? `#/lighting/${sub}/${encodeURIComponent(item)}`
    : `#/lighting/${sub}`;
  const query = new URLSearchParams(extra).toString();
  return query ? `${base}?${query.replace(/=(&|$)/g, "$1")}` : base;
}

/** The old Stage 3D address maps to its place in the Lighting area, keeping
 *  anything after `#/stage`. Any other hash is returned as it is. */
export function redirectLegacyHash(hash: string): string {
  const m = /^#\/stage(?=$|[/?])(.*)$/.exec(hash);
  return m ? `#/lighting/stage${m[1]}` : hash;
}
