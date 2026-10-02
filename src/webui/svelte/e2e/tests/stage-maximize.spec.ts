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

// The stage card's maximize: the card fills the window under the
// navigation, in place, until it is pressed again or Escape.

import { test, expect, type Page } from "@playwright/test";

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

const METADATA = {
  type: "metadata",
  fixtures: {
    left: { tags: ["front"], type: "par", position: [-2, 3, 4], rig: null },
    right: { tags: ["front"], type: "par", position: [2, 3, 4], rig: null },
  },
  venue: { name: "test-venue", dir: null, focus_points: {} },
};

const card = (page: Page) => page.locator(".stage-card");
const button = (page: Page) => page.getByTestId("stage-maximize");
const bar = (page: Page) => page.locator(".miniplayer");

/** The card's box, and where the navigation ends. */
async function boxes(page: Page) {
  const box = (await card(page).boundingBox())!;
  const nav = (await page.locator(".topnav").boundingBox())!;
  const size = page.viewportSize()!;
  return { box, navBottom: nav.y + nav.height, size };
}

async function expectMaximized(page: Page) {
  await expect(button(page)).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(async () => {
      const { box, navBottom, size } = await boxes(page);
      return [
        Math.round(box.x),
        Math.round(box.y - navBottom),
        Math.round(box.width - size.width),
        Math.round(box.y + box.height - size.height),
      ];
    })
    .toEqual([0, 0, 0, 0]);
}

test.describe("Stage card: maximize", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `max-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.setViewportSize({ width: 1280, height: 800 });
  });

  test("the dashboard's card fills the window under the navigation and comes back", async ({
    page,
  }) => {
    await page.goto(`/?wsId=${wsId}#/`);
    await sendWsMessage(page, wsId, METADATA);
    await expect(button(page)).toHaveAttribute("aria-pressed", "false");
    const before = (await boxes(page)).box;
    const plot = page.locator(".stage-card__viewport > canvas");
    const plotBefore = (await plot.boundingBox())!;
    // At this width the playback bar is not shown.
    await expect(bar(page)).toBeHidden();

    await button(page).click();
    await expectMaximized(page);
    await expect(page).toHaveURL(/#\/$/);
    // The picture is what grew, and the navigation is still there.
    await expect
      .poll(async () => (await plot.boundingBox())!.width)
      .toBeGreaterThan(plotBefore.width * 1.5);
    await expect(page.locator(".topnav")).toBeVisible();
    // The card covers the dashboard's transport: the playback bar stands in.
    await expect(bar(page)).toBeVisible();

    await button(page).click();
    await expect(button(page)).toHaveAttribute("aria-pressed", "false");
    await expect
      .poll(async () => (await boxes(page)).box.width)
      .toBe(before.width);
    await expect(bar(page)).toBeHidden();

    // Escape restores too.
    await button(page).click();
    await expectMaximized(page);
    await page.keyboard.press("Escape");
    await expect(button(page)).toHaveAttribute("aria-pressed", "false");
    await expect(bar(page)).toBeHidden();
  });

  test("3D stays 3D through a maximize, with its camera where it was", async ({
    page,
  }) => {
    await page.goto(`/?wsId=${wsId}#/`);
    await sendWsMessage(page, wsId, METADATA);
    await page.getByTestId("stage-view-3d").click();
    const viewport = page.locator(".stage3d__viewport");
    await expect(viewport).toHaveAttribute("data-renderer", /webgl|none/, {
      timeout: 15000,
    });
    test.skip(
      (await viewport.getAttribute("data-renderer")) !== "webgl",
      "no WebGL in this browser",
    );
    const camera = async () =>
      JSON.parse((await viewport.getAttribute("data-view")) ?? "{}").camera as
        | number[]
        | undefined;
    await expect.poll(camera).toBeTruthy();
    const before = (await camera())!;
    const width = (await viewport.boundingBox())!.width;

    await button(page).click();
    await expectMaximized(page);
    await expect
      .poll(async () => (await viewport.boundingBox())!.width)
      .toBeGreaterThan(width * 1.5);
    await expect(page.getByTestId("stage-view-3d")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // The same scene, still drawing, seen from the same place.
    const frames = Number(await viewport.getAttribute("data-frames"));
    await expect
      .poll(async () => Number(await viewport.getAttribute("data-frames")))
      .toBeGreaterThan(frames);
    expect(await camera()).toEqual(before);
  });

  test("on Venues the inspector comes along, and Escape clears a selection first", async ({
    page,
  }) => {
    await page.goto(`/?wsId=${wsId}#/lighting/venues/test-venue`);
    await sendWsMessage(page, wsId, {
      ...METADATA,
      venue: { ...METADATA.venue, focus_points: { drummer: [0, 2.8, 1.4] } },
    });
    await button(page).click();
    await expectMaximized(page);
    const inspector = page.locator(".inspector");
    await expect(inspector).toBeVisible();
    await expect(page.locator(".stage-card__add-focus")).toBeVisible();

    // Escape in a field being typed in is the field's.
    const name = page.locator(".stage-card__focus-name");
    await name.fill("singer");
    await name.press("Escape");
    await expect(button(page)).toHaveAttribute("aria-pressed", "true");
    const tick = inspector.getByLabel("left", { exact: true });
    await tick.check();
    // Elsewhere: once for the selection, once for the card.
    await card(page).locator(".stage-card__title").click();
    await page.keyboard.press("Escape");
    await expect(tick).not.toBeChecked();
    await expect(button(page)).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(button(page)).toHaveAttribute("aria-pressed", "false");
  });

  test("leaving the page while maximized leaves nothing behind", async ({
    page,
  }) => {
    await page.goto(`/?wsId=${wsId}#/`);
    await sendWsMessage(page, wsId, METADATA);
    await button(page).click();
    await expectMaximized(page);
    await page.locator(".topnav").getByRole("tab", { name: "Songs" }).click();
    await expect(page).toHaveURL(/#\/songs/);
    await expect(bar(page)).toBeHidden();
    expect(
      await page.evaluate(() => [
        document.body.classList.contains("stage-maximized"),
        document.documentElement.style.overflow,
      ]),
    ).toEqual([false, ""]);
  });
});
