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
 * The Lighting overview's five checks as the page shows them. The verdicts
 * are the player's (`Readiness.checks`); what is here is the wording of
 * each part, a translatable message, and the link to where a finding is
 * fixed (design/lighting_ui_design.md, section 8.1).
 */

import type { Fix, FindingWhat, Summary } from "../../gen/rules/mtrack_rules";

/** A translatable message: an i18n key and its parameters. */
export interface Msg {
  key: string;
  params?: Record<string, string | number>;
}

/** What a finding says. */
export function findingMsg({ kind, ...params }: FindingWhat): Msg {
  return { key: `lighting.hub.finding.${kind}`, params };
}

/** What a part of a check's summary says. */
export function summaryMsg({ kind, ...params }: Summary): Msg {
  return { key: `lighting.hub.summary.${kind}`, params };
}

/** The Fit shows page, optionally with a group selected. */
export function fitHref(group?: string | null): string {
  return group
    ? `#/lighting/fit?group=${encodeURIComponent(group)}`
    : "#/lighting/fit";
}

/** Where a song's lighting is edited. */
export function songLightingHref(song: string): string {
  return `#/songs/${encodeURIComponent(song)}/lighting`;
}

/** Where a finding is fixed. `profileName` is the running profile's, for
 *  its DMX settings. */
export function fixHref(fix: Fix, profileName: string | null): string {
  const profile = profileName ? encodeURIComponent(profileName) : null;
  switch (fix.kind) {
    case "song":
      return songLightingHref(fix.song);
    case "fit":
      return fitHref(fix.group);
    case "fixtures":
      return "#/lighting/fixtures";
    case "venues":
      return "#/lighting/venues";
    case "config":
      return profile ? `#/config/${profile}/lighting` : "#/config";
    case "chooseVenue":
      return profile
        ? `#/lighting/groups?profile=${profile}`
        : "#/lighting/groups";
  }
}
