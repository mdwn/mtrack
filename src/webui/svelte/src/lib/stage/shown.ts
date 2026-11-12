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

import type { StageVenue } from "../../gen/rules/mtrack_rules";

/**
 * A venue as a plot shows it: the live venue as the stage stream says it, or
 * a venue's file picked on the Venues page. A file's `dir` is the venues
 * directory it was read from (a profile's override), which its saves name;
 * the live venue has none, and a save of it uses the configured directory.
 */
export type ShownVenue = StageVenue & { dir?: string | null };
