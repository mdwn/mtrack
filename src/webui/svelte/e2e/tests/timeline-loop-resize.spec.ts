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

// A loop's block resized on the timeline: the count the ghost shows while
// dragging is the count written on the drop, at every width, under a tempo
// ramp where the passes shorten one after another.

import { test, expect } from "@playwright/test";
import { loadRules } from "./rules-node";
import { rules } from "../../src/lib/rules";

test.beforeAll(async () => {
  await loadRules();
});

const SOURCE =
  "tempo {\n    bpm: 60\n    time_signature: 4/4\n    changes: [\n        @1/1 { bpm: 240, transition: 3m }\n    ]\n}\n" +
  'sequence "s", length: 1measure {\n    @1/1\n    a: static, color: "red", duration: 1beat\n}\n' +
  'show "S" {\n    @1/1\n    sequence "s", loop: 6\n}\n';

test("the ghost's count is the count a drop writes", async ({ page }) => {
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".lighting-section")).toBeVisible();
  const raw = page.locator(".tab-btn", { hasText: "Raw DSL" });
  const timeline = page.locator(".tab-btn", { hasText: "Timeline" });
  const textarea = page.locator(".raw-textarea");
  await raw.click();
  await textarea.fill(SOURCE);
  await timeline.click();

  let previous = 6;
  for (const dx of [40, 160, -60, -200, 25, 400]) {
    const handle = page.locator(".cue-block:has(.seq-label) .resize-handle");
    await expect(handle).toHaveCount(1);
    await handle.scrollIntoViewIfNeeded();
    const box = (await handle.boundingBox())!;
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + dx / 2, y, { steps: 4 });
    await page.mouse.move(x + dx, y, { steps: 4 });
    const ghost = page.locator(".ghost-label");
    await expect(ghost).toBeVisible();
    const shown = parseInt((await ghost.textContent())!, 10);
    await page.mouse.up();

    await raw.click();
    const read = rules.document(await textarea.inputValue()).document!;
    const written = read.shows[0].cues[0].sequences[0].loop;
    expect(written, `drag by ${dx}px`).toBe(String(shown));
    // A drag right never shrinks the loop; a drag left never grows it.
    if (dx > 0) expect(shown).toBeGreaterThanOrEqual(previous);
    else expect(shown).toBeLessThanOrEqual(previous);
    previous = shown;
    await timeline.click();
  }
});

test("a small drag keeps the count of a cue's first reference", async ({
  page,
}) => {
  // Two references: the block is as long as the longer, a bar; the
  // dragged one, the first, is two passes of a beat.
  const source =
    "tempo {\n    bpm: 120\n    time_signature: 4/4\n}\n" +
    'sequence "s", length: 1beat {\n    @1/1\n    a: static, color: "red", duration: 1beat\n}\n' +
    'sequence "t", length: 4beats {\n    @1/1\n    b: static, color: "blue", duration: 4beats\n}\n' +
    'show "S" {\n    @1/1\n    sequence "s", loop: 2\n    sequence "t", loop: 1\n}\n';
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".lighting-section")).toBeVisible();
  const raw = page.locator(".tab-btn", { hasText: "Raw DSL" });
  const textarea = page.locator(".raw-textarea");
  await raw.click();
  await textarea.fill(source);
  await page.locator(".tab-btn", { hasText: "Timeline" }).click();

  const handle = page.locator(".cue-block:has(.seq-label) .resize-handle");
  await handle.scrollIntoViewIfNeeded();
  const box = (await handle.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 5, y, { steps: 3 });
  await expect(page.locator(".ghost-label")).toHaveText("2x");
  await page.mouse.up();

  await raw.click();
  const read = rules.document(await textarea.inputValue()).document!;
  const refs = read.shows[0].cues[0].sequences;
  expect(refs.map((r) => r.loop)).toEqual(["2", "1"]);
});
