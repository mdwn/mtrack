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
import { FIT } from "../mock-server/test-data";

// Every test routes its own fit facts, venue file and profile with
// `page.route`, so no test depends on state another one left in the shared
// mock server.

type FitFacts = typeof FIT;
type VenueBody = {
  fixtures: {
    name: string;
    fixture_type: string;
    universe: number;
    start_channel: number;
    tags: string[];
    position: number[];
    rotation: number[];
  }[];
  focus_points: Record<string, number[]>;
  source: { mvr: string; origin: number[] };
};

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Serves a copy of the fit facts, changed by `change`. */
async function routeFit(page: Page, change: (f: FitFacts) => void = () => {}) {
  const facts = structuredClone(FIT);
  change(facts);
  await page.route("**/api/lighting/fit", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(facts),
    });
  });
  return facts;
}

/** The venue file the page reads and writes: positions, rotations, a focus
 *  point and an MVR provenance, all of which a tag save must leave alone.
 *  Saves are collected in `puts`. */
async function routeVenue(page: Page) {
  const original: VenueBody = {
    fixtures: FIT.venue.fixtures.map((f, i) => ({
      name: f.name,
      fixture_type: f.type,
      universe: 1,
      start_channel: 1 + i * 20,
      tags: [...f.tags],
      position: [...f.position],
      rotation: [0, 15, 90],
    })),
    focus_points: { center: [0, 2, 0] },
    source: { mvr: "imports/rig.mvr", origin: [1, 2, 3] },
  };
  const puts: VenueBody[] = [];
  await page.route(
    /\/api\/lighting\/venues\/fit-venue(\?.*)?$/,
    async (route) => {
      const request = route.request();
      if (request.method() === "PUT") {
        puts.push(request.postDataJSON());
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
        return;
      }
      const fixtures = Object.fromEntries(
        original.fixtures.map((f) => [f.name, f]),
      );
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          venue: {
            name: "fit-venue",
            fixtures,
            focus_points: original.focus_points,
            source: original.source,
          },
          dsl: "",
        }),
      });
    },
  );
  return { original, puts };
}

/** Metadata that gives the plan the fit venue's geometry, so the page's
 *  stage card draws the fixtures and can save into `fit-venue`. */
function planMetadata() {
  return {
    type: "metadata",
    fixtures: Object.fromEntries(
      FIT.venue.fixtures.map((f) => [
        f.name,
        {
          tags: f.tags,
          type: f.type,
          position: f.position,
          rotation: [0, 0, 0],
        },
      ]),
    ),
    venue: {
      name: "fit-venue",
      dir: null,
      focus_points: { center: [0, 2, 0] },
    },
  };
}

/** Opens the page with the plan drawn, and returns the ws id. */
async function openWithPlan(
  page: Page,
  hash = "#/lighting/fit",
): Promise<string> {
  const wsId = `fit-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
  await page.goto(`/?wsId=${wsId}${hash}`);
  await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();
  await sendWsMessage(page, wsId, planMetadata());
  await expect
    .poll(async () =>
      page
        .locator(".stage-card__viewport canvas")
        .getAttribute("data-positions"),
    )
    .toContain("viper1");
  return wsId;
}

/** Where a fixture is drawn, in the canvas's own pixels. */
async function fixtureAt(page: Page, name: string) {
  const raw = await page
    .locator(".stage-card__viewport canvas")
    .getAttribute("data-positions");
  return JSON.parse(raw!)[name] as { x: number; y: number };
}

async function clickFixture(page: Page, name: string) {
  const at = await fixtureAt(page, name);
  await page
    .locator(".stage-card__viewport canvas")
    .click({ position: { x: at.x, y: at.y } });
}

const PROFILE_YAML = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes:
        - universe: 1
          name: u1
`;

test.describe("Lighting: Fit shows", () => {
  test("lists groups with empty ones first and selects the first", async ({
    page,
  }) => {
    await routeFit(page);
    await page.goto("/#/lighting/fit");

    const names = page.getByTestId("fit-groups").locator(".group__name");
    await expect(names).toHaveText(["movers", "strobes", "back_wash"]);
    // State is text, and the first empty group is selected.
    await expect(page.getByTestId("fit-group-movers")).toContainText(
      "finds no fixtures",
    );
    await expect(page.getByTestId("fit-group-back_wash")).toContainText(
      "finds 1 fixture",
    );
    await expect(page.getByTestId("fit-group-movers")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("fit-footer")).toHaveText(
      "1 of 3 groups find fixtures",
    );
    await expect(page.getByTestId("fit-group-fix")).toContainText("movers");

    // Selecting another group drives the third column.
    await page.getByTestId("fit-group-back_wash").click();
    await expect(page.getByTestId("fit-group-back_wash")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("fit-group-movers")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    await expect(page.getByTestId("fit-finds")).toContainText(
      "back_wash already finds 1 fixture: par1.",
    );
    await expect(page.getByTestId("fit-suggestion")).toHaveCount(0);
  });

  test("a ?group= link selects that group", async ({ page }) => {
    await routeFit(page);
    await page.goto("/#/lighting/fit?group=strobes");
    await expect(page.getByTestId("fit-group-strobes")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  test("Apply writes the suggestion's tags and keeps everything else in the venue", async ({
    page,
  }) => {
    await routeFit(page);
    const { original, puts } = await routeVenue(page);
    await page.goto("/#/lighting/fit");

    const suggestion = page.getByTestId("fit-suggestion");
    await expect(suggestion).toContainText(
      "The 3 Viper on the upstage truss can move and mix colour, which movers needs.",
    );
    await expect(suggestion).toContainText(
      "Adds the tag moving_head to: viper1, viper2, viper3.",
    );
    await expect(page.getByTestId("fit-selection-count")).toHaveText(
      "3 fixtures selected",
    );

    await page.getByTestId("fit-apply").click();
    await expect(page.getByTestId("fit-tag-msg")).toHaveText(
      "Tagged 3 fixtures with moving_head.",
    );

    expect(puts).toHaveLength(1);
    const saved = puts[0];
    // The suggested fixtures gained the tag ...
    for (const name of ["viper1", "viper2", "viper3"]) {
      expect(saved.fixtures.find((f) => f.name === name)!.tags).toEqual([
        "moving_head",
      ]);
    }
    // ... nothing else about any fixture moved, ...
    const stripTags = (f: VenueBody["fixtures"][number]) => ({
      ...f,
      tags: undefined,
    });
    expect(saved.fixtures.map(stripTags)).toEqual(
      original.fixtures.map(stripTags),
    );
    expect(saved.fixtures.find((f) => f.name === "esprite1")!.tags).toEqual([
      "spot",
    ]);
    expect(saved.fixtures.find((f) => f.name === "par1")!.tags).toEqual([
      "wash",
    ]);
    // ... and the focus points and the provenance rode along.
    expect(saved.focus_points).toEqual(original.focus_points);
    expect(saved.source).toEqual(original.source);
  });

  test("Pick others switches to another set and tags only that", async ({
    page,
  }) => {
    await routeFit(page);
    const { puts } = await routeVenue(page);
    await page.goto("/#/lighting/fit");

    const pick = page.getByTestId("fit-pick-others");
    await expect(pick).toHaveAttribute("aria-expanded", "false");
    await pick.click();
    await expect(pick).toHaveAttribute("aria-expanded", "true");
    const picker = page.getByTestId("fit-picker");
    await expect(picker.getByTestId("fit-cluster-0")).toHaveText(
      "Use the 3 Viper, upstage truss",
    );
    await expect(picker.getByTestId("fit-cluster-0")).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await picker.getByTestId("fit-cluster-1").click();
    await expect(page.getByTestId("fit-selection-count")).toHaveText(
      "1 fixture selected",
    );
    // Not the suggestion any more, so the button says what it will do.
    await expect(page.getByTestId("fit-apply")).toHaveCount(0);
    await page.getByTestId("fit-tag-selection").click();
    await expect(page.getByTestId("fit-tag-msg")).toHaveText(
      "Tagged 1 fixture with moving_head.",
    );

    expect(puts).toHaveLength(1);
    const tagged = puts[0].fixtures.filter((f) =>
      f.tags.includes("moving_head"),
    );
    expect(tagged.map((f) => f.name)).toEqual(["esprite1"]);
    // The tag joined the one it already had.
    expect(tagged[0].tags).toEqual(["spot", "moving_head"]);
  });

  test("a manual selection: click fixtures on the plan or tick them in the list", async ({
    page,
  }) => {
    await routeFit(page);
    const { puts } = await routeVenue(page);
    await openWithPlan(page);

    await expect(page.getByTestId("fit-selection-count")).toHaveText(
      "3 fixtures selected",
    );
    // A click on the plan adds a fixture to the selection ...
    await clickFixture(page, "par1");
    await expect(page.getByTestId("fit-selection-count")).toHaveText(
      "4 fixtures selected",
    );
    await expect(page.getByTestId("fit-tag-selection")).toHaveText(
      "Tag 4 fixtures",
    );
    // ... and a second click takes it out, which is the suggestion again.
    await clickFixture(page, "par1");
    await expect(page.getByTestId("fit-apply")).toBeVisible();

    // The same selection is reachable without the canvas.
    await page.getByTestId("fit-pick-others").click();
    const viper2 = page.getByTestId("fit-fixture-viper2");
    await expect(viper2).toBeChecked();
    await viper2.uncheck();
    await expect(page.getByTestId("fit-selection-count")).toHaveText(
      "2 fixtures selected",
    );
    await page.getByTestId("fit-fixture-esprite1").check();
    await page.getByTestId("fit-tag-selection").click();
    await expect(page.getByTestId("fit-tag-msg")).toContainText("Tagged 3");

    expect(puts).toHaveLength(1);
    const tagged = puts[0].fixtures
      .filter((f) => f.tags.includes("moving_head"))
      .map((f) => f.name);
    expect(tagged.sort()).toEqual(["esprite1", "viper1", "viper3"]);
  });

  test("a group nothing fits gets no suggestion and says what is missing", async ({
    page,
  }) => {
    await routeFit(page);
    await page.goto("/#/lighting/fit");
    await page.getByTestId("fit-group-strobes").click();

    await expect(page.getByTestId("fit-no-suggestion")).toContainText(
      "Nothing here fits.",
    );
    await expect(page.getByTestId("fit-unmet")).toHaveText(
      "Nothing in this venue can strobe, which strobes needs.",
    );
    await expect(page.getByTestId("fit-suggestion")).toHaveCount(0);
    await expect(page.getByTestId("fit-apply")).toHaveCount(0);
    // A person can still choose by hand.
    await expect(page.getByTestId("fit-pick-others")).toHaveText(
      "Pick fixtures",
    );
  });

  test("wants met only one at a time say so", async ({ page }) => {
    await routeFit(page, (f) => {
      const g = f.groups.find((g) => g.name === "strobes")!;
      g.wants = ["move", "color"];
      g.unmet = ["move", "color"];
      g.unmet_together = true;
    });
    await page.goto("/#/lighting/fit?group=strobes");
    await expect(page.getByTestId("fit-unmet")).toHaveText(
      "No single fixture in this venue can do all of these: move and mix colour. strobes needs them together.",
    );
  });

  test("a group the profile does not define points at Groups", async ({
    page,
  }) => {
    await routeFit(page, (f) => {
      const g = f.groups.find((g) => g.name === "strobes")!;
      g.defined = false;
    });
    await page.goto("/#/lighting/fit?group=strobes");
    await expect(page.getByTestId("fit-undefined")).toContainText(
      "strobes is not defined for this profile",
    );
    await expect(
      page.getByTestId("fit-undefined").getByRole("link", { name: "Groups" }),
    ).toBeVisible();
    await expect(page.getByTestId("fit-apply")).toHaveCount(0);
  });

  test("Place on plan creates the named focus point where the plan is clicked", async ({
    page,
  }) => {
    await routeFit(page);
    const { original, puts } = await routeVenue(page);
    await openWithPlan(page);

    await page.getByTestId("fit-place-drummer").click();
    await expect(page.getByTestId("fit-placing")).toContainText(
      "Click the plan to place drummer",
    );
    // A click on the plot (well inside it, away from the pins).
    const canvas = page.locator(".stage-card__viewport canvas");
    const box = (await canvas.boundingBox())!;
    await canvas.click({
      position: { x: box.width * 0.5, y: box.height * 0.45 },
    });

    await expect(page.getByTestId("fit-focus-msg")).toHaveText(
      "Placed focus point drummer.",
    );
    expect(puts).toHaveLength(1);
    const points = puts[0].focus_points;
    expect(Object.keys(points).sort()).toEqual(["center", "drummer"]);
    expect(points.center).toEqual(original.focus_points.center);
    expect(points.drummer).toHaveLength(3);
    expect(points.drummer.every((n) => Number.isFinite(n))).toBe(true);
    // Placing is over, and the fixtures were not touched.
    await expect(page.getByTestId("fit-placing")).toHaveCount(0);
    expect(puts[0].fixtures).toEqual(original.fixtures);
  });

  test("Place on plan can be cancelled", async ({ page }) => {
    await routeFit(page);
    await page.goto("/#/lighting/fit");
    await page.getByTestId("fit-place-drummer").click();
    await expect(page.getByTestId("fit-placing")).toBeVisible();
    await page.getByRole("button", { name: "Cancel placing" }).click();
    await expect(page.getByTestId("fit-placing")).toHaveCount(0);
    await expect(page.getByTestId("fit-place-drummer")).toBeVisible();
  });

  test("Add to profile appends the universe to the running profile", async ({
    page,
  }) => {
    await routeFit(page);
    await page.route("**/api/config/store", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ yaml: PROFILE_YAML, checksum: "fit" }),
      });
    });
    const writes: {
      expected_checksum: string;
      profile: {
        hostname: string;
        dmx: { universes: { universe: number; name: string }[] };
      };
    }[] = [];
    await page.route("**/api/config/profiles/0", async (route) => {
      writes.push(route.request().postDataJSON());
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ yaml: PROFILE_YAML, checksum: "fit-2" }),
      });
    });
    await page.goto("/#/lighting/fit");

    await expect(page.getByTestId("fit-universe-11")).toContainText(
      "Universe 11 has fixtures, but this profile has no output for it.",
    );
    await page.getByTestId("fit-add-universe-11").click();
    await expect(page.getByTestId("fit-output-msg")).toHaveText(
      "Added universe 11 to test-host.",
    );

    expect(writes).toHaveLength(1);
    expect(writes[0].expected_checksum).toBe("fit");
    expect(writes[0].profile.hostname).toBe("test-host");
    expect(writes[0].profile.dmx.universes).toEqual([
      { universe: 1, name: "u1" },
      { universe: 11, name: "u11" },
    ]);
  });

  test("an unpatched universe gets its ola_patch line with placeholders", async ({
    page,
  }) => {
    await routeFit(page);
    await page.goto("/#/lighting/fit");
    const patch = page.getByTestId("fit-patch-1");
    await expect(patch).toContainText(
      "olad has no output port patched to universe 1",
    );
    await expect(patch.locator("code")).toHaveText(
      "ola_patch -d <device> -p <port> -u 1",
    );
    await expect(patch.getByRole("button", { name: "Copy" })).toBeVisible();
  });

  test("olad not answering is said, not read as patched", async ({ page }) => {
    await routeFit(page, (f) => {
      f.output.reachable = false;
      f.output.unpatched = [];
    });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-olad-unreachable")).toHaveText(
      "olad's web server on port 9090 did not answer, so patching could not be checked.",
    );
  });

  test("a venue that needs nothing says so", async ({ page }) => {
    await routeFit(page, (f) => {
      f.groups = [];
      f.focus_points_wanted = [];
      f.output.unconfigured = [];
      f.output.unpatched = [];
    });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-groups-none")).toBeVisible();
    await expect(page.getByTestId("fit-focus")).toContainText(
      "Every focus point your shows aim at is in the venue.",
    );
    await expect(page.getByTestId("fit-output")).toContainText(
      "Every universe the venue uses has an output",
    );
    await expect(page.getByTestId("fit-footer")).toHaveText(
      "0 of 0 groups find fixtures",
    );
  });

  test("without a current venue there is nothing to fit", async ({ page }) => {
    await routeFit(page, (f) => {
      (f as { venue: unknown }).venue = null;
    });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-no-venue")).toContainText(
      "No venue is current",
    );
  });

  test("a failed read is reported with a way to retry", async ({ page }) => {
    let fail = true;
    await page.route("**/api/lighting/fit", async (route) => {
      if (fail) {
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({ error: "boom" }),
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(FIT),
      });
    });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-error")).toContainText("boom");
    fail = false;
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page.getByTestId("fit-groups")).toBeVisible();
  });

  test("an answer that is not the fit facts is an error with Retry, not a crash", async ({
    page,
  }) => {
    const pageErrors: Error[] = [];
    page.on("pageerror", (e) => pageErrors.push(e));
    let broken = true;
    await page.route("**/api/lighting/fit", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: broken ? "text/html" : "application/json",
        body: broken ? "<html>gateway</html>" : JSON.stringify(FIT),
      });
    });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-error")).toContainText(
      "answer was not understood",
    );
    broken = false;
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page.getByTestId("fit-groups")).toBeVisible();
    expect(pageErrors).toEqual([]);
  });

  test("it works at phone width with the columns stacked", async ({ page }) => {
    await routeFit(page);
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-groups")).toBeVisible();
    await expect(page.getByTestId("fit-suggestion")).toBeVisible();

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    expect(overflow).toBe(false);

    // One column: groups, then the plan, then the fixes, each below the last.
    const y = async (id: string) => (await page.locator(id).boundingBox())!.y;
    const groups = await y("#fit-groups-title");
    const plan = await y("#fit-plan-title");
    const fixes = await y("#fit-fixes-title");
    expect(plan).toBeGreaterThan(groups);
    expect(fixes).toBeGreaterThan(plan);
  });

  test("at desktop width the three columns sit side by side", async ({
    page,
  }) => {
    await routeFit(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/#/lighting/fit");
    await expect(page.getByTestId("fit-groups")).toBeVisible();
    const x = async (id: string) => (await page.locator(id).boundingBox())!.x;
    expect(await x("#fit-plan-title")).toBeGreaterThan(
      await x("#fit-groups-title"),
    );
    expect(await x("#fit-fixes-title")).toBeGreaterThan(
      await x("#fit-plan-title"),
    );
  });
});
