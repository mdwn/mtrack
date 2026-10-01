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
 * The facts a GDTF fixture type's page and card quote, as plain values the
 * components put into words. A fact the archive does not state is absent —
 * never 0 or "unknown".
 */

import type { FixtureTypeGdtf } from "../api/config";

/** A number without trailing noise: `13`, `0.4`, `5500`. */
export function trimNumber(value: number): string {
  return String(Number(value.toFixed(2)));
}

const stated = (value: number | null | undefined): value is number =>
  typeof value === "number" && Number.isFinite(value);

/** The beam's kind and angles, each only when stated; null when none is. */
export function beamFacts(
  beam: FixtureTypeGdtf["beam"],
): { type?: string; beam?: string; field?: string } | null {
  if (!beam) return null;
  const out: { type?: string; beam?: string; field?: string } = {};
  if (beam.type && beam.type.trim()) out.type = beam.type.trim();
  if (stated(beam.beam_angle)) out.beam = trimNumber(beam.beam_angle);
  if (stated(beam.field_angle)) out.field = trimNumber(beam.field_angle);
  return Object.keys(out).length > 0 ? out : null;
}

/** Luminous flux and colour temperature, each only when stated. */
export function outputFacts(
  beam: FixtureTypeGdtf["beam"],
): { flux?: string; cct?: string } | null {
  if (!beam) return null;
  const out: { flux?: string; cct?: string } = {};
  if (stated(beam.luminous_flux)) out.flux = trimNumber(beam.luminous_flux);
  if (stated(beam.color_temperature))
    out.cct = trimNumber(beam.color_temperature);
  return Object.keys(out).length > 0 ? out : null;
}

/** Watts, when stated. */
export function powerFact(beam: FixtureTypeGdtf["beam"]): string | null {
  return beam && stated(beam.power) ? trimNumber(beam.power) : null;
}

/** Who uses a mode, per venue: the fixture names, or only a count once
 *  there are more than `max` of them. */
export interface VenueUse {
  venue: string;
  count: number;
  /** Null when the list would be too long to read. */
  names: string[] | null;
}

/** The venue fixtures of a type, per venue — all of them, or only those
 *  driven in `mode` (the archive's spelling) when it is given. */
export function venueUse(
  venues: FixtureTypeGdtf["venues"],
  mode?: string,
  max = 8,
): VenueUse[] {
  return venues
    .map((v) => ({
      venue: v.name,
      fixtures: v.fixtures
        .filter((f) => mode === undefined || f.mode === mode)
        .map((f) => f.name),
    }))
    .filter((v) => v.fixtures.length > 0)
    .map((v) => ({
      venue: v.venue,
      count: v.fixtures.length,
      names: v.fixtures.length <= max ? v.fixtures : null,
    }));
}

/** How many venue fixtures are driven in each mode. */
export function modeCounts(
  venues: FixtureTypeGdtf["venues"],
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const v of venues)
    for (const f of v.fixtures)
      if (f.mode !== null) counts.set(f.mode, (counts.get(f.mode) ?? 0) + 1);
  return counts;
}

/** Venue fixtures whose mode names no mode of the archive: they cannot be
 *  driven, and their venue does not load. */
export function unresolvedFixtures(
  venues: FixtureTypeGdtf["venues"],
): { venue: string; fixture: string }[] {
  return venues.flatMap((v) =>
    v.fixtures
      .filter((f) => f.mode === null)
      .map((f) => ({ venue: v.name, fixture: f.name })),
  );
}
