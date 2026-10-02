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

async function sendWs(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Lets the page paint `n` frames: anything the browser has queued for
 *  after the next frame (a ResizeObserver callback) runs by the second. */
async function frames(page: Page, n: number) {
  await page.evaluate(
    (n) =>
      new Promise<void>((done) => {
        const step = (left: number) =>
          left === 0 ? done() : requestAnimationFrame(() => step(left - 1));
        step(n);
      }),
    n,
  );
}

test.describe("Stage View", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/");
    // Wait for WS to deliver metadata with fixtures
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("stage card is visible on dashboard", async ({ page }) => {
    await expect(page.locator(".stage-card, .card").first()).toBeVisible();
  });

  test("stage viewport contains canvas", async ({ page }) => {
    // Just verify the stage viewport container exists
    await expect(page.locator(".stage-card__viewport")).toBeVisible();
  });

  test("a state message redraws the plot without resetting its canvas", async ({
    page,
  }) => {
    // Setting a canvas's width or height blanks it until the next draw, so
    // the plot flashed empty on every state message when each one tore
    // the draw loop down and set the canvas up again. Count the size
    // assignments the plot's canvas gets while state streams in.
    const wsId = `stage-redraw-${test.info().parallelIndex}-${Date.now()}`;
    await page.addInitScript(() => {
      const counts = new WeakMap<HTMLCanvasElement, number>();
      for (const prop of ["width", "height"] as const) {
        const desc = Object.getOwnPropertyDescriptor(
          HTMLCanvasElement.prototype,
          prop,
        )!;
        Object.defineProperty(HTMLCanvasElement.prototype, prop, {
          ...desc,
          set(this: HTMLCanvasElement, value: number) {
            counts.set(this, (counts.get(this) ?? 0) + 1);
            desc.set!.call(this, value);
          },
        });
      }
      (
        window as unknown as { __canvasResizes: typeof counts }
      ).__canvasResizes = counts;
    });
    await page.goto(`/?wsId=${wsId}#/`);
    const canvas = page.locator(".stage-card__viewport canvas");
    await expect(canvas).toBeVisible();
    const resizes = () =>
      canvas.evaluate(
        (el) =>
          (
            window as unknown as {
              __canvasResizes: WeakMap<HTMLCanvasElement, number>;
            }
          ).__canvasResizes.get(el as HTMLCanvasElement) ?? 0,
      );
    // Let the canvas settle: its first sizing, and the resize observer's
    // first callback, are both done before the baseline is read.
    await expect.poll(resizes).toBeGreaterThan(0);
    await frames(page, 3);
    const settled = await resizes();

    for (let i = 0; i < 5; i++) {
      await sendWs(page, wsId, {
        type: "state",
        fixtures: {
          "front-left": {
            red: 50 * i,
            green: 255 - 50 * i,
            blue: 0,
            dimmer: 255,
          },
          "front-right": { red: 0, green: 50 * i, blue: 255, dimmer: 255 },
        },
        active_effects: [],
      });
      await frames(page, 3);
    }
    expect(await resizes()).toBe(settled);
  });
});
