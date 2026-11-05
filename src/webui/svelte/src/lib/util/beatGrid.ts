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

/** "1:23.456" — the format the position readout shows and accepts. */
export function formatSeconds(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(3).padStart(6, "0")}`;
}

/** "1:23", for tick labels where milliseconds would be noise. */
export function formatSecondsShort(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds - minutes * 60);
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/** Parses "1:23.456", "83.456" or "83"; null when it is not a time. */
export function parseSeconds(raw: string): number | null {
  const match = /^(?:(\d+):)?(\d+(?:[.,]\d+)?)$/.exec(raw.trim());
  if (!match) return null;
  const minutes = match[1] ? parseInt(match[1]) * 60 : 0;
  return minutes + parseFloat(match[2].replace(",", "."));
}
