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
 * A time as a clock reads it: `m:ss`, with `digits` decimals of a second
 * (`m:ss.s`, `m:ss.mmm`), minutes zero-padded to two with `padMinutes` (the
 * light show DSL's `@mm:ss.mmm`). Whole seconds are counted down (a clock
 * shows 0:01 until 2s), decimals rounded to the nearest; a time before 0
 * has a minus sign. Every clock in the UI is this one.
 */
export function formatClock(
  seconds: number,
  digits: 0 | 1 | 3 = 0,
  padMinutes = false,
): string {
  const sign = seconds < 0 ? "-" : "";
  const scale = 10 ** digits;
  const abs = Math.abs(seconds) * scale;
  // A whisker over, so a time that floating point puts a hair short of a
  // second (1.9999999) reads as the second.
  const units = digits === 0 ? Math.floor(abs + 1e-6) : Math.round(abs);
  const minutes = Math.floor(units / (60 * scale));
  const rest = units - minutes * 60 * scale;
  const secs = Math.floor(rest / scale);
  const frac = rest - secs * scale;
  const mm = padMinutes ? String(minutes).padStart(2, "0") : String(minutes);
  const ss = String(secs).padStart(2, "0");
  return digits === 0
    ? `${sign}${mm}:${ss}`
    : `${sign}${mm}:${ss}.${String(frac).padStart(digits, "0")}`;
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
