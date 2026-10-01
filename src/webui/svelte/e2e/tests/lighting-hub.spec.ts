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
import { READINESS } from "../mock-server/test-data";

// Every test routes its own readiness answer with `page.route`, so no test
// depends on state another one left in the shared mock server.

type Readiness = typeof READINESS;

let testCounter = 0;

/** A profile named like the one the mock reports as running, so the DMX
 *  settings link can name it. */
const PROFILE_YAML = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes: []
`;

/** Serves a copy of the all-ready facts, changed by `change`. */
async function routeReadiness(
  page: Page,
  change: (r: Readiness) => void = () => {},
) {
  const facts = structuredClone(READINESS);
  change(facts);
  await page.route("**/api/lighting/readiness", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(facts),
    });
  });
  await page.route("**/api/config/store", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml: PROFILE_YAML, checksum: "hub" }),
    });
  });
}

const CHECKS = ["fixtures", "venue", "groups", "shows", "output"] as const;

async function states(page: Page): Promise<string[]> {
  return Promise.all(
    CHECKS.map((c) =>
      page.getByTestId(`check-${c}`).getAttribute("data-state"),
    ),
  ).then((all) => all.map((s) => s ?? ""));
}

test.describe("Lighting hub", () => {
  test("a venue that does not load is said first, and blocks the Venue check", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.venue_error = {
        venue: "test-venue",
        fixture: "Brick3",
        reason: 'mode "13: DIM" of fixture type "brick" did not load',
      };
    });
    await page.goto("/#/lighting");
    const alert = page.getByTestId("hub-venue-error");
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(
      'Venue "test-venue" did not load: fixture "Brick3"',
    );
    await expect(alert).toContainText("no fixtures will light");
    await expect(page.getByTestId("check-venue")).toHaveAttribute(
      "data-state",
      "blocked",
    );
  });

  test("patch overlaps are the venue's, listed once and not under every song", async ({
    page,
  }) => {
    const message =
      'fixtures "A" and "B" are both patched on universe 1 at addresses 2-4; each will overwrite the other';
    await routeReadiness(page, (r) => {
      r.patch_warnings = [{ kind: "patch-overlap", message }];
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-venue")).toHaveAttribute(
      "data-state",
      "attention",
    );
    await expect(page.getByText(message)).toHaveCount(1);
    await expect(page.getByTestId("hub-venue-error")).toHaveCount(0);
  });

  test("a rig where everything lines up is ready in all five checks", async ({
    page,
  }) => {
    await routeReadiness(page);
    await page.goto("/#/lighting");

    const strip = page.getByRole("list", { name: "Readiness checks" });
    await expect(strip.getByRole("listitem")).toHaveCount(5);
    await expect(page.getByTestId("check-fixtures")).toContainText(
      "Fixture types",
    );
    expect(await states(page)).toEqual(Array(5).fill("ready"));
    // State is text, not only colour.
    for (const c of CHECKS) {
      await expect(page.getByTestId(`check-${c}-state`)).toHaveText("Ready");
    }
    await expect(page.getByTestId("check-venue")).toContainText(
      "8 fixtures, all placed",
    );
    await expect(page.getByTestId("check-groups")).toContainText(
      "1 group finds fixtures",
    );
    await expect(page.getByTestId("hub-none")).toHaveText(
      "Nothing needs attention.",
    );
    // The live stage is there, and it is a view, not an editor.
    await expect(page.locator(".stage-card")).toBeVisible();
    await expect(page.locator(".stage-card__add-focus")).toHaveCount(0);
    // One heading for it: the stage card's own, not a wrapper's as well.
    await expect(page.getByText("Live stage")).toHaveCount(0);
    await expect(page.locator(".stage-card")).toHaveCount(1);
  });

  test("a fixture type that did not load blocks, with the reason and a link", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.fixture_types.unresolved = [
        {
          fixture: "Brick9",
          type: "Nope",
          reason: "no fixture type named 'Nope' is defined",
        },
      ];
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-fixtures")).toHaveAttribute(
      "data-state",
      "blocked",
    );
    await expect(page.getByTestId("check-fixtures-state")).toHaveText(
      "Blocked",
    );
    const findings = page.getByTestId("findings-fixtures");
    await expect(findings).toContainText(
      "Brick9 is a Nope, which did not load",
    );
    await expect(findings).toContainText("no fixture type named 'Nope'");
    await expect(
      findings.getByRole("link", { name: "Open Fixture types" }),
    ).toHaveAttribute("href", "#/lighting/fixtures");
  });

  test("no current venue blocks the venue and leaves what depends on it unknown", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.venue = null;
      r.groups = [];
      r.shows = [];
    });
    await page.goto("/#/lighting");
    expect(await states(page)).toEqual([
      "unknown",
      "blocked",
      "unknown",
      "ready",
      "unknown",
    ]);
    await expect(page.getByTestId("check-fixtures")).toContainText(
      "Needs a venue",
    );
    await expect(
      page
        .getByTestId("findings-venue")
        .getByRole("link", { name: "Choose a venue" }),
    ).toHaveAttribute("href", "#/lighting/groups?profile=test-host");
  });

  test("a group that finds no fixtures needs attention and links to Groups", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.groups = [
        { name: "front_wash", fixtures: 0, songs: ["Test Song Alpha"] },
        { name: "back_wash", fixtures: 3, songs: ["Test Song Alpha"] },
      ];
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-groups")).toHaveAttribute(
      "data-state",
      "attention",
    );
    await expect(page.getByTestId("check-groups-state")).toHaveText(
      "Needs attention",
    );
    const findings = page.getByTestId("findings-groups");
    await expect(findings.getByRole("listitem")).toHaveCount(1);
    await expect(findings).toContainText(
      "Group front_wash finds no fixtures in test-venue. Used by: Test Song Alpha.",
    );
    await expect(
      findings.getByRole("link", { name: "Open Fit shows" }),
    ).toHaveAttribute("href", "#/lighting/fit?group=front_wash");
  });

  test("a show that does not load blocks and links to the song's lighting editor", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.shows.push({
        song: "Esaweg",
        files: [],
        error: "Effect 'static' requires a 'duration'",
        warnings: [],
      } as never);
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-shows")).toHaveAttribute(
      "data-state",
      "blocked",
    );
    const findings = page.getByTestId("findings-shows");
    await expect(findings).toContainText("Esaweg does not load");
    await expect(findings).toContainText("requires a 'duration'");
    await expect(
      findings.getByRole("link", { name: "Open the song's lighting" }),
    ).toHaveAttribute("href", "#/songs/Esaweg/lighting");
  });

  test("lint that stops a cue needs attention; advisory lint is only listed", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.shows[0].warnings = [
        {
          kind: "past-end-of-song",
          message: "a cue starts after the song ends",
        },
      ];
    });
    await page.goto("/#/lighting");
    // Advisory kinds do not change the state ...
    await expect(page.getByTestId("check-shows")).toHaveAttribute(
      "data-state",
      "ready",
    );
    // ... but they are listed under the song, marked as notes.
    const findings = page.getByTestId("findings-shows");
    await expect(findings).toContainText("Test Song Alpha");
    await expect(findings).toContainText("Note");
    await expect(findings).toContainText("a cue starts after the song ends");

    // A capability gap does.
    await page.unroute("**/api/lighting/readiness");
    await routeReadiness(page, (r) => {
      r.shows[0].warnings = [
        {
          kind: "capability-gap",
          message: "strobe on fixtures with no strobe",
        },
      ];
    });
    await page.reload();
    await expect(page.getByTestId("check-shows")).toHaveAttribute(
      "data-state",
      "attention",
    );
  });

  test("a universe with no output blocks and links to Fit shows", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.output.universes = [1, 4];
      r.output.unconfigured = [4];
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-output")).toHaveAttribute(
      "data-state",
      "blocked",
    );
    const findings = page.getByTestId("findings-output");
    await expect(findings).toContainText(
      "Universe 4 has fixtures but no output under dmx.universes.",
    );
    await expect(
      findings.getByRole("link", { name: "Open Fit shows" }),
    ).toHaveAttribute("href", "#/lighting/fit");
  });

  test("a universe olad has no port for blocks", async ({ page }) => {
    await routeReadiness(page, (r) => {
      r.output.olad = { reachable: true, unpatched: [1] };
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-output")).toHaveAttribute(
      "data-state",
      "blocked",
    );
    await expect(page.getByTestId("findings-output")).toContainText(
      "olad has no output port patched to universe 1",
    );
  });

  test("olad not answering leaves the output unknown, not ready", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.output.olad = { reachable: false, unpatched: [] };
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("check-output")).toHaveAttribute(
      "data-state",
      "unknown",
    );
    await expect(page.getByTestId("check-output-state")).toHaveText("Unknown");
    await expect(page.getByTestId("findings-output")).toContainText(
      "olad's web server did not answer",
    );
    // The other four are unaffected.
    expect((await states(page)).slice(0, 4)).toEqual(Array(4).fill("ready"));
  });

  test("a profile without DMX says so, but its shows are still checked", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.dmx = false;
      r.venue = null;
      r.fixture_types = { in_use: [], unresolved: [] };
      r.groups = [];
      r.shows = [
        {
          song: "Esaweg",
          files: [],
          error: "Effect 'static' requires a 'duration'",
          warnings: [],
        } as never,
      ];
      r.output = { universes: [], unconfigured: [], olad: null };
    });
    await page.goto("/#/lighting");
    expect(await states(page)).toEqual([
      "unknown",
      "unknown",
      "unknown",
      "blocked",
      "blocked",
    ]);
    for (const c of ["fixtures", "venue", "groups"]) {
      await expect(page.getByTestId(`check-${c}`)).toContainText(
        "No DMX on this profile",
      );
    }
    await expect(page.getByTestId("findings-shows")).toContainText(
      "Esaweg does not load",
    );
    await expect(page.getByTestId("findings-output")).toContainText(
      "This profile has no DMX output",
    );
    await expect(
      page
        .getByTestId("findings-output")
        .getByRole("link", { name: "Open DMX settings" }),
    ).toHaveAttribute("href", "#/config/test-host/lighting");
  });

  test("the shows summary counts what won't load apart from what to check", async ({
    page,
  }) => {
    await routeReadiness(page, (r) => {
      r.shows = [
        {
          song: "Esaweg",
          files: [],
          error: "Effect 'static' requires a 'duration'",
          warnings: [],
        } as never,
        {
          song: "Devoured in Decay",
          files: ["show.light"],
          warnings: [
            {
              kind: "unbound-focus-point",
              message:
                "`move` on drummer aims at a focus point the venue lacks",
            },
          ],
        },
      ];
    });
    await page.goto("/#/lighting");
    const card = page.getByTestId("check-shows");
    await expect(card).toHaveAttribute("data-state", "blocked");
    await expect(card).toContainText("1 song won't load");
    await expect(card).toContainText("1 to check");
    await expect(card).not.toContainText("problems stop it");
    // The lint message is already a sentence: no raw code in front of it.
    const findings = page.getByTestId("findings-shows");
    await expect(findings).toContainText(
      "aims at a focus point the venue lacks",
    );
    await expect(findings).not.toContainText("unbound-focus-point");
    // An unbound focus point is placed on the plan, on Fit shows; the
    // song that fails to load is fixed in the song.
    await expect(
      findings.getByRole("link", { name: "Open Fit shows" }),
    ).toHaveAttribute("href", "#/lighting/fit");
    await expect(
      findings.getByRole("link", { name: "Open the song's lighting" }),
    ).toHaveAttribute("href", "#/songs/Esaweg/lighting");
  });

  test("the hub refreshes when the venue reloads, not on a timer", async ({
    page,
  }) => {
    const wsId = `hub-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
    let requests = 0;
    let broken = false;
    await page.route("**/api/lighting/readiness", async (route) => {
      requests++;
      const facts = structuredClone(READINESS);
      if (broken) facts.groups[0].fixtures = 0;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(facts),
      });
    });
    await page.goto(`/?wsId=${wsId}#/lighting`);
    await expect(page.getByTestId("check-groups")).toHaveAttribute(
      "data-state",
      "ready",
    );
    const before = requests;

    // The server re-broadcasts metadata after a venue or config reload.
    broken = true;
    await page.request.post("http://127.0.0.1:3111/test/send-ws", {
      data: {
        type: "metadata",
        fixtures: {},
        venue: { name: `venue-${wsId}`, dir: null, focus_points: {} },
        _wsId: wsId,
      },
    });
    await expect(page.getByTestId("check-groups")).toHaveAttribute(
      "data-state",
      "attention",
    );
    expect(requests).toBeGreaterThan(before);
  });

  test("it works at phone width with the checks stacked", async ({ page }) => {
    await routeReadiness(page, (r) => {
      r.groups[0].fixtures = 0;
    });
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/lighting");
    for (const c of CHECKS) {
      await expect(page.getByTestId(`check-${c}`)).toBeVisible();
    }
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    expect(overflow).toBe(false);
    // One column: each check sits below the previous one.
    const tops = await Promise.all(
      CHECKS.map(
        async (c) => (await page.getByTestId(`check-${c}`).boundingBox())!.y,
      ),
    );
    for (let i = 1; i < tops.length; i++) {
      expect(tops[i]).toBeGreaterThan(tops[i - 1]);
    }
    await expect(
      page
        .getByTestId("findings-groups")
        .getByRole("link", { name: "Open Fit shows" }),
    ).toBeVisible();
  });

  test("a failed read is reported with a way to retry", async ({ page }) => {
    let fail = true;
    await page.route("**/api/lighting/readiness", async (route) => {
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
        body: JSON.stringify(READINESS),
      });
    });
    await page.goto("/#/lighting");
    await expect(page.getByTestId("hub-error")).toContainText("boom");
    fail = false;
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page.getByTestId("check-venue")).toHaveAttribute(
      "data-state",
      "ready",
    );
    await expect(page.getByTestId("hub-error")).toHaveCount(0);
  });

  for (const [label, body, contentType] of [
    ["an empty object", "{}", "application/json"],
    ["an HTML page", "<html><body>gateway</body></html>", "text/html"],
    ["a JSON array", "[]", "application/json"],
  ] as const) {
    test(`${label} answering 200 is an error with Retry, not a crash`, async ({
      page,
    }) => {
      const pageErrors: Error[] = [];
      page.on("pageerror", (e) => pageErrors.push(e));
      let broken = true;
      await page.route("**/api/lighting/readiness", async (route) => {
        await route.fulfill({
          status: 200,
          contentType: broken ? contentType : "application/json",
          body: broken ? body : JSON.stringify(READINESS),
        });
      });
      await page.goto("/#/lighting");
      await expect(page.getByTestId("hub-error")).toContainText(
        "answer was not understood",
      );
      await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
      broken = false;
      await page.getByRole("button", { name: "Retry" }).click();
      await expect(page.getByTestId("check-venue")).toHaveAttribute(
        "data-state",
        "ready",
      );
      expect(pageErrors).toEqual([]);
    });
  }
});
