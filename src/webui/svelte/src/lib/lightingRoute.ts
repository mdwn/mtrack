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
  | "stage";

export interface LightingRoute {
  sub: LightingSub;
  /** The `?profile=` value (Groups only). */
  profile: string | null;
}

const SUBS: readonly LightingSub[] = ["fixtures", "venues", "groups", "stage"];

/** Parses `#/lighting[/sub][?profile=name]`. An unknown sub-page is the
 *  overview. */
export function lightingRoute(hash: string): LightingRoute {
  const rest = hash.replace(/^#\/?lighting\/?/, "");
  const [path, query = ""] = rest.split("?", 2);
  const first = path.split("/")[0];
  const sub = (SUBS as readonly string[]).includes(first)
    ? (first as LightingSub)
    : "overview";
  return { sub, profile: new URLSearchParams(query).get("profile") };
}

/** The old Stage 3D address maps to its place in the Lighting area, keeping
 *  anything after `#/stage`. Any other hash is returned as it is. */
export function redirectLegacyHash(hash: string): string {
  const m = /^#\/stage(?=$|[/?])(.*)$/.exec(hash);
  return m ? `#/lighting/stage${m[1]}` : hash;
}
