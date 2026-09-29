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
  /** The one-line summary the card shows. */
  summary: Msg;
  findings: Finding[];
  /** Findings that count towards the card's number (everything but notes). */
  count: number;
}

/** Lint kinds that stop a cue doing what it says: the Shows check needs
 *  attention for these. */
const SHOW_BREAKING_KINDS = new Set(["capability-gap", "unbound-focus-point"]);

/** Lint kinds another check already reports, so listing them under the song
 *  as well would say everything twice. */
const COVERED_ELSEWHERE_KINDS = new Set([
  "empty-group",
  "unconfigured-universe",
]);

const SEVERITY_ORDER: Finding["severity"][] = [
  "blocked",
  "attention",
  "unknown",
];

function stateOf(findings: Finding[]): CheckState {
  for (const s of SEVERITY_ORDER) {
    if (findings.some((f) => f.severity === s)) return s as CheckState;
  }
  return "ready";
}

function finish(
  key: CheckKey,
  findings: Finding[],
  readySummary: Msg,
  problemSummary: (count: number) => Msg,
): Check {
  const count = findings.filter((f) => f.severity !== "note").length;
  const state = stateOf(findings);
  return {
    key,
    state,
    findings,
    count,
    summary: state === "ready" ? readySummary : problemSummary(count),
  };
}

export const CHECK_ORDER: CheckKey[] = [
  "fixtures",
  "venue",
  "groups",
  "shows",
  "output",
];

/** Where a song's lighting is edited. */
export function songLightingHref(song: string): string {
  return `#/songs/${encodeURIComponent(song)}/lighting`;
}

export interface EvaluateOptions {
  /** The running profile's name, for the link into its DMX settings. */
  profileName: string | null;
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

  if (!r.dmx) {
    // No DMX on this profile: there is no lighting system to ask about, so
    // only the output is known, and it is known to be missing.
    const needsDmx = (key: CheckKey): Check => ({
      key,
      state: "unknown",
      count: 0,
      findings: [],
      summary: { key: "lighting.hub.summary.needsDmx" },
    });
    return CHECK_ORDER.map((key) =>
      key === "output"
        ? finish(
            "output",
            [
              {
                severity: "blocked",
                msg: { key: "lighting.hub.finding.noDmx" },
                href: configHref,
                linkKey: "lighting.hub.fix.config",
              },
            ],
            { key: "lighting.hub.summary.needsDmx" },
            (count) => ({
              key: "lighting.hub.summary.blocked",
              params: { count },
            }),
          )
        : needsDmx(key),
    );
  }

  const venue = r.venue;
  const needsVenue = (key: CheckKey): Check => ({
    key,
    state: "unknown",
    count: 0,
    findings: [],
    summary: { key: "lighting.hub.summary.needsVenue" },
  });

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
        (count) => ({ key: "lighting.hub.summary.blocked", params: { count } }),
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
    (count) => ({
      key: venue
        ? "lighting.hub.summary.attention"
        : "lighting.hub.summary.blocked",
      params: { count },
    }),
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
            href: "#/lighting/groups",
            linkKey: "lighting.hub.fix.groups",
          })),
        r.groups.length === 0
          ? { key: "lighting.hub.summary.groupsNone" }
          : {
              key: "lighting.hub.summary.groupsReady",
              params: { count: r.groups.length },
            },
        (count) => ({
          key: "lighting.hub.summary.attention",
          params: { count },
        }),
      )
    : needsVenue("groups");

  // Shows.
  const showFindings: Finding[] = [];
  for (const show of r.shows) {
    const href = songLightingHref(show.song);
    if (show.error) {
      showFindings.push({
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
      showFindings.push({
        severity: SHOW_BREAKING_KINDS.has(w.kind) ? "attention" : "note",
        msg: {
          key: "lighting.hub.finding.lint",
          params: { kind: w.kind, detail: w.message },
        },
        href,
        linkKey: "lighting.hub.fix.song",
        song: show.song,
      });
    }
  }
  const shows = finish(
    "shows",
    showFindings,
    r.shows.length === 0
      ? { key: "lighting.hub.summary.showsNone" }
      : {
          key: "lighting.hub.summary.showsReady",
          params: { count: r.shows.length },
        },
    (count) => ({
      key: showFindings.some((f) => f.severity === "blocked")
        ? "lighting.hub.summary.blocked"
        : "lighting.hub.summary.attention",
      params: { count },
    }),
  );

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
        href: configHref,
        linkKey: "lighting.hub.fix.config",
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
          href: configHref,
          linkKey: "lighting.hub.fix.config",
        });
      }
    }
    output = finish(
      "output",
      findings,
      {
        key: "lighting.hub.summary.outputReady",
        params: { count: r.output.universes.length },
      },
      (count) => ({
        key: findings.some((f) => f.severity === "blocked")
          ? "lighting.hub.summary.blocked"
          : "lighting.hub.summary.unknown",
        params: { count },
      }),
    );
  }

  return [fixtures, venueCheck, groups, shows, output];
}
