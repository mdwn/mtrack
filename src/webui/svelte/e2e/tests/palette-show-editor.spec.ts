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

// The show editor's palette: the panel that adds, recolours, renames and
// removes a file's slots, the colour inputs that offer a slot beside a literal,
// and the timeline blocks that paint a slot's colour. The edits are the
// player's (`rules.palette_edit`), so the file saved is what these read.

import { test, expect, type Page } from "@playwright/test";

const SHOW = `tempo {
    start: 0.0s
    bpm: 120.0
    time_signature: 4/4
}

palette {
    primary: "#c01020"
    secondary: "orange"
}

sequence "hit"(color: palette.primary, other: "front"), length: 1measure {
    @1/1
    flash: static color: $color, duration: 1beat
}

show "Main" {
    @00:00.000
    front: static color: palette.primary, duration: 5s
    @2/1
    sequence "hit"(color: palette.primary)
    @00:06.000
    back: cycle color: palette.primary, color: palette.secondary, speed: 1, duration: 4s
}
`;

/** Serves SHOW as the song's light file, and collects what is saved to it. */
async function stubShow(page: Page): Promise<{ saved: string[] }> {
  const saved: string[] = [];
  await page.route(/\/api\/lighting\/show\.light$/, (route) => {
    const method = route.request().method();
    if (method === "PUT") {
      saved.push(route.request().postData() ?? "");
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved" }),
      });
    }
    if (method === "GET") {
      return route.fulfill({
        status: 200,
        contentType: "text/plain",
        body: SHOW,
      });
    }
    return route.fallback();
  });
  return { saved };
}

async function openEditor(page: Page) {
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".cue-block").first()).toBeVisible();
  await page.getByTestId("palette-card").locator("summary").click();
  await expect(page.getByTestId("palette-0-panel")).toBeVisible();
}

/** Opens the first cue's first effect, ready for its colors. */
async function openFirstEffect(page: Page) {
  // `force`: the cue sits over a canvas lane that intercepts pointer events.
  await page.locator(".cue-block").first().click({ force: true });
  const expand = page.locator(".effect-form .expand-toggle").first();
  await expect(expand).toBeVisible();
  await expand.click();
}

async function save(page: Page) {
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".unsaved")).toHaveCount(0);
}

test.describe("Show editor: the palette", () => {
  test("lists the file's slots with their colors", async ({ page }) => {
    await stubShow(page);
    await openEditor(page);
    const slots = page.getByTestId("palette-0-slot");
    await expect(slots).toHaveCount(2);
    await expect(slots.nth(0).getByTestId("palette-0-name")).toHaveValue(
      "primary",
    );
    await expect(slots.nth(0).locator("input[type=color]")).toHaveValue(
      "#c01020",
    );
    // A name reads as its hex: `orange` is #ffa500.
    await expect(slots.nth(1).locator("input[type=color]")).toHaveValue(
      "#ffa500",
    );
    await expect(page.getByTestId("palette-count")).toHaveText("2");
  });

  test("a slot is added, used in an effect, and saved with both", async ({
    page,
  }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    await page.getByTestId("palette-0-new-name").fill("accent");
    await page.locator("input[aria-label='New slot color']").fill("#00ff80");
    await page.getByTestId("palette-0-add").click();
    await expect(page.getByTestId("palette-0-slot")).toHaveCount(3);

    await openFirstEffect(page);
    const source = page.getByTestId("color-source").first();
    // The choice is a literal or any of the file's slots.
    await expect(source.locator("option")).toHaveText([
      "Custom color",
      "Palette: primary",
      "Palette: secondary",
      "Palette: accent",
    ]);
    await expect(source).toHaveValue("primary");
    await source.selectOption("accent");
    await expect(page.getByTestId("color-slot-swatch").first()).toBeVisible();
    // The preview paints the slot: the cue block takes its color.
    await expect
      .poll(() =>
        page
          .locator(".cue-block")
          .first()
          .evaluate((el) => el.style.getPropertyValue("--cue-color")),
      )
      .toBe("#00ff80");

    await save(page);
    expect(saved).toHaveLength(1);
    expect(saved[0]).toContain('accent: "#00ff80"');
    expect(saved[0]).toContain("front: static color: palette.accent");
    // The rest of the file is as it was.
    expect(saved[0]).toContain('secondary: "orange"');
    expect(saved[0]).toContain("color: palette.secondary");
  });

  test("a slot goes back to a literal color as the color it was", async ({
    page,
  }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    await openFirstEffect(page);
    await page.getByTestId("color-source").first().selectOption("");
    await expect(page.locator(".color-text").first()).toHaveValue("#c01020");
    await save(page);
    expect(saved[0]).toContain('front: static color: "#c01020"');
  });

  test("renaming a slot rewrites every reference to it", async ({ page }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    const name = page.getByTestId("palette-0-name").first();
    await name.fill("lead");
    await name.press("Enter");
    await expect(name).toHaveValue("lead");

    // The effect form follows the rename.
    await openFirstEffect(page);
    await expect(page.getByTestId("color-source").first()).toHaveValue("lead");

    await save(page);
    expect(saved[0]).toContain('lead: "#c01020"');
    expect(saved[0]).toContain("front: static color: palette.lead");
    expect(saved[0]).toContain("color: palette.lead, color: palette.secondary");
    expect(saved[0]).not.toContain("palette.primary");
  });

  test("a rename onto a name in use, or to none, is refused and kept as it was", async ({
    page,
  }) => {
    await stubShow(page);
    await openEditor(page);
    const name = page.getByTestId("palette-0-name").first();
    await name.fill("secondary");
    await name.press("Enter");
    await expect(page.getByTestId("palette-0-error")).toContainText(
      "A slot named secondary already exists.",
    );
    await expect(name).toHaveValue("primary");
    await name.fill("two words");
    await name.press("Enter");
    await expect(page.getByTestId("palette-0-error")).toContainText(
      "starts with a letter",
    );
    await expect(name).toHaveValue("primary");
  });

  test("a slot in use is not removed, and the uses are listed", async ({
    page,
  }) => {
    await stubShow(page);
    await openEditor(page);
    await page.getByTestId("palette-0-remove").first().click();
    await expect(page.getByTestId("palette-0-error")).toContainText(
      "cannot be removed",
    );
    const uses = page.getByTestId("palette-0-uses").locator("li");
    await expect(uses).toHaveText([
      "sequence hit, color",
      "show Main, @00:00.000, color",
      "show Main, @2/1, color",
      "show Main, @00:06.000, color",
    ]);
    await expect(page.getByTestId("palette-0-slot")).toHaveCount(2);
  });

  test("a slot nothing uses is removed", async ({ page }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    await page.getByTestId("palette-0-new-name").fill("spare");
    await page.getByTestId("palette-0-add").click();
    await expect(page.getByTestId("palette-0-slot")).toHaveCount(3);
    await page
      .getByTestId("palette-0-slot")
      .and(page.locator("[data-slot=spare]"))
      .getByTestId("palette-0-remove")
      .click();
    await expect(page.getByTestId("palette-0-slot")).toHaveCount(2);
    await expect(page.getByTestId("palette-0-error")).toHaveCount(0);
    await save(page);
    expect(saved[0]).not.toContain("spare");
  });

  test("a sequence's color argument takes a slot too", async ({ page }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    await page
      .locator(".cue-block:has(.seq-label)")
      .first()
      .click({ force: true });
    const args = page.getByTestId("sequence-color-args");
    await expect(args).toBeVisible();
    // Only the color parameter is offered; `other` is not a color's.
    await expect(args.locator(".field")).toHaveCount(1);
    const source = args.getByTestId("color-source");
    await expect(source).toHaveValue("primary");
    await source.selectOption("secondary");
    await save(page);
    expect(saved[0]).toContain('sequence "hit"(color: palette.secondary)');
    // The sequence's own default is untouched.
    expect(saved[0]).toContain('sequence "hit"(color: palette.primary,');
  });

  test("an edit on the timeline keeps the palette", async ({ page }) => {
    const { saved } = await stubShow(page);
    await openEditor(page);
    await openFirstEffect(page);
    await page.locator(".effect-form select").first().selectOption("strobe");
    await save(page);
    expect(saved[0]).toContain("palette {");
    expect(saved[0]).toContain('primary: "#c01020"');
  });
});
