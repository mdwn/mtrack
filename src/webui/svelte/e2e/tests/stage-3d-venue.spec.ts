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

import { test, expect, type Page } from "@playwright/test";

// Plot | 3D on the venue's stage card: the picture changes in place; the
// venue, the selection, the inspector and the header stay. The current
// venue is drawn live; any other from its file.

let counter = 0;

/** The engine's current venue: "built-in", with one placed PAR. */
const BUILT_IN = {
  type: "metadata",
  fixtures: {
    "house-par": {
      tags: [],
      type: "par",
      position: [0, 2, 4],
      rotation: [0, 0, 0],
      rig: null,
    },
  },
  venue: { name: "built-in", dir: null, focus_points: {} },
};

/** "club" as its file reads: one fixture placed, two not. */
const CLUB_SCENE = {
  fixtures: {
    A: {
      tags: [],
      type: "par",
      mode: null,
      position: [-1, 2, 3],
      rotation: [0, 0, 0],
      rig: null,
    },
    B: {
      tags: [],
      type: "par",
      mode: null,
      position: null,
      rotation: null,
      rig: null,
    },
    C: {
      tags: [],
      type: "par",
      mode: null,
      position: null,
      rotation: null,
      rig: null,
    },
  },
  venue: {
    name: "club",
    focus_points: { dj: [0, 3, 1] },
    scenery: null,
    scenery_error: null,
  },
};

/** Opens `path` with "built-in" current; "club"'s scene is CLUB_SCENE. */
async function withCurrentVenue(page: Page, path: string) {
  const wsId = `s3dv-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  let sceneReads = 0;
  await page.route(/\/api\/lighting\/venues\/club\/scene(\?.*)?$/, (r) => {
    sceneReads++;
    return r.fulfill({ json: CLUB_SCENE });
  });
  await page.goto(`/?wsId=${wsId}${path}`);
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...BUILT_IN, _wsId: wsId },
  });
  return { wsId, reads: () => sceneReads };
}

const viewport = (page: Page) => page.locator(".stage3d__viewport");
const plot = (page: Page) => page.locator(".stage-card__viewport canvas");
const plotButton = (page: Page) => page.getByTestId("stage-view-plot");
const threeButton = (page: Page) => page.getByTestId("stage-view-3d");

async function rendered(page: Page): Promise<boolean> {
  await expect(viewport(page)).toHaveAttribute("data-renderer", /webgl|none/, {
    timeout: 15000,
  });
  return (await viewport(page).getAttribute("data-renderer")) === "webgl";
}

/** Where the 3D view draws a fixture, in page pixels. */
async function onScreen(page: Page, name: string) {
  await expect
    .poll(async () => {
      const raw = (await viewport(page).getAttribute("data-view")) ?? "{}";
      return name in ((JSON.parse(raw).screen ?? {}) as object);
    })
    .toBe(true);
  const view = JSON.parse((await viewport(page).getAttribute("data-view"))!);
  const [fx, fy] = view.screen[name] as [number, number];
  const box = (await viewport(page).boundingBox())!;
  return { x: box.x + fx * box.width, y: box.y + fy * box.height };
}

test("Plot | 3D switches the picture in place, in the address", async ({
  page,
}) => {
  const three: string[] = [];
  page.on("request", (r) => {
    if (/three|scene3d|Stage3DView|PreviewPanel/.test(r.url()))
      three.push(r.url());
  });
  await withCurrentVenue(page, "#/lighting/venues/club");
  await expect(page.getByTestId("stage-venue-label")).toContainText(
    "Venue file: club",
  );
  await expect(plot(page)).toBeVisible();
  await expect(plotButton(page)).toHaveAttribute("aria-pressed", "true");
  // Nobody pressed 3D: three.js and the 3D code were never fetched.
  await page.waitForTimeout(500);
  expect(three).toEqual([]);

  await threeButton(page).click();
  await expect(page).toHaveURL(/#\/lighting\/venues\/club\?view=3d$/);
  await expect(threeButton(page)).toHaveAttribute("aria-pressed", "true");
  await expect(plot(page)).toHaveCount(0);
  await expect(viewport(page)).toHaveAttribute("data-source", "file");
  await expect(viewport(page)).toHaveAttribute("data-venue", "club");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
  await expect(viewport(page)).toHaveAttribute("data-placed", "1");
  expect(three.length).toBeGreaterThan(0);
  // The card stays what it was: its header, its inspector, its focus point
  // button.
  await expect(page.getByTestId("stage-venue-label")).toContainText(
    "Venue file: club",
  );
  await expect(page.locator(".inspector")).toBeVisible();
  await expect(page.locator(".stage-card__add-focus")).toBeVisible();
  // Not the current venue: no Preview, and the line says how to get one.
  await expect(page.getByTestId("stage3d-mode-preview")).toHaveCount(0);
  await expect(page.getByTestId("stage3d-no-preview")).toContainText(
    "Groups page",
  );
  // The unplaced fixtures, inside the card.
  const unplaced = page.locator(".stage-card").getByTestId("stage3d-unplaced");
  await expect(unplaced).toContainText(
    "2 fixtures have no position yet — drawn in a row in front of the stage.",
  );
  await expect(page.getByTestId("stage3d-grid")).toHaveText("Grid: 1 m");
  await expect(page.getByTestId("stage3d-hint")).toHaveText(
    "Drag to orbit · click a fixture to select it",
  );

  // A reload keeps 3D.
  await page.reload();
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");

  // Plot returns, and the address loses the view.
  await plotButton(page).click();
  await expect(page).toHaveURL(/#\/lighting\/venues\/club$/);
  await expect(plot(page)).toBeVisible();
  // Back is 3D again.
  await page.goBack();
  await expect(page).toHaveURL(/\?view=3d$/);
  await expect(viewport(page)).toBeVisible();
});

test("the current venue's 3D is live, with Live | Preview", async ({
  page,
}) => {
  const { reads } = await withCurrentVenue(
    page,
    "#/lighting/venues/built-in?view=3d",
  );
  await expect(page.getByTestId("stage-venue-label")).toContainText(
    "Current venue: built-in",
  );
  await expect(viewport(page)).toHaveAttribute("data-source", "live");
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "1");
  await expect(page.getByTestId("stage3d-mode-live")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("stage3d-mode-preview")).toBeEnabled();
  await expect(page.getByTestId("stage3d-no-preview")).toHaveCount(0);
  await expect(page.getByTestId("stage3d-unplaced")).toHaveCount(0);
  expect(reads()).toBe(0);
});

test("selection is shared between the plot, 3D and the inspector", async ({
  page,
}) => {
  await withCurrentVenue(page, "#/lighting/venues/built-in");
  await expect(plot(page)).toHaveAttribute("data-positions", /house-par/);
  // Selected on the plot...
  const at = JSON.parse((await plot(page).getAttribute("data-positions"))!)[
    "house-par"
  ];
  await plot(page).click({ position: { x: at.x, y: at.y } });
  await expect(
    page.locator(".inspector").getByLabel("house-par", { exact: true }),
  ).toBeChecked();
  // ...it is marked in 3D.
  await threeButton(page).click();
  await expect(viewport(page)).toHaveAttribute("data-selected", "house-par");
  if (!(await rendered(page))) return;

  // A click on empty space clears it, as on the plot.
  const box = (await viewport(page).boundingBox())!;
  await page.mouse.click(box.x + 8, box.y + box.height / 3);
  await expect(viewport(page)).toHaveAttribute("data-selected", "");
  await expect(
    page.locator(".inspector").getByLabel("house-par", { exact: true }),
  ).not.toBeChecked();

  // A click on the fixture selects it; a drag from it orbits and does not.
  const p = await onScreen(page, "house-par");
  await page.mouse.click(p.x, p.y);
  await expect(viewport(page)).toHaveAttribute("data-selected", "house-par");
  await expect(
    page.locator(".inspector").getByLabel("house-par", { exact: true }),
  ).toBeChecked();
  await page.mouse.click(box.x + 8, box.y + box.height / 3);
  await expect(viewport(page)).toHaveAttribute("data-selected", "");
  const q = await onScreen(page, "house-par");
  await page.mouse.move(q.x, q.y);
  await page.mouse.down();
  await page.mouse.move(q.x + 60, q.y + 10, { steps: 5 });
  await page.mouse.up();
  await expect(viewport(page)).toHaveAttribute("data-selected", "");

  // Selected in 3D, it is selected on the plot too (once the orbit's
  // damping has let the camera settle, so the fixture is where it is said).
  await page.waitForTimeout(1500);
  const r = await onScreen(page, "house-par");
  await page.mouse.click(r.x, r.y);
  await expect(viewport(page)).toHaveAttribute("data-selected", "house-par");
  await plotButton(page).click();
  await expect(
    page.locator(".inspector").getByLabel("house-par", { exact: true }),
  ).toBeChecked();
});

test("an inspector edit is redrawn in 3D without moving the camera", async ({
  page,
}) => {
  // A venue only this test saves (the mock keeps saves by name).
  const name = `rot-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  await withCurrentVenue(page, `#/lighting/venues/${name}?view=3d`);
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "1");
  if (!(await rendered(page))) return;
  await expect(viewport(page)).toHaveAttribute(
    "data-transforms",
    /"front-left"/,
  );
  const before = JSON.parse(
    (await viewport(page).getAttribute("data-transforms"))!,
  )["front-left"];
  expect(before.rotation).toEqual([0, 0, 0]);

  // Orbit somewhere, so the camera is the user's own.
  const box = (await viewport(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2, {
    steps: 6,
  });
  await page.mouse.up();
  await page.waitForTimeout(600);
  const camera = JSON.parse((await viewport(page).getAttribute("data-view"))!)
    .camera as number[];

  await page
    .locator(".inspector")
    .getByLabel("front-left", { exact: true })
    .check();
  await page.locator("#insp-direction").selectOption("upstage");
  await page.locator("#insp-tilt").fill("30");
  await page.getByRole("button", { name: "Face this way" }).click();

  await expect
    .poll(async () => {
      const t = JSON.parse(
        (await viewport(page).getAttribute("data-transforms"))!,
      )["front-left"];
      return JSON.stringify(t.rotation);
    })
    .not.toBe(JSON.stringify(before.rotation));
  await page.waitForTimeout(600);
  const after = JSON.parse((await viewport(page).getAttribute("data-view"))!)
    .camera as number[];
  // Damping may settle a hair; the view did not jump back to its framing.
  after.forEach((v, i) => expect(Math.abs(v - camera[i])).toBeLessThan(0.05));
  await expect(viewport(page)).toHaveAttribute("data-selected", "front-left");
});

test("a saved venue or fixture type re-reads the file view", async ({
  page,
}) => {
  const { reads } = await withCurrentVenue(
    page,
    "#/lighting/venues/club?view=3d",
  );
  await expect(viewport(page)).toHaveAttribute("data-fixtures", "3");
  const before = reads();
  await page.evaluate(async () => {
    const m = await import(/* @vite-ignore */ "/src/lib/lighting/changes.ts");
    m.venueChanged("club");
  });
  await expect.poll(reads).toBe(before + 1);
  await page.evaluate(async () => {
    const m = await import(/* @vite-ignore */ "/src/lib/lighting/changes.ts");
    m.venueChanged("elsewhere");
    m.fixtureTypesChanged();
  });
  await expect.poll(reads).toBe(before + 2);
});

test("without WebGL the card says so, and Plot still returns", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    // @ts-expect-error -- narrowing the overloads is beside the point here.
    HTMLCanvasElement.prototype.getContext = function (kind, ...rest) {
      if (String(kind).startsWith("webgl")) return null;
      // @ts-expect-error -- as above.
      return original.call(this, kind, ...rest);
    };
  });
  await withCurrentVenue(page, "#/lighting/venues/built-in?view=3d");
  await expect(viewport(page)).toHaveAttribute("data-renderer", "none", {
    timeout: 15000,
  });
  await expect(page.getByTestId("stage3d-nowebgl")).toContainText("WebGL");
  await plotButton(page).click();
  await expect(plot(page)).toBeVisible();
  await expect(plot(page)).toHaveAttribute("data-positions", /house-par/);
});
