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
// The section edit dialog's boundary handling.
//
// Nothing reached this component before. `setBoundary` is where the editor has
// to agree with the player about which positions exist:
//
//   - A boundary that would invert the section is walked back to the last
//     position that does not, rather than refused. The picker is a controlled
//     input, so a silent refusal reads as a broken button, and an inverted range
//     is what `Section::validate` rejects on save.
//   - The walk stops at the ends of the song instead of looping.
//
// The fixture song is 16 measures of 4/4 at 120bpm and carries a "stab" section
// whose start and end both sit inside measure 10 — a section can only be
// inverted when its boundaries are close enough for a step to cross them, which
// is why the fixture needed one.

import { test, expect } from "@playwright/test";

/** Opens the sections tab for the fixture song and taps a section open. */
async function openSectionDialog(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.goto("/#/songs/Test%20Song%20Gamma");
  await page.locator(".tab", { hasText: "Timeline" }).click();
  const block = page.locator(".section-block", { hasText: name });
  await expect(block).toBeVisible();
  // The component listens for pointer events on a bar with overlays, so a
  // plain click fails actionability. This is the gesture it actually treats as
  // a tap: press and release without moving.
  const box = await block.boundingBox();
  if (!box) throw new Error(`section "${name}" has no box`);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.up();
  await expect(page.locator(".position-picker").first()).toBeVisible();
}

/** The position a picker is showing, as "measure.beat". */
async function readout(
  page: import("@playwright/test").Page,
  label: string,
): Promise<string> {
  const picker = page
    .locator(".labeled-picker", { hasText: label })
    .locator(".position-picker");
  return ((await picker.locator(".pp-value").textContent()) ?? "").trim();
}

/** The first two numbers in a readout, as measure and beat.
 *
 * Written against the numbers rather than a format: stripping non-digits and
 * splitting on "." silently concatenates "m10 b1" into 101, which compares as
 * measure 101 and made two different positions look identical. */
function parsePos(text: string): { measure: number; beat: number } {
  // The readout must be a measure/beat position. The picker can render a clock
  // time instead, and "0:18.0" parses to a perfectly plausible measure 0 beat
  // 18 — so a regression that flips the unit would be compared as an ordering
  // fact rather than failing.
  if (!/^m\d+/.test(text.trim())) {
    throw new Error(`expected a measure/beat readout, got "${text}"`);
  }
  // Half beats render as a fraction glyph — "m10 \u00b7 b1\u00bd" — which a
  // plain number match reads as 1, making a walked-forward boundary look
  // identical to one that never moved.
  const normalised = text.replace(/(\d+)\u00bd/g, (_, whole) => `${whole}.5`);
  const numbers = normalised.match(/\d+(?:\.\d+)?/g) ?? [];
  return {
    measure: Number.parseFloat(numbers[0] ?? "0"),
    beat: Number.parseFloat(numbers[1] ?? "1"),
  };
}

/** A position as a single comparable number, for ordering assertions. */
function asTime(pos: { measure: number; beat: number }): number {
  return pos.measure * 100 + pos.beat;
}

/** Clicks a picker's next-beat button `times` times. */
async function nextBeat(
  page: import("@playwright/test").Page,
  label: string,
  times: number,
) {
  const button = page
    .locator(".labeled-picker", { hasText: label })
    .getByRole("button", { name: `${label}: next beat` });
  for (let i = 0; i < times; i++) {
    await button.click();
  }
}

test.describe("Section edit dialog", () => {
  test("a start pushed past the end is walked back, not refused", async ({
    page,
  }) => {
    await openSectionDialog(page, "stab");

    const startLabel = "Start";
    const before = await readout(page, startLabel);
    expect(before).not.toBe("");

    // Both boundaries are inside measure 10, so stepping the start forward
    // enough asks for a position at or past the end.
    await nextBeat(page, startLabel, 8);

    const start = parsePos(await readout(page, startLabel));
    const end = parsePos(await readout(page, "End"));

    // Still a forward range. A refusal would have left the start where it was;
    // an unclamped step would have put it at or past the end.
    expect(asTime(start)).toBeLessThan(asTime(end));
    expect(await readout(page, startLabel)).not.toBe(before);
  });

  test("an end dragged back past the start is walked forward", async ({
    page,
  }) => {
    await openSectionDialog(page, "stab");

    const endLabel = "End";
    const button = page
      .locator(".labeled-picker", { hasText: endLabel })
      .getByRole("button", { name: `${endLabel}: previous beat` });
    for (let i = 0; i < 8; i++) {
      await button.click();
    }

    const start = parsePos(await readout(page, "Start"));
    const end = parsePos(await readout(page, endLabel));
    // Walked forward off the start rather than left on top of it.
    expect(asTime(start)).toBeLessThan(asTime(end));
  });

  // Weaker than the two above, and recorded as such: with the clamp disabled
  // this still passes, because the picker's own bounds happen to keep the two
  // boundaries apart. Kept as a regression guard against the walk running off
  // the end of the song, not as evidence the clamp works.
  test("the section is still saveable after both boundaries are driven to their limits", async ({
    page,
  }) => {
    await openSectionDialog(page, "stab");

    await nextBeat(page, "Start", 12);
    await nextBeat(page, "End", 12);

    const start = parsePos(await readout(page, "Start"));
    const end = parsePos(await readout(page, "End"));
    expect(asTime(start)).toBeLessThan(asTime(end));
    // Within the song: the fixture grid is 16 measures.
    expect(end.measure).toBeLessThanOrEqual(16);
  });
});
