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
// Which controls the effect form offers per effect type.
//
// The form used to offer a colour on strobe, chase and pulse, an intensity and
// duty cycle on strobe, a dimmer on cycle and pulse, and a loop on cycle, chase
// and rainbow. The engine reads none of them: the parser accepts each one and
// drops it, so every one wrote a setting to the file that never took effect.
//
// Those controls were removed by deleting markup by line range, which
// `svelte-check` can only prove still compiles. This opens the form for real and
// asserts both halves: the removed controls are gone, and the ones the effect
// actually reads survived the deletion.

import { test, expect } from "@playwright/test";

/** Opens the timeline editor's properties panel for the first cue. */
async function openFirstCue(page: import("@playwright/test").Page) {
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  const cue = page.locator(".cue-block").first();
  await expect(cue).toBeVisible();
  // `force`: the cue sits over a canvas lane that intercepts pointer events.
  await cue.click({ force: true });
  // The properties panel opens with each effect collapsed to a summary row; the
  // controls live behind its expand toggle.
  const expand = page.locator(".effect-form .expand-toggle").first();
  await expect(expand).toBeVisible();
  await expand.click();
  await expect(page.locator(".param-label").first()).toBeVisible();
}

/** The parameter labels the form is currently showing. */
async function labels(
  page: import("@playwright/test").Page,
): Promise<string[]> {
  return (await page.locator(".param-label").allTextContents()).map((t) =>
    t.trim(),
  );
}

/** Switches the open effect to another type. */
async function setType(page: import("@playwright/test").Page, type: string) {
  const select = page.locator(".effect-form select").first();
  await select.selectOption(type);
  // Svelte re-renders the branch; wait for the labels to settle.
  await page.waitForTimeout(200);
}

test.describe("Effect form parameters", () => {
  test("a static effect keeps the controls it reads", async ({ page }) => {
    await openFirstCue(page);
    const shown = await labels(page);
    // `static` is the one type that reads a dimmer, and the deletion had to
    // leave that one alone.
    expect(shown).toContain("Dimmer");
    expect(shown).toContain("Duration");
  });

  test("a strobe offers frequency and duration, and nothing it ignores", async ({
    page,
  }) => {
    await openFirstCue(page);
    await setType(page, "strobe");
    const shown = await labels(page);

    expect(shown).toContain("Frequency");
    expect(shown).toContain("Duration");
    // `EffectType::Strobe` carries a frequency and a duration and nothing else.
    expect(shown).not.toContain("Intensity");
    expect(shown).not.toContain("Duty");
    expect(shown).not.toContain("Dimmer");
  });

  test("a cycle keeps speed and direction but offers no dimmer or loop", async ({
    page,
  }) => {
    await openFirstCue(page);
    await setType(page, "cycle");
    const shown = await labels(page);

    // The controls either side of the deleted blocks — proof the line-range
    // deletion did not take a neighbour with it.
    expect(shown).toContain("Speed");
    expect(shown).toContain("Direction");
    expect(shown).toContain("Duration");
    expect(shown).not.toContain("Dimmer");
    expect(shown).not.toContain("Loop");
  });

  test("a chase keeps its pattern and offers no colour or loop", async ({
    page,
  }) => {
    await openFirstCue(page);
    await setType(page, "chase");
    const shown = await labels(page);

    expect(shown).toContain("Speed");
    expect(shown).toContain("Duration");
    expect(shown).not.toContain("Loop");
  });

  test("a rainbow offers no direction or loop", async ({ page }) => {
    await openFirstCue(page);
    await setType(page, "rainbow");
    const shown = await labels(page);

    expect(shown).toContain("Duration");
    // The control that started all of this: `EffectType::Rainbow` has no
    // direction, so the picker wrote a setting that did nothing.
    expect(shown).not.toContain("Direction");
    expect(shown).not.toContain("Loop");
  });
});
