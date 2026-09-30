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
import { SONGS } from "../mock-server/test-data";

// Stage 3D's Preview mode (lighting UI design, section 12.1). Every test
// routes its own songs and evaluations with `page.route`. The scene's test
// hook is the viewport's `data-fed` attribute: in Preview it carries exactly
// what the scene was handed (channels, poses, cells).

let testCounter = 0;

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

const SONG = "Test Song Alpha";

/** The one lighting song, with the beat grid and sections the strip draws. */
function songsWithSections() {
  const beta = SONGS.songs.find((s) => s.name === "Test Song Beta")!;
  return {
    songs: [
      {
        ...SONGS.songs[0],
        beat_grid: beta.beat_grid,
        sections: beta.sections,
      },
      SONGS.songs[1],
    ],
    failures: [],
  };
}

async function routeSongs(page: Page, body: object = songsWithSections()) {
  await page.route("**/api/songs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
}

interface Asked {
  song: string;
  times: number[];
}

/** Answers every evaluation with `answer(time)`, remembering the asks. */
async function routeEvaluate(
  page: Page,
  answer: (time: number) => object = () => ({}),
  untouched: string[] = [],
  status = 200,
) {
  const asked: Asked[] = [];
  await page.route("**/api/lighting/evaluate", async (route) => {
    const body = route.request().postDataJSON() as Asked;
    asked.push(body);
    if (status !== 200) {
      await route.fulfill({
        status,
        contentType: "application/json",
        body: JSON.stringify({ error: "Failed to parse show.light: line 3" }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        song: body.song,
        evaluations: body.times.map((time) => ({
          time,
          fixtures: {},
          poses: {},
          cells: {},
          active_effects: [],
          ...answer(time),
        })),
        untouched,
      }),
    });
  });
  return asked;
}

/** A show that lights the mover red and swings it towards the drummer. */
const MOVING = (time: number) => ({
  fixtures: { "mover-1": { red: time > 5 ? 255 : 0, dimmer: 255 } },
  poses: {
    "mover-1": {
      pan: time,
      tilt: 10,
      aim: [0, 0, -1],
      floor: [time / 10, 0],
    },
  },
  cells: {},
  active_effects: [
    {
      id: "a",
      groups: ["washes"],
      kind: "Static",
      layer: "background",
      elapsed: 3,
      duration: 10,
      fixtures: ["mover-1"],
    },
    {
      id: "b",
      groups: ["movers"],
      kind: "Move",
      layer: "foreground",
      elapsed: 2,
      duration: 4,
      fixtures: ["mover-1"],
    },
  ],
});

const METADATA = (capabilities: string[]) => ({
  type: "metadata",
  fixtures: {
    "mover-1": {
      tags: [],
      type: "Mover",
      position: [0, 3.5, 4.2],
      rotation: [0, 0, 180],
      rig: null,
      capabilities,
    },
  },
  venue: { name: "test-venue", dir: null, focus_points: {} },
});

const previewUrl = (wsId: string, query = "") =>
  `/?wsId=${wsId}#/lighting/stage${query}`;

const fed = async (page: Page) =>
  JSON.parse(
    (await page.locator(".stage3d__viewport").getAttribute("data-fed")) ??
      "null",
  );

test.describe("Stage 3D Preview", () => {
  let wsId: string;

  test.beforeEach(() => {
    wsId = `prev-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
  });

  test("the header switches between Live and Preview and says which", async ({
    page,
  }) => {
    await routeSongs(page);
    const asked = await routeEvaluate(page);
    await page.goto(previewUrl(wsId));

    const badge = page.getByTestId("stage3d-mode");
    await expect(badge).toHaveText("Live");
    await expect(page.getByTestId("preview-panel")).toHaveCount(0);
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-source",
      "live",
    );

    await page.getByRole("button", { name: "Preview" }).click();
    await expect(badge).toHaveText("Preview");
    await expect(page.getByRole("button", { name: "Preview" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("preview-panel")).toBeVisible();
    await expect(page.locator(".stage3d__viewport")).toHaveAttribute(
      "data-source",
      "preview",
    );
    // Opening evaluates the first moment of the only lighting song.
    await expect.poll(() => asked.length).toBeGreaterThan(0);
    expect(asked[0]).toEqual({ song: SONG, times: [0] });

    await page.getByRole("button", { name: "Live" }).click();
    await expect(badge).toHaveText("Live");
    await expect(page.getByTestId("preview-panel")).toHaveCount(0);
  });

  test("scrubbing evaluates that moment and feeds the scene", async ({
    page,
  }) => {
    await routeSongs(page);
    const asked = await routeEvaluate(page, MOVING);
    await page.goto(previewUrl(wsId, "?mode=preview"));

    const range = page.getByRole("slider", { name: "Moment in the song" });
    await expect(range).toBeVisible();
    await expect(page.getByTestId("preview-clock")).toHaveText("0:00.0 / 3:00");

    await range.fill("12.5");
    await expect(page.getByTestId("preview-clock")).toHaveText("0:12.5 / 3:00");
    await expect.poll(() => asked.some((a) => a.times[0] === 12.5)).toBe(true);
    // The scene got the evaluation's channels and poses, as the live state
    // message would carry them.
    await expect
      .poll(async () => (await fed(page)).fixtures["mover-1"]?.red)
      .toBe(255);
    const scene = await fed(page);
    expect(scene.poses["mover-1"].pan).toBe(12.5);
    expect(scene.poses["mover-1"].floor).toEqual([1.25, 0]);

    // Scrubbing rests before it asks: a burst is one request, not one each.
    const before = asked.length;
    await range.fill("20");
    await range.fill("21");
    await range.fill("22");
    await expect.poll(() => asked.some((a) => a.times[0] === 22)).toBe(true);
    expect(asked.length - before).toBeLessThan(3);
  });

  test("the scrubber steps from the keyboard", async ({ page }) => {
    await routeSongs(page);
    const asked = await routeEvaluate(page, MOVING);
    await page.goto(previewUrl(wsId, "?mode=preview&t=10"));
    const range = page.getByRole("slider", { name: "Moment in the song" });
    await expect(page.getByTestId("preview-clock")).toHaveText("0:10.0 / 3:00");
    await range.focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByTestId("preview-clock")).toHaveText("0:10.1 / 3:00");
    await expect
      .poll(() => asked.some((a) => Math.abs(a.times[0] - 10.1) < 1e-6))
      .toBe(true);
    await page.keyboard.press("Home");
    await expect(page.getByTestId("preview-clock")).toHaveText("0:00.0 / 3:00");
  });

  test("the song's sections are drawn as a strip", async ({ page }) => {
    await routeSongs(page);
    await routeEvaluate(page);
    await page.goto(previewUrl(wsId, "?mode=preview"));
    const strip = page.getByTestId("preview-sections");
    await expect(strip.locator(".preview__section")).toHaveText([
      "verse",
      "chorus",
    ]);
    await expect(strip).toHaveAttribute("aria-label", /verse, chorus/);
    // 120 BPM in 4/4: measure 5 starts 8 s into a 180 s song.
    const chorus = strip.locator(".preview__section").nth(1);
    await expect(chorus).toHaveAttribute("style", /left: 4\.44/);
  });

  test("at this moment says what each group is doing, in words", async ({
    page,
  }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING);
    await page.goto(previewUrl(wsId, "?mode=preview&t=5"));
    const now = page.getByTestId("preview-activity");
    // Groups in name order, each effect by kind and progress.
    await expect(now.locator("li")).toHaveCount(2);
    await expect(now.locator("li").nth(0)).toContainText(
      "movers: move, 50% done (2.0 s of 4.0 s)",
    );
    await expect(now.locator("li").nth(1)).toContainText(
      "washes: static colour, 30% done (3.0 s of 10.0 s)",
    );
  });

  test("an idle moment says nothing is running", async ({ page }) => {
    await routeSongs(page);
    await routeEvaluate(page);
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-idle")).toContainText(
      "No effect is running",
    );
  });

  test("untouched fixtures are counted, and named on expanding", async ({
    page,
  }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING, ["spare", "wing-left"]);
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-untouched-count")).toContainText(
      "2 fixtures no cue targets",
    );
    await expect(page.getByTestId("preview-untouched-names")).toBeHidden();
    await page.getByTestId("preview-untouched-count").click();
    await expect(page.getByTestId("preview-untouched-names")).toHaveText(
      "spare, wing-left",
    );
  });

  test("a show that reaches every fixture says so", async ({ page }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING, []);
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-untouched-count")).toContainText(
      "Every fixture",
    );
  });

  test("Open this cue in the timeline carries the time", async ({ page }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING);
    await page.goto(previewUrl(wsId, "?mode=preview&t=12.5"));
    const link = page.getByTestId("preview-open-timeline");
    await expect(link).toHaveAttribute(
      "href",
      "#/songs/Test%20Song%20Alpha/lighting?t=12.5",
    );
    await link.click();
    await expect(page).toHaveURL(
      /#\/songs\/Test%20Song%20Alpha\/lighting\?t=12\.5$/,
    );
    // The song's lighting tab opens: the query does not hide the tab.
    await expect(page.locator("#song-tab-lighting")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("the song's lighting editor opens with its cursor at ?t=", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting?t=12.5");
    await expect(page.locator("#song-tab-lighting")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.locator(".play-cursor-time")).toContainText("12");
    // A bad time is ignored rather than breaking the address.
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting?t=nope");
    await page.reload();
    await expect(page.locator("#song-tab-lighting")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.locator(".play-cursor-time")).toHaveCount(0);
  });

  test("a show that does not load says why and draws nothing", async ({
    page,
  }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING, [], 400);
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-error")).toContainText(
      "Failed to parse show.light: line 3",
    );
    await expect(page.getByTestId("preview-activity")).toHaveCount(0);
  });

  test("an evaluation that is not an evaluation says so and draws nothing", async ({
    page,
  }) => {
    const pageErrors: Error[] = [];
    page.on("pageerror", (e) => pageErrors.push(e));
    await routeSongs(page);
    await page.route("**/api/lighting/evaluate", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: "{}",
      });
    });
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-error")).toContainText(
      "answer was not understood",
    );
    await expect(page.getByTestId("preview-activity")).toHaveCount(0);
    expect(pageErrors).toEqual([]);
  });

  test("with no lighting song there is nothing to preview", async ({
    page,
  }) => {
    await routeSongs(page, { songs: [SONGS.songs[1]], failures: [] });
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await expect(page.getByTestId("preview-no-songs")).toBeVisible();
  });

  test("a playing song keeps playing while the preview is scrubbed", async ({
    page,
  }) => {
    await routeSongs(page);
    await routeEvaluate(page, MOVING);
    let transport = 0;
    // The preview must never call the transport.
    await page.route("**/api/player/**", async (route) => {
      transport++;
      await route.continue();
    });
    await page.goto(previewUrl(wsId, "?mode=preview"));
    await page.getByRole("slider", { name: "Moment in the song" }).fill("30");
    await expect(page.getByTestId("preview-activity")).toBeVisible();
    expect(transport).toBe(0);
  });

  test.describe("caveats", () => {
    test("a colour wheel is named only when a fixture has one", async ({
      page,
    }) => {
      await routeSongs(page);
      await routeEvaluate(page, MOVING);
      await page.goto(previewUrl(wsId, "?mode=preview"));
      await sendWsMessage(page, wsId, METADATA(["pan_tilt", "dimmer"]));
      await expect(page.getByTestId("stage3d-mode")).toHaveText("Preview");
      await expect(page.getByTestId("caveat-wheel")).toHaveCount(0);

      await sendWsMessage(
        page,
        wsId,
        METADATA(["pan_tilt", "dimmer", "color_wheel"]),
      );
      await expect(page.getByTestId("caveat-wheel")).toHaveText(
        "1 fixture uses a colour wheel — shown white",
      );

      // A fixture that mixes colour has no wheel to worry about.
      await sendWsMessage(
        page,
        wsId,
        METADATA(["color", "color_wheel", "pan_tilt"]),
      );
      await expect(page.getByTestId("caveat-wheel")).toHaveCount(0);
    });

    test("beams that miss the deck are named with the length they are drawn", async ({
      page,
    }) => {
      await routeSongs(page);
      const asked = await routeEvaluate(page, (time) => ({
        poses: {
          "mover-1": {
            pan: 0,
            tilt: 0,
            aim: [0, 0, time > 10 ? 1 : -1],
            floor: time > 10 ? null : [0, 0],
          },
        },
      }));
      await page.goto(previewUrl(wsId, "?mode=preview"));
      await expect.poll(() => asked.length).toBeGreaterThan(0);
      await expect(page.getByTestId("caveat-beam")).toHaveCount(0);

      await page.getByRole("slider", { name: "Moment in the song" }).fill("20");
      await expect(page.getByTestId("caveat-beam")).toHaveText(
        "1 beam misses the deck — drawn 4 m long",
      );
    });

    test("'no state yet' appears in Live until the engine reports", async ({
      page,
    }) => {
      await routeSongs(page);
      await page.goto(previewUrl(wsId));
      await expect(page.getByTestId("caveat-nostate")).toBeVisible();
      await sendWsMessage(page, wsId, {
        type: "state",
        fixtures: { "mover-1": { red: 255 } },
        poses: {},
        cells: {},
        active_effects: [],
      });
      await expect(page.getByTestId("caveat-nostate")).toHaveCount(0);
    });

    test("'no state yet' is a Live caveat only", async ({ page }) => {
      await routeSongs(page);
      await routeEvaluate(page);
      await page.goto(previewUrl(wsId, "?mode=preview"));
      await expect(page.getByTestId("preview-panel")).toBeVisible();
      await expect(page.getByTestId("caveat-nostate")).toHaveCount(0);
    });
  });
});
