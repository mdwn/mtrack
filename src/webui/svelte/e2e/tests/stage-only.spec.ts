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

// The stage-only page (stage.html, src/stage-main.ts), with this spec as its
// host: the contract the phone app is written against, on the recorded
// stream (e2e/fixtures/stage-stream.json). The page is the dev server's, as
// every spec here has it; the frontend checks build dist-stage.

import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const steps: { what: string; update: string; dropped?: boolean }[] = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "e2e/fixtures/stage-stream.json"),
    "utf8",
  ),
).steps;

interface Stats {
  applied: number;
  dropped: number;
  errors: number;
  fps: number;
}
interface Stage {
  ready: boolean;
  update(base64: string): void;
  reset(): void;
  setTheme(theme: string): void;
  stats(): Stats;
}
type HostMessage = { type: string; message?: string };
type Hosted = {
  mtrackStage: Stage;
  MtrackHost: { postMessage(message: string): void };
  heard: HostMessage[];
};

/** A host that keeps what the page tells it. */
async function listen(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as Hosted;
    w.heard = [];
    w.MtrackHost = {
      postMessage: (message) => w.heard.push(JSON.parse(message)),
    };
  });
}
const heard = (page: Page) =>
  page.evaluate(() => (window as unknown as Hosted).heard);
const stats = (page: Page) =>
  page.evaluate(() => (window as unknown as Hosted).mtrackStage.stats());
const ready = (page: Page) =>
  page.waitForFunction(
    () => (window as unknown as Partial<Hosted>).mtrackStage?.ready === true,
  );
const give = (page: Page, updates: string[]) =>
  page.evaluate((updates) => {
    const stage = (window as unknown as Hosted).mtrackStage;
    for (const update of updates) stage.update(update);
  }, updates);

/** What the page asked the player for: there is no player. */
function watchRequests(page: Page): string[] {
  const asked: string[] = [];
  const ofPlayer = (url: string) => {
    const path = new URL(url).pathname;
    return path.startsWith("/api") || path.startsWith("/player.v1");
  };
  page.on("request", (r) => {
    if (ofPlayer(r.url())) asked.push(r.url());
  });
  // Any websocket is the page asking something of someone (but the dev
  // server's own, for hot reload, which carries a token).
  page.on("websocket", (ws) => {
    if (!new URL(ws.url()).searchParams.has("token")) asked.push(ws.url());
  });
  return asked;
}

test.describe("the stage-only page", () => {
  test("draws the stream its host gives it, and asks the player for nothing", async ({
    page,
  }) => {
    const asked = watchRequests(page);
    await listen(page);
    await page.goto("/stage.html");
    await ready(page);
    expect(await heard(page)).toEqual([{ type: "ready" }]);

    // Nothing given yet.
    await expect(page.getByTestId("stage-empty")).toHaveText(
      "No lights to show.",
    );
    await expect(page.locator(".stage-card")).toHaveCount(0);

    await give(
      page,
      steps.map((s) => s.update),
    );
    // Two layouts and four frames applied; the two frames of a layout the
    // page did not hold dropped.
    expect(await stats(page)).toMatchObject({
      applied: 6,
      dropped: 2,
      errors: 0,
    });

    // The venue's three fixtures, on the plot.
    await expect(page.getByTestId("stage-empty")).toHaveCount(0);
    await expect(page.locator(".stage-card__title")).toContainText(
      "3 fixtures",
    );
    const canvas = page.locator(".stage-card__viewport > canvas");
    await expect(canvas).toBeVisible();
    await expect(canvas).toHaveAttribute("data-under-test", "head");
    await expect
      .poll(() =>
        canvas.evaluate((el: HTMLCanvasElement) => {
          const data = el
            .getContext("2d")!
            .getImageData(0, 0, el.width, el.height).data;
          let painted = 0;
          for (let i = 3; i < data.length; i += 4) if (data[i] > 0) painted++;
          return painted;
        }),
      )
      .toBeGreaterThan(1000);

    // A view and nothing else: no way to the editor, no maximize, no 3D.
    await expect(page.locator(".stage-card__edit")).toHaveCount(0);
    await expect(page.getByTestId("stage-maximize")).toHaveCount(0);
    await expect(page.getByTestId("stage-view-3d")).toHaveCount(0);
    await expect(page.locator("a[href]")).toHaveCount(0);
    // The card is the page, edge to edge.
    const card = await page.locator(".stage-card").boundingBox();
    expect(card!.x).toBe(0);
    expect(card!.width).toBe(page.viewportSize()!.width);

    // The page counts its frames.
    await expect.poll(async () => (await stats(page)).fps).toBeGreaterThan(0);

    expect(asked).toEqual([]);
    expect(await heard(page)).toEqual([{ type: "ready" }]);
  });

  test("queues what it is given before it is ready", async ({ page }) => {
    // The host calls the moment the page's object exists, which is before
    // the rules module has loaded.
    await page.addInitScript(
      (updates) => {
        let stage: Stage | undefined;
        const w = window as unknown as { wasReady?: boolean };
        Object.defineProperty(window, "mtrackStage", {
          configurable: true,
          get: () => stage,
          set(value: Stage) {
            stage = value;
            w.wasReady = value.ready;
            for (const update of updates) value.update(update);
          },
        });
      },
      steps.slice(0, 2).map((s) => s.update),
    );
    await page.goto("/stage.html");
    await ready(page);
    expect(
      await page.evaluate(
        () => (window as unknown as { wasReady?: boolean }).wasReady,
      ),
    ).toBe(false);
    expect(await stats(page)).toMatchObject({ applied: 2, dropped: 0 });
    await expect(page.locator(".stage-card__title")).toContainText(
      "3 fixtures",
    );
    await expect(
      page.locator(".stage-card__viewport > canvas"),
    ).toHaveAttribute("data-under-test", "head");
  });

  test("reset empties the stage, and the next stream starts over", async ({
    page,
  }) => {
    await page.goto("/stage.html");
    await ready(page);
    await give(
      page,
      steps.slice(0, 2).map((s) => s.update),
    );
    await expect(page.locator(".stage-card")).toBeVisible();

    await page.evaluate(() =>
      (window as unknown as Hosted).mtrackStage.reset(),
    );
    await expect(page.getByTestId("stage-empty")).toBeVisible();
    await expect(page.locator(".stage-card")).toHaveCount(0);

    // A frame of the stream that ended is of no layout the page holds.
    await give(page, [steps[1].update]);
    expect(await stats(page)).toMatchObject({ applied: 2, dropped: 1 });
    await expect(page.getByTestId("stage-empty")).toBeVisible();

    await give(page, [steps[0].update]);
    await expect(page.locator(".stage-card")).toBeVisible();
  });

  test("tells its host what it could not read, and keeps what it shows", async ({
    page,
  }) => {
    await listen(page);
    await page.goto("/stage.html");
    await ready(page);
    await give(
      page,
      steps.slice(0, 2).map((s) => s.update),
    );
    // Not base64, then base64 of bytes that are no StageUpdate.
    await give(page, ["not base64!", btoa("\xff\xff\xff")]);
    expect(await stats(page)).toMatchObject({ applied: 2, errors: 2 });
    const messages = await heard(page);
    expect(messages.map((m) => m.type)).toEqual(["ready", "error", "error"]);
    expect(messages[1].message).toMatch(/^stage update: /);
    await expect(page.locator(".stage-card__title")).toContainText(
      "3 fixtures",
    );

    // An error of the page's own reaches the host too.
    await page.evaluate(() => {
      setTimeout(() => {
        throw new Error("thrown by stage-only.spec.ts, on purpose");
      }, 0);
    });
    await expect
      .poll(async () => (await heard(page)).at(-1)?.message)
      .toBe("uncaught error: thrown by stage-only.spec.ts, on purpose");
  });

  test("takes its theme from the system, the address or its host", async ({
    page,
  }) => {
    const html = page.locator("html");
    const background = () =>
      page.evaluate(
        () =>
          getComputedStyle(document.querySelector(".stage-only")!)
            .backgroundColor,
      );

    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto("/stage.html");
    await ready(page);
    await expect(html).toHaveClass(/nc--dark/);
    expect(await background()).toBe("rgb(35, 31, 32)");

    // The system changes while the page is open.
    await page.emulateMedia({ colorScheme: "light" });
    await expect(html).not.toHaveClass(/nc--dark/);
    expect(await background()).toBe("rgb(255, 255, 255)");

    // The host says, whatever the system does.
    const setTheme = (theme: string) =>
      page.evaluate(
        (theme) => (window as unknown as Hosted).mtrackStage.setTheme(theme),
        theme,
      );
    await setTheme("dark");
    await expect(html).toHaveClass(/nc--dark/);
    await setTheme("light");
    await page.emulateMedia({ colorScheme: "dark" });
    await expect(html).not.toHaveClass(/nc--dark/);
    await setTheme("system");
    await expect(html).toHaveClass(/nc--dark/);

    // And in the address, for the first paint.
    await page.goto("/stage.html?theme=light");
    await ready(page);
    await expect(html).not.toHaveClass(/nc--dark/);
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/stage.html?theme=dark");
    await ready(page);
    await expect(html).toHaveClass(/nc--dark/);
  });
});
