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
 * The Lighting overview's five checks, worked out from the facts
 * `GET /api/lighting/readiness` reports.
 *
 * The backend states facts and never a verdict; every rule that turns a fact
 * into "ready", "needs attention", "blocked" or "unknown" lives here, so the
 * wording and the rules can be read, and translated, in one place
 * (design/lighting_ui_design.md, section 8.1).
 */

import {
  arr,
  hasAny,
  isObj,
  num,
  obj,
  objOrNull,
  str,
  type Obj,
} from "./answer";

/** What `GET /api/lighting/readiness` returns. */
export interface Readiness {
  dmx: boolean;
  venue: {
    name: string;
    fixtures: number;
    placed: number;
    focus_points: string[];
  } | null;
  fixture_types: {
    in_use: string[];
    unresolved: { fixture: string; type: string; reason: string }[];
  };
  groups: { name: string; fixtures: number; songs: string[] }[];
  shows: {
    song: string;
    files: string[];
    error?: string;
    warnings: { kind: string; message: string }[];
  }[];
  output: {
    universes: number[];
    unconfigured: number[];
    olad: { reachable: boolean; unpatched: number[] } | null;
  };
}

/** Guards `GET /api/lighting/readiness`'s answer: every array defaults to
 *  `[]`, objects to null, `dmx` to false. Null when the body is not an object
 *  with any of the expected fields (the caller treats that as an error). */
export function parseReadiness(body: unknown): Readiness | null {
  if (
    !hasAny(body, [
      "dmx",
      "venue",
      "fixture_types",
      "groups",
      "shows",
      "output",
    ])
  ) {
    return null;
  }
  const venue = objOrNull(body.venue);
  const types = obj(body.fixture_types);
  const output = obj(body.output);
  const olad = objOrNull(output.olad);
  return {
    dmx: body.dmx === true,
    venue: venue
      ? {
          name: str(venue.name),
          fixtures: num(venue.fixtures),
          placed: num(venue.placed),
          focus_points: arr<string>(venue.focus_points),
        }
      : null,
    fixture_types: {
      in_use: arr<string>(types.in_use),
      unresolved: arr<Readiness["fixture_types"]["unresolved"][number]>(
        types.unresolved,
      ).filter(isObj),
    },
    groups: arr<Obj>(body.groups)
      .filter(isObj)
      .map((g) => ({
        name: str(g.name),
        fixtures: num(g.fixtures),
        songs: arr<string>(g.songs),
      })),
    shows: arr<Obj>(body.shows)
      .filter(isObj)
      .map((show) => ({
        song: str(show.song),
        files: arr<string>(show.files),
        ...(typeof show.error === "string" ? { error: show.error } : {}),
        warnings: arr<Readiness["shows"][number]["warnings"][number]>(
          show.warnings,
        ).filter(isObj),
      })),
    output: {
      universes: arr<number>(output.universes),
      unconfigured: arr<number>(output.unconfigured),
      olad: olad
        ? {
            reachable: olad.reachable === true,
            unpatched: arr<number>(olad.unpatched),
          }
        : null,
    },
  };
}

export type CheckKey = "fixtures" | "venue" | "groups" | "shows" | "output";
export type CheckState = "ready" | "attention" | "blocked" | "unknown";

/** A translatable message: an i18n key and its parameters. */
export interface Msg {
  key: string;
  params?: Record<string, string | number>;
}

export interface Finding {
  /** How much this finding matters: a `note` is listed but changes nothing. */
  severity: "blocked" | "attention" | "unknown" | "note";
  msg: Msg;
  /** Where it is fixed. */
  href: string;
  /** What the link says (an i18n key). */
  linkKey: string;
  /** A song, when the finding is listed under one. */
  song?: string;
}

export interface Check {
  key: CheckKey;
  state: CheckState;
  /** The card's one-line summary, as parts shown side by side: a check that
   *  mixes severities says how many of each. */
  summary: Msg[];
  findings: Finding[];
}

/** Lint kinds that stop a cue doing what it says: the Shows check needs
 *  attention for these. */
const SHOW_BREAKING_KINDS = new Set(["capability-gap", "unbound-focus-point"]);

/** Lint kinds the Groups check already reports, so listing them under the
 *  song as well would say everything twice. (Venue-level coverage is not in a
 *  song's warnings at all; the Output section carries it.) */
const COVERED_ELSEWHERE_KINDS = new Set(["empty-group"]);

function count(findings: Finding[], severity: Finding["severity"]): number {
  return findings.filter((f) => f.severity === severity).length;
}

function stateOf(findings: Finding[]): CheckState {
  if (count(findings, "blocked") > 0) return "blocked";
  if (count(findings, "attention") > 0) return "attention";
  if (count(findings, "unknown") > 0) return "unknown";
  return "ready";
}

/** Builds a check from its findings. `blockedKey` words the blocking count
 *  (a plural message taking `count`). */
function finish(
  key: CheckKey,
  findings: Finding[],
  ready: Msg,
  blockedKey = "lighting.hub.summary.blocked",
): Check {
  const blocked = count(findings, "blocked");
  const attention = count(findings, "attention");
  const summary: Msg[] = [];
  if (blocked > 0)
    summary.push({ key: blockedKey, params: { count: blocked } });
  if (attention > 0) {
    summary.push({
      key: "lighting.hub.summary.attention",
      params: { count: attention },
    });
  }
  if (summary.length === 0) {
    summary.push(
      count(findings, "unknown") > 0
        ? { key: "lighting.hub.summary.unknown" }
        : ready,
    );
  }
  return { key, state: stateOf(findings), findings, summary };
}

/** A check that cannot be evaluated, and why. */
function cannot(key: CheckKey, why: string): Check {
  return {
    key,
    state: "unknown",
    findings: [],
    summary: [{ key: why }],
  };
}

export const CHECK_ORDER: CheckKey[] = [
  "fixtures",
  "venue",
  "groups",
  "shows",
  "output",
];

/** The Fit shows page, optionally with a group selected. */
export function fitHref(group?: string): string {
  return group
    ? `#/lighting/fit?group=${encodeURIComponent(group)}`
    : "#/lighting/fit";
}

/** Where a song's lighting is edited. */
export function songLightingHref(song: string): string {
  return `#/songs/${encodeURIComponent(song)}/lighting`;
}

export interface EvaluateOptions {
  /** The running profile's name, for the link into its DMX settings. */
  profileName: string | null;
}

/** The Shows check. It needs no venue or output: a show that does not load is
 *  broken on a laptop with no DMX too, which is where shows are written. */
function showsCheck(r: Readiness): Check {
  const findings: Finding[] = [];
  for (const show of r.shows) {
    const href = songLightingHref(show.song);
    if (show.error) {
      findings.push({
        severity: "blocked",
        msg: {
          key: "lighting.hub.finding.showError",
          params: { song: show.song, detail: show.error },
        },
        href,
        linkKey: "lighting.hub.fix.song",
        song: show.song,
      });
    }
    for (const w of show.warnings) {
      if (COVERED_ELSEWHERE_KINDS.has(w.kind)) continue;
      // A focus point the venue does not bind is fitted on the Fit shows
      // page, where it is placed on the plan; every other finding is fixed
      // in the song.
      const unbound = w.kind === "unbound-focus-point";
      findings.push({
        severity: SHOW_BREAKING_KINDS.has(w.kind) ? "attention" : "note",
        msg: {
          key: "lighting.hub.finding.lint",
          params: { detail: w.message },
        },
        href: unbound ? fitHref() : href,
        linkKey: unbound ? "lighting.hub.fix.fit" : "lighting.hub.fix.song",
        song: show.song,
      });
    }
  }
  return finish(
    "shows",
    findings,
    r.shows.length === 0
      ? { key: "lighting.hub.summary.showsNone" }
      : {
          key: "lighting.hub.summary.showsReady",
          params: { count: r.shows.length },
        },
    "lighting.hub.summary.showsBlocked",
  );
}

/** The five checks, in the order a show needs them. */
export function evaluateReadiness(
  r: Readiness,
  { profileName }: EvaluateOptions,
): Check[] {
  const configHref = profileName
    ? `#/config/${encodeURIComponent(profileName)}/lighting`
    : "#/config";
  const groupsHref = profileName
    ? `#/lighting/groups?profile=${encodeURIComponent(profileName)}`
    : "#/lighting/groups";

  const shows = showsCheck(r);

  if (!r.dmx) {
    // No DMX on this profile: no lighting system exists to ask about, so the
    // venue-shaped checks cannot run and the output is known to be missing.
    // The shows still can be.
    const noDmx = "lighting.hub.summary.noDmx";
    return [
      cannot("fixtures", noDmx),
      cannot("venue", noDmx),
      cannot("groups", noDmx),
      shows,
      finish(
        "output",
        [
          {
            severity: "blocked",
            msg: { key: "lighting.hub.finding.noDmx" },
            href: configHref,
            linkKey: "lighting.hub.fix.config",
          },
        ],
        { key: noDmx },
      ),
    ];
  }

  const venue = r.venue;
  const needsVenue = (key: CheckKey) =>
    cannot(key, "lighting.hub.summary.needsVenue");

  // Fixture types.
  const fixtures: Check = venue
    ? finish(
        "fixtures",
        r.fixture_types.unresolved.map((u) => ({
          severity: "blocked" as const,
          msg: {
            key: "lighting.hub.finding.typeUnresolved",
            params: { fixture: u.fixture, type: u.type, reason: u.reason },
          },
          href: "#/lighting/fixtures",
          linkKey: "lighting.hub.fix.fixtures",
        })),
        {
          key: "lighting.hub.summary.typesReady",
          params: { count: r.fixture_types.in_use.length },
        },
      )
    : needsVenue("fixtures");

  // Venue.
  const venueFindings: Finding[] = [];
  if (!venue) {
    venueFindings.push({
      severity: "blocked",
      msg: { key: "lighting.hub.finding.noVenue" },
      href: groupsHref,
      linkKey: "lighting.hub.fix.chooseVenue",
    });
  } else if (venue.fixtures === 0) {
    venueFindings.push({
      severity: "attention",
      msg: {
        key: "lighting.hub.finding.emptyVenue",
        params: { venue: venue.name },
      },
      href: "#/lighting/venues",
      linkKey: "lighting.hub.fix.venues",
    });
  }
  const venueCheck = finish(
    "venue",
    venueFindings,
    venue
      ? {
          key:
            venue.placed === venue.fixtures
              ? "lighting.hub.summary.venueAllPlaced"
              : "lighting.hub.summary.venueSomeUnplaced",
          params: {
            venue: venue.name,
            fixtures: venue.fixtures,
            unplaced: venue.fixtures - venue.placed,
          },
        }
      : { key: "lighting.hub.summary.needsVenue" },
  );

  // Groups.
  const groups: Check = venue
    ? finish(
        "groups",
        r.groups
          .filter((g) => g.fixtures === 0)
          .map((g) => ({
            severity: "attention" as const,
            msg: {
              key: "lighting.hub.finding.groupEmpty",
              params: {
                group: g.name,
                venue: venue.name,
                songs: g.songs.join(", "),
              },
            },
            href: fitHref(g.name),
            linkKey: "lighting.hub.fix.fit",
          })),
        r.groups.length === 0
          ? { key: "lighting.hub.summary.groupsNone" }
          : {
              key: "lighting.hub.summary.groupsReady",
              params: { count: r.groups.length },
            },
      )
    : needsVenue("groups");

  // Output.
  let output: Check;
  if (!venue) {
    output = needsVenue("output");
  } else {
    const findings: Finding[] = [];
    for (const universe of r.output.unconfigured) {
      findings.push({
        severity: "blocked",
        msg: {
          key: "lighting.hub.finding.universeUnconfigured",
          params: { universe },
        },
        href: fitHref(),
        linkKey: "lighting.hub.fix.fit",
      });
    }
    const olad = r.output.olad;
    if (olad && !olad.reachable) {
      findings.push({
        severity: "unknown",
        msg: { key: "lighting.hub.finding.oladUnreachable" },
        href: configHref,
        linkKey: "lighting.hub.fix.config",
      });
    } else if (olad) {
      for (const universe of olad.unpatched) {
        findings.push({
          severity: "blocked",
          msg: {
            key: "lighting.hub.finding.universeUnpatched",
            params: { universe },
          },
          href: fitHref(),
          linkKey: "lighting.hub.fix.fit",
        });
      }
    }
    output = finish("output", findings, {
      key: "lighting.hub.summary.outputReady",
      params: { count: r.output.universes.length },
    });
  }

  return [fixtures, venueCheck, groups, shows, output];
}
