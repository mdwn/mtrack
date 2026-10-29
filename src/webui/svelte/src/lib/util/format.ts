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

/** Formats milliseconds as "m:ss". */
export function formatMs(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/** A fade length as written (`1measure`, `4beats`, `2s`) for reading:
 *  beats and measures spaced from their count. */
export function formatFadeLength(length: string): string {
  const m = /^\s*([\d.]+)\s*(beats?|measures?)\s*$/.exec(length);
  return m ? `${m[1]} ${m[2]}` : length.trim();
}

/** A gap in milliseconds as seconds: `2s`, `1.5s`. */
export function formatGapMs(ms: number): string {
  return `${Math.round(ms / 100) / 10}s`;
}

/** Whether a fade length turns the fade off (`0s`, `0ms`, `0`). */
export function isZeroLength(length: string): boolean {
  return /^\s*0\s*(s|ms)?\s*$/.test(length);
}
