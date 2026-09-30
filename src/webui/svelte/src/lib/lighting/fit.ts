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
 * The Fit shows page's data: the facts and suggestions `GET /api/lighting/fit`
 * reports (design/lighting_ui_design.md, section 9), and the small pure rules
 * the page applies to them.
 */

import type { Vec3 } from "../api/config";
import { arr, hasAny, isObj, obj, objOrNull, str, type Obj } from "./answer";

export type Want = "move" | "color" | "strobe" | "cells" | "dimmer";
export type Height = "deck" | "low" | "truss";
export type Depth = "downstage" | "mid" | "upstage";

export interface FitFixture {
  name: string;
  type: string;
  tags: string[];
  position: Vec3 | null;
  capabilities: string[];
}

/** A set of like fixtures hanging in one place. */
export interface Cluster {
  fixtures: string[];
  type: string;
  /** The place in English words, for logs; the page words it from
   *  `height` and `depth`. Null when the venue does not place them. */
  where: string | null;
  height: Height | null;
  depth: Depth | null;
}

export interface Suggestion {
  fixtures: string[];
  tags: string[];
  reason: {
    count: number;
    type: string;
    where: string | null;
    height: Height | null;
    depth: Depth | null;
    can: Want[];
  };
}

export interface FitGroup {
  name: string;
  /** Whether the running profile declares the group. */
  defined: boolean;
  needs: { all_of: string[]; any_of: string[]; prefer: string[] };
  /** The fixtures it finds in the current venue now. */
  fixtures: string[];
  songs: string[];
  wants: Want[];
  suggestion: Suggestion | null;
  others: Cluster[];
  /** With no suggestion: the wants no fixture meets. */
  unmet: Want[];
  /** Every want is met by some fixture, but none meets them all. */
  unmet_together: boolean;
}

/** What `GET /api/lighting/fit` returns. */
export interface Fit {
  venue: {
    name: string;
    fixtures: FitFixture[];
    focus_points: string[];
  } | null;
  groups: FitGroup[];
  focus_points_wanted: { name: string; songs: string[] }[];
  output: {
    unconfigured: number[];
    unpatched: number[];
    /** Whether olad's web server answered; null when nothing was asked. */
    reachable: boolean | null;
    ola_http_port: number | null;
  };
}

/** Guards `GET /api/lighting/fit`'s answer the way `parseReadiness` guards
 *  the readiness one: arrays default to `[]`, objects to null. Null when the
 *  body is not an object with any of the expected fields. */
export function parseFit(body: unknown): Fit | null {
  if (!hasAny(body, ["venue", "groups", "focus_points_wanted", "output"])) {
    return null;
  }
  const venue = objOrNull(body.venue);
  const output = obj(body.output);
  const cluster = (c: Obj): Cluster => ({
    fixtures: arr<string>(c.fixtures),
    type: str(c.type),
    where: typeof c.where === "string" ? c.where : null,
    height: (c.height as Height | null) ?? null,
    depth: (c.depth as Depth | null) ?? null,
  });
  return {
    venue: venue
      ? {
          name: str(venue.name),
          fixtures: arr<Obj>(venue.fixtures)
            .filter(isObj)
            .map((f) => ({
              name: str(f.name),
              type: str(f.type),
              tags: arr<string>(f.tags),
              capabilities: arr<string>(f.capabilities),
              position: (f.position as Vec3 | null | undefined) ?? null,
            })),
          focus_points: arr<string>(venue.focus_points),
        }
      : null,
    groups: arr<Obj>(body.groups)
      .filter(isObj)
      .map((g) => {
        const needs = obj(g.needs);
        const suggestion = objOrNull(g.suggestion);
        const reason = obj(suggestion?.reason);
        return {
          name: str(g.name),
          defined: g.defined === true,
          needs: {
            all_of: arr<string>(needs.all_of),
            any_of: arr<string>(needs.any_of),
            prefer: arr<string>(needs.prefer),
          },
          fixtures: arr<string>(g.fixtures),
          songs: arr<string>(g.songs),
          wants: arr<Want>(g.wants),
          suggestion: suggestion
            ? {
                fixtures: arr<string>(suggestion.fixtures),
                tags: arr<string>(suggestion.tags),
                reason: {
                  count:
                    typeof reason.count === "number"
                      ? reason.count
                      : arr(suggestion.fixtures).length,
                  type: str(reason.type),
                  where: typeof reason.where === "string" ? reason.where : null,
                  height: (reason.height as Height | null) ?? null,
                  depth: (reason.depth as Depth | null) ?? null,
                  can: arr<Want>(reason.can),
                },
              }
            : null,
          others: arr<Obj>(g.others).filter(isObj).map(cluster),
          unmet: arr<Want>(g.unmet),
          unmet_together: g.unmet_together === true,
        };
      }),
    focus_points_wanted: arr<Obj>(body.focus_points_wanted)
      .filter(isObj)
      .map((f) => ({ name: str(f.name), songs: arr<string>(f.songs) })),
    output: {
      unconfigured: arr<number>(output.unconfigured),
      unpatched: arr<number>(output.unpatched),
      reachable:
        typeof output.reachable === "boolean" ? output.reachable : null,
      ola_http_port:
        typeof output.ola_http_port === "number" ? output.ola_http_port : null,
    },
  };
}

/** Groups in the order the page lists them: those finding no fixtures first,
 *  each half by name. */
export function orderGroups(groups: FitGroup[]): FitGroup[] {
  return [...groups].sort((a, b) => {
    const ea = a.fixtures.length === 0 ? 0 : 1;
    const eb = b.fixtures.length === 0 ? 0 : 1;
    return ea - eb || a.name.localeCompare(b.name);
  });
}

/** The tags that would put a fixture in a group: every `AllOf` tag plus the
 *  first `AnyOf` one, as the server suggests them. */
export function tagsFor(group: FitGroup): string[] {
  const tags = [...group.needs.all_of];
  if (group.needs.any_of.length > 0) tags.push(group.needs.any_of[0]);
  return [...new Set(tags)];
}

/** Whether two selections name the same fixtures. */
export function sameSet(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((n) => b.includes(n));
}

interface VenueFileFixture {
  name: string;
  tags: string[];
  [key: string]: unknown;
}

/** The venue's fixtures with `tags` added to each of `names`. Everything else
 *  about every fixture (position, rotation, patch) is carried through as it
 *  is, and tags a fixture already has stay where they are. */
export function withTags<F extends VenueFileFixture>(
  fixtures: F[],
  names: string[],
  tags: string[],
): F[] {
  return fixtures.map((f) => {
    if (!names.includes(f.name)) return f;
    const missing = tags.filter((t) => !f.tags.includes(t));
    return missing.length === 0 ? f : { ...f, tags: [...f.tags, ...missing] };
  });
}

/** The `ola_patch` command for one universe. The device and port belong to
 *  the user's rig, so they are left as placeholders. */
export function olaPatchLine(universe: number): string {
  return `ola_patch -d <device> -p <port> -u ${universe}`;
}
