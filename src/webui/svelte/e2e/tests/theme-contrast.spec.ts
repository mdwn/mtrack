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

// Light/dark contrast regressions.
//
// The web UI ships two themes off one set of CSS custom properties, so a
// colour written as a literal — or as `var(--token, #fallback)` where the
// token was never defined — silently pins one theme's value into both. That
// is invisible to type-checking and to every other test here: the element
// still renders, still carries its class, still passes a visibility check.
// It is only wrong to look at.
//
// These tests read the *computed* colour of the elements where that went
// wrong, composite them over their real background chain, and assert a WCAG
// contrast floor in each theme. Text is held to 4.5:1; marks that carry
// meaning by being seen at all (the playhead line, the drag-create ghost,
// hint ticks) are held to the 3:1 non-text floor.

import { test, expect, type Page } from "@playwright/test";

let testCounter = 0;

function freshWsId(label: string): string {
  return `theme-${label}-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;
}

async function sendWsMessage(page: Page, wsId: string, msg: object) {
  await expect(async () => {
    const res = await page.request.post("http://127.0.0.1:3111/test/send-ws", {
      data: { ...msg, _wsId: wsId },
    });
    expect((await res.json()).sent).toBe(1);
  }).toPass({ timeout: 10000 });
}

type Theme = "light" | "dark";
const THEMES: Theme[] = ["light", "dark"];

/** WCAG floors: 4.5:1 for body text, 3:1 for meaningful non-text marks. */
const TEXT_MIN = 4.5;
const MARK_MIN = 3;

async function useTheme(page: Page, theme: Theme) {
  await page.addInitScript((t) => {
    localStorage.setItem("mtrack-theme", t as string);
  }, theme);
  await page.emulateMedia({ colorScheme: theme });
}

interface Probe {
  /** Contrast of the element's text against what is behind it. */
  text: number;
  /** Contrast of its own background fill against what is behind it. */
  fill: number | null;
  /** Contrast of its border against what is behind it. */
  border: number | null;
}

/**
 * Measures an element in the page: composites its colour, background and
 * border over the accumulated background of every ancestor (so a
 * half-transparent fill on a card is judged against the card, not against
 * an assumed white) and returns WCAG contrast ratios.
 */
async function probe(
  page: Page,
  selector: string,
  pseudo?: string,
): Promise<Probe> {
  const result = await page.evaluate(
    ([sel, pseudoEl]) => {
      const toLinear = (c: number) => {
        const v = c / 255;
        return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
      };
      const luminance = (rgb: number[]) =>
        0.2126 * toLinear(rgb[0]) +
        0.7152 * toLinear(rgb[1]) +
        0.0722 * toLinear(rgb[2]);
      const parse = (c: string): number[] | null => {
        // color-mix() computes to `color(srgb r g b / a)` with 0–1 channels,
        // everything else to `rgb()` / `rgba()` with 0–255 channels.
        const srgb = c.match(/color\(srgb\s+([^)]+)\)/);
        if (srgb) {
          const [rgb, alpha] = srgb[1].split("/");
          const parts = rgb
            .trim()
            .split(/\s+/)
            .map((n) => parseFloat(n) * 255);
          return [
            parts[0],
            parts[1],
            parts[2],
            alpha === undefined ? 1 : parseFloat(alpha),
          ];
        }
        const m = c.match(/rgba?\(([^)]+)\)/);
        if (!m) return null;
        const parts = m[1].split(",").map((n) => parseFloat(n));
        return [parts[0], parts[1], parts[2], parts.length > 3 ? parts[3] : 1];
      };
      const composite = (fg: number[], bg: number[]): number[] => {
        const a = fg[3];
        return [
          fg[0] * a + bg[0] * (1 - a),
          fg[1] * a + bg[1] * (1 - a),
          fg[2] * a + bg[2] * (1 - a),
          1,
        ];
      };
      const ratio = (a: number[], b: number[]) => {
        const [l1, l2] = [luminance(a), luminance(b)];
        const [hi, lo] = l1 > l2 ? [l1, l2] : [l2, l1];
        return (hi + 0.05) / (lo + 0.05);
      };

      const el = document.querySelector(sel);
      if (!el) return null;

      // Everything the element is painted on top of, outermost first. A
      // pseudo-element also sits on top of its own host's background.
      const ancestors: Element[] = [];
      const from = pseudoEl ? (el as Element) : el.parentElement;
      for (let n: Element | null = from; n; n = n.parentElement) {
        ancestors.push(n);
      }
      ancestors.reverse();
      let behind = [255, 255, 255, 1];
      for (const n of ancestors) {
        const c = parse(getComputedStyle(n).backgroundColor);
        if (c && c[3] > 0) behind = composite(c, behind);
      }

      const cs = getComputedStyle(el, pseudoEl || null);
      const own = parse(cs.backgroundColor);
      const border = parse(cs.borderTopColor);
      const hasBorder = parseFloat(cs.borderTopWidth) > 0;
      // Text sits on the element's own fill when it has one.
      const textBg = own && own[3] > 0 ? composite(own, behind) : behind;
      const text = parse(cs.color);

      return {
        text: text ? ratio(composite(text, textBg), textBg) : 0,
        fill: own && own[3] > 0 ? ratio(composite(own, behind), behind) : null,
        border:
          hasBorder && border && border[3] > 0
            ? ratio(composite(border, behind), behind)
            : null,
      };
    },
    [selector, pseudo] as [string, string | undefined],
  );

  if (!result) throw new Error(`no element matched ${selector}`);
  return {
    text: Number(result.text.toFixed(2)),
    fill: result.fill === null ? null : Number(result.fill.toFixed(2)),
    border: result.border === null ? null : Number(result.border.toFixed(2)),
  };
}

const SONG = "Test Song Beta";
const SONG_ENC = encodeURIComponent(SONG);

const SONG_YAML = `name: ${SONG}
tracks:
  - name: guitar
    file: guitar.wav
sections:
  - name: verse
    start_measure: 1
    end_measure: 4
  - name: chorus
    start_measure: 5
    end_measure: 8
tempo:
  bpm: 120
  time_signature: 4/4
  start: 0
pilot:
  track: pilot
  hints:
    - at: { measure: 3 }
      label: verse
`;

const PLAYBACK = {
  type: "playback",
  is_playing: true,
  elapsed_ms: 0,
  song_name: SONG,
  song_duration_ms: 34000,
  playlist_name: "setlist",
  playlist_position: 1,
  playlist_songs: ["Test Song Alpha", SONG],
  tracks: [],
  available_playlists: ["all_songs", "setlist"],
  persisted_playlist_name: "setlist",
  locked: false,
};

// A label-only hint immediately followed by its countdown: their display
// windows overlap, so at 55.5s "bridge" is live while "3..2..1" is not —
// which is what lets the test compare the highlight against its sibling.
const HINTS = [
  {
    label: "bridge",
    at_ms: 55000,
    start_ms: 55000,
    end_ms: 55000,
    has_audio: false,
  },
  {
    label: "3..2..1",
    at_ms: 60000,
    start_ms: 57000,
    end_ms: 60000,
    has_audio: true,
  },
];

async function openTimeline(page: Page, yaml: string, wsId?: string) {
  await page.route(`**/api/songs/${SONG_ENC}`, async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    await route.fulfill({ status: 200, contentType: "text/yaml", body: yaml });
  });
  await page.route("**/api/songs", async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    const res = await route.fetch();
    const data = await res.json();
    for (const song of data.songs ?? []) {
      if (song.name === SONG) {
        song.has_tempo_map = true;
        song.duration_ms = 34000;
        song.duration_display = "0:34";
      }
    }
    await route.fulfill({ response: res, json: data });
  });
  await page.goto(
    wsId
      ? `/?wsId=${wsId}#/songs/${SONG_ENC}/sections`
      : `/#/songs/${SONG_ENC}/sections`,
  );
  await expect(page.locator(".section-timeline-editor")).toBeVisible();
}

for (const theme of THEMES) {
  test.describe(`${theme} theme`, () => {
    test.beforeEach(async ({ page }) => {
      await useTheme(page, theme);
    });

    test("the live pilot cue is the most legible label on the card", async ({
      page,
    }) => {
      const wsId = freshWsId("pilot");
      await page.goto(`/?wsId=${wsId}#/`);
      await expect(page.locator(".playback-card__title")).toBeVisible();
      await sendWsMessage(page, wsId, {
        ...PLAYBACK,
        song_duration_ms: 240000,
        elapsed_ms: 55500,
        pilot_hints: HINTS,
      });
      await expect(
        page.locator(".playback-card__hint-label--live"),
      ).toHaveCount(1);

      const live = await probe(page, ".playback-card__hint-label--live");
      expect(live.text).toBeGreaterThanOrEqual(TEXT_MIN);

      // The highlight has to beat the un-highlighted state, not just clear
      // the floor: amber-400 on a white card scored 1.83 against a 5.15
      // grey sibling, which read as the live cue being *de*-emphasised.
      const idle = await probe(
        page,
        ".playback-card__hint-label:not(.playback-card__hint-label--live)",
      );
      expect(live.text).toBeGreaterThan(idle.text);
    });

    test("pilot hint ticks are visible on the scrub bar", async ({ page }) => {
      const wsId = freshWsId("ticks");
      await page.goto(`/?wsId=${wsId}#/`);
      await expect(page.locator(".playback-card__title")).toBeVisible();
      await sendWsMessage(page, wsId, {
        ...PLAYBACK,
        song_duration_ms: 240000,
        elapsed_ms: 1000,
        pilot_hints: HINTS,
      });
      await expect(page.locator(".scrub__hint").first()).toBeVisible();

      const tick = await probe(page, ".scrub__hint");
      expect(tick.fill).toBeGreaterThanOrEqual(MARK_MIN);
    });

    test("the timeline playhead and its readout stand out", async ({
      page,
    }) => {
      const wsId = freshWsId("playhead");
      await openTimeline(page, SONG_YAML, wsId);
      await sendWsMessage(page, wsId, { ...PLAYBACK, elapsed_ms: 12000 });
      await expect(page.locator(".playhead-info")).toBeVisible();

      const readout = await probe(page, ".playhead-info__pos");
      expect(readout.text).toBeGreaterThanOrEqual(TEXT_MIN);

      // The playhead line is drawn as a ::before, so it has to be probed
      // as one — reading the host element would measure nothing.
      const line = await probe(page, ".playhead", "::before");
      expect(line.fill).toBeGreaterThanOrEqual(MARK_MIN);

      // The time pill only exists mid-drag.
      const playhead = page.locator(".playhead");
      const box = await playhead.boundingBox();
      if (!box) throw new Error("playhead has no bounding box");
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.mouse.down();
      await page.mouse.move(box.x + 40, box.y + box.height / 2, { steps: 6 });
      await expect(page.locator(".playhead__time")).toBeVisible();

      const badge = await probe(page, ".playhead__time");
      expect(badge.text).toBeGreaterThanOrEqual(TEXT_MIN);
      expect(badge.fill).toBeGreaterThanOrEqual(MARK_MIN);

      await page.mouse.up();
    });

    test("the drag-to-create ghost is visible while dragging", async ({
      page,
    }) => {
      // Only the first section, so the rest of the bar is empty and a drag
      // there creates a new one.
      const oneSection = SONG_YAML.replace(
        `  - name: chorus
    start_measure: 5
    end_measure: 8
`,
        "",
      );
      await openTimeline(page, oneSection);

      const bar = page.locator(".section-bar .bar-content");
      const box = await bar.boundingBox();
      if (!box) throw new Error("section bar has no bounding box");
      await page.mouse.move(box.x + box.width * 0.65, box.y + box.height / 2);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width * 0.9, box.y + box.height / 2, {
        steps: 12,
      });

      const ghost = page.locator(".section-block.creating");
      await expect(ghost).toBeVisible();
      // `toBeVisible` passed on white-on-white too — the ghost was in the
      // DOM at full opacity and simply the same colour as the bar behind it.
      const measured = await probe(page, ".section-block.creating");
      expect(measured.border).toBeGreaterThanOrEqual(MARK_MIN);
      expect(measured.fill).toBeGreaterThan(1);

      await page.mouse.up();
    });
  });
}

test("no component pins a colour through an undefined custom property", async ({
  page,
}) => {
  // `var(--token, #literal)` on a token that was never defined resolves to
  // the literal in both themes — the theme switch cannot reach it. Assert
  // the tokens these components name actually exist.
  await page.goto("/#/");
  const tokens = [
    "--nc-amber-400",
    "--nc-amber-fg",
    "--nc-ink",
    "--nc-error",
    "--accent",
    "--nc-fg-1",
    "--nc-bg-1",
  ];
  const missing = await page.evaluate((names) => {
    const root = getComputedStyle(document.documentElement);
    return names.filter((n) => !root.getPropertyValue(n).trim());
  }, tokens);
  expect(missing).toEqual([]);
});
