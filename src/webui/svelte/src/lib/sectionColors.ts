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

/** The rotating palette for sections without an explicit color. */
export const SECTION_COLORS = [
  "#5ecaea",
  "#8b5cf6",
  "#eab308",
  "#ef60a3",
  "#22c55e",
  "#f97316",
];

/** A section's display color: its own, or the palette rotation slot. */
export function sectionColor(color: string | undefined, index: number): string {
  return color ?? SECTION_COLORS[index % SECTION_COLORS.length];
}
