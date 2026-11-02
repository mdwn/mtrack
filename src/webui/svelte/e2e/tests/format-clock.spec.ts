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

// The one clock every time readout in the UI uses (no browser).

import { test, expect } from "@playwright/test";
import { formatClock } from "../../src/lib/util/format";

test("whole seconds count down, as a clock does", () => {
  expect(formatClock(0)).toBe("0:00");
  expect(formatClock(1.9)).toBe("0:01");
  expect(formatClock(83)).toBe("1:23");
  // A hair short of a second, from floating point, is the second.
  expect(formatClock(1.9999999999)).toBe("0:02");
  expect(formatClock(3600)).toBe("60:00");
});

test("decimals round, carrying into the seconds and minutes", () => {
  expect(formatClock(83.456, 3)).toBe("1:23.456");
  expect(formatClock(1.234, 3)).toBe("0:01.234");
  expect(formatClock(12.5, 1)).toBe("0:12.5");
  expect(formatClock(10.06, 1)).toBe("0:10.1");
  expect(formatClock(59.9996, 3)).toBe("1:00.000");
});

test("the light show DSL's form pads the minutes", () => {
  expect(formatClock(5, 3, true)).toBe("00:05.000");
  expect(formatClock(754.25, 3, true)).toBe("12:34.250");
});

test("a time before 0 has a sign", () => {
  expect(formatClock(-1.5, 1)).toBe("-0:01.5");
});
