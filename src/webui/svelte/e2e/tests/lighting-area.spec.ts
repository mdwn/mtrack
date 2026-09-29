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

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Two profiles, both with lighting, so the Groups picker has a choice. The
 *  mock reports `test-host` as the running profile. Each test routes its own
 *  copy: nothing here is shared between tests. */
const TWO_PROFILES_YAML = `songs: songs
profiles:
  - hostname: other-host
    dmx:
      universes: []
      lighting:
        current_venue: test-venue
  - hostname: test-host
    dmx:
      universes: []
      lighting:
        groups:
          wash:
            name: wash
            constraints: []
`;

async function routeTwoProfiles(page: Page) {
  await page.route("**/api/config/store", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml: TWO_PROFILES_YAML, checksum: "two" }),
    });
  });
}

test.describe("Lighting area: routes and navigation", () => {
  test("the nav has a Lighting item between Playlists and Config", async ({
    page,
  }) => {
    await page.goto("/#/");
    const hrefs = await page
      .locator(".topnav__tab")
      .evaluateAll((els) => els.map((e) => e.getAttribute("href")));
    const lighting = hrefs.indexOf("#/lighting");
    expect(lighting).toBeGreaterThan(-1);
    expect(hrefs.indexOf("#/playlists")).toBe(lighting - 1);
    expect(hrefs.indexOf("#/config")).toBe(lighting + 1);
  });

  test("the nav item opens the overview and is active", async ({ page }) => {
    await page.goto("/#/");
    await page.locator('.topnav__tab[href="#/lighting"]').click();
    await expect(page).toHaveURL(/#\/lighting$/);
    await expect(page.locator(".topnav__tab--active")).toHaveAttribute(
      "href",
      "#/lighting",
    );
    await expect(page.getByTestId("lighting-overview")).toBeVisible();
    await expect(page).toHaveTitle(/^Lighting - mtrack/);
  });

  test("the overview summarises the venue and counts", async ({ page }) => {
    await routeTwoProfiles(page);
    // Run as the profile that selects a venue.
    await page.route("**/api/status", async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      body.hardware.profile = "other-host";
      await route.fulfill({ response: res, json: body });
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("overview-venue")).toHaveText("test-venue");
    await expect(page.getByTestId("overview-types")).toHaveText("3");
    await expect(page.getByTestId("overview-venues")).not.toHaveText("-");
  });

  for (const [hash, tab, title] of [
    ["/#/lighting/fixtures", "Fixture types", /Lighting - Fixture types/],
    ["/#/lighting/venues", "Venues", /Lighting - Venues/],
    ["/#/lighting/groups", "Groups", /Lighting - Groups/],
    ["/#/lighting/stage", "3D", /Lighting - 3D/],
  ] as const) {
    test(`${hash} renders with its tab current and its title`, async ({
      page,
    }) => {
      await routeTwoProfiles(page);
      await page.goto(hash);
      const nav = page.getByRole("navigation", { name: "Lighting sections" });
      await expect(nav.locator('[aria-current="page"]')).toHaveText(tab);
      await expect(page).toHaveTitle(title);
    });
  }

  test("the sub-navigation is made of links a keyboard can reach", async ({
    page,
  }) => {
    await page.goto("/#/lighting");
    const nav = page.getByRole("navigation", { name: "Lighting sections" });
    await expect(nav.getByRole("link")).toHaveCount(5);
    await nav.getByRole("link", { name: "Venues" }).focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
  });

  test("#/stage redirects to #/lighting/stage and Back skips it", async ({
    page,
  }) => {
    await page.goto("/#/");
    await page.evaluate(() => {
      window.location.hash = "#/stage";
    });
    await expect(page).toHaveURL(/#\/lighting\/stage$/);
    await expect(page.locator(".stage3d .page__title")).toHaveText("Stage 3D");
    await page.goBack();
    await expect(page).toHaveURL(/#\/$/);
  });

  test("a bookmarked #/stage loads Stage 3D", async ({ page }) => {
    await page.goto("/#/stage");
    await expect(page).toHaveURL(/#\/lighting\/stage$/);
    await expect(page.locator(".stage3d .page__title")).toHaveText("Stage 3D");
  });

  test("the area works at phone width", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("lighting-overview")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    expect(overflow).toBe(false);
    await page.locator(".topnav__hamburger").click();
    await expect(page.locator('.drawer a[href="#/lighting"]')).toBeVisible();
  });
});

test.describe("Lighting area: Groups", () => {
  test("the picker defaults to the running profile", async ({ page }) => {
    await routeTwoProfiles(page);
    await page.goto("/#/lighting/groups");
    const picker = page.locator("#groups-profile");
    await expect(picker).toHaveValue("test-host");
    // Its current venue and group are what the panel shows.
    await expect(page.locator(".group-card")).toHaveCount(1);
    await expect(page.locator("#lighting-venue")).toHaveValue("");
  });

  test("?profile= picks the profile", async ({ page }) => {
    await routeTwoProfiles(page);
    await page.goto("/#/lighting/groups?profile=other-host");
    await expect(page.locator("#groups-profile")).toHaveValue("other-host");
    await expect(page.locator("#lighting-venue")).toHaveValue("test-venue");
    await expect(page.locator(".group-card")).toHaveCount(0);
  });

  test("choosing another profile updates the URL and the panel", async ({
    page,
  }) => {
    await routeTwoProfiles(page);
    await page.goto("/#/lighting/groups");
    await page.locator("#groups-profile").selectOption("other-host");
    await expect(page).toHaveURL(/#\/lighting\/groups\?profile=other-host$/);
    await expect(page.locator("#lighting-venue")).toHaveValue("test-venue");
    await expect(page.locator(".group-card")).toHaveCount(0);
  });

  test("saving writes the selected inline profile by index", async ({
    page,
  }) => {
    await routeTwoProfiles(page);
    let put: {
      url: string;
      body: {
        expected_checksum: string;
        profile: { dmx: { lighting: { current_venue: string } } };
      };
    } | null = null;
    await page.route("**/api/config/profiles/*", async (route) => {
      put = {
        url: route.request().url(),
        body: route.request().postDataJSON(),
      };
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ yaml: TWO_PROFILES_YAML, checksum: "after" }),
      });
    });
    await page.goto("/#/lighting/groups");
    await expect(page.locator(".group-card")).toHaveCount(1);
    const save = page.getByRole("button", { name: "Save", exact: true });
    await expect(save).toBeDisabled();
    await page.locator("#lighting-venue").selectOption("test-venue");
    await expect(save).toBeEnabled();
    await save.click();
    await expect(page.getByText("Saved", { exact: true })).toBeVisible();
    expect(put!.url).toMatch(/\/config\/profiles\/1$/);
    expect(put!.body.expected_checksum).toBe("two");
    expect(put!.body.profile.dmx.lighting.current_venue).toBe("test-venue");
  });

  test("unsaved edits are guarded when leaving the page", async ({ page }) => {
    await routeTwoProfiles(page);
    await page.goto("/#/lighting/groups");
    await expect(page.locator(".group-card")).toHaveCount(1);
    await page.locator("#lighting-venue").selectOption("test-venue");
    await page.locator('.topnav__tab[href="#/songs"]').click();
    await expect(page.getByText("Discard unsaved changes?")).toBeVisible();
  });

  test("a profile without lighting points to Config", async ({ page }) => {
    // The default mock profile (test-host) has no `dmx` block.
    await page.goto("/#/lighting/groups");
    await expect(page.getByTestId("groups-no-dmx")).toBeVisible();
    await expect(
      page.getByRole("link", { name: "Enable lighting in Config" }),
    ).toHaveAttribute("href", "#/config/test-host/lighting");
  });
});

test.describe("Lighting area: Config and the dashboard link across", () => {
  test("Config's profile editor shows a summary and a link, not an editor", async ({
    page,
  }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await page.getByRole("button", { name: "Enable Lighting" }).click();

    const summary = page.getByTestId("lighting-summary");
    await expect(summary).toBeVisible();
    await expect(
      summary.getByRole("link", { name: "Edit in Lighting" }),
    ).toHaveAttribute("href", "#/lighting/groups?profile=test-host");
    // DMX hardware stays; the lighting editor is gone.
    await expect(page.locator("#dmx-ola-port")).toBeVisible();
    await expect(page.locator(".sub-tab")).toHaveCount(0);
    await expect(page.locator("#lighting-venue")).toHaveCount(0);
  });

  test("the dashboard's stage card is a live view that links to Venues", async ({
    page,
  }) => {
    const wsId = `lighting-dash-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
    await sendWsMessage(page, wsId, {
      type: "metadata",
      fixtures: {
        "front-left": {
          tags: ["front"],
          type: "par",
          position: [-2, 0.5, 3],
          rotation: [0, 0, 0],
        },
      },
      venue: {
        name: `venue-${wsId}`,
        dir: null,
        focus_points: { drummer: [0, 2.8, 1.4] },
      },
    });
    await expect(page.locator(".stage-card--geometry")).toBeVisible();
    await expect(page.locator(".stage-card__add-focus")).toHaveCount(0);
    await expect(page.locator(".stage-card__focus")).toHaveCount(0);
    await expect(page.locator(".stage-card__viewport canvas")).toBeVisible();

    await page.locator(".stage-card__edit").click();
    await expect(page).toHaveURL(/#\/lighting\/venues$/);
    // There, the same plot is editable.
    await expect(page.locator(".stage-card__add-focus")).toBeVisible();
  });
});

test.describe("Lighting area: Venues stage plot", () => {
  const GEOMETRY = (name: string) => ({
    type: "metadata",
    fixtures: {
      "front-left": {
        tags: ["front"],
        type: "par",
        position: [-2, 0.5, 3],
        rotation: [0, 0, 0],
      },
      spare: { tags: [], type: "par", position: null },
    },
    venue: { name, dir: null, focus_points: { drummer: [0, 2.8, 1.4] } },
  });

  test("with no current venue it says so instead of drawing a stage", async ({
    page,
  }) => {
    await page.goto("/#/lighting/venues");
    await expect(page.getByTestId("stage-no-venue")).toBeVisible();
    await expect(page.locator(".stage-card__viewport")).toHaveCount(0);
  });

  for (const [label, size] of [
    ["desktop", { width: 1440, height: 900 }],
    ["phone", { width: 375, height: 667 }],
  ] as const) {
    test(`the whole plot fits inside its card at ${label} width`, async ({
      page,
    }) => {
      const wsId = `lighting-fit-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
      await page.setViewportSize(size);
      await page.goto(`/?wsId=${wsId}#/lighting/venues`);
      const name = `venue-${wsId}`;
      await sendWsMessage(page, wsId, GEOMETRY(name));
      await expect(page.locator(".stage-card--geometry")).toBeVisible();
      await expect(page.getByTestId("stage-venue-label")).toHaveText(
        `Current venue: ${name} · live`,
      );
      const box = async (sel: string) => {
        const b = await page.locator(sel).boundingBox();
        expect(b, sel).not.toBeNull();
        return b!;
      };
      const card = await box(".stage-card");
      const viewport = await box(".stage-card__viewport");
      const canvas = await box(".stage-card__viewport canvas");
      for (const inner of [viewport, canvas]) {
        expect(inner.x).toBeGreaterThanOrEqual(card.x - 0.5);
        expect(inner.y).toBeGreaterThanOrEqual(card.y - 0.5);
        expect(inner.x + inner.width).toBeLessThanOrEqual(
          card.x + card.width + 0.5,
        );
        expect(inner.y + inner.height).toBeLessThanOrEqual(
          card.y + card.height + 0.5,
        );
      }
      // The canvas is drawn at the size it is shown at.
      const drawn = await page
        .locator(".stage-card__viewport canvas")
        .evaluate((c: HTMLCanvasElement) => [c.clientWidth, c.clientHeight]);
      expect(drawn[0]).toBeCloseTo(canvas.width, 0);
      expect(drawn[1]).toBeCloseTo(canvas.height, 0);
    });
  }
});

test.describe("Lighting area: headings", () => {
  test("3D has one h1 and its own title is an h2", async ({ page }) => {
    await page.goto("/#/lighting/stage");
    await expect(page.getByRole("heading", { level: 1 })).toHaveCount(1);
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(
      "Lighting",
    );
    await expect(
      page.getByRole("heading", { level: 2, name: "Stage 3D" }),
    ).toBeVisible();
  });
});
