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

// Documentation screenshots of the Lighting area (mock-server backed), run
// with capture.spec.ts by `npm run screenshots:mock`.
//
// One small rig, "house", is served for every shot with `page.route`, the
// way the e2e specs serve their own state: five PB15 PixelBricks from a GDTF
// (four in 8: RGBS, one in 9: RGBWS overlapping its neighbour) on universe 1
// and two hand-written LED pars on universe 2, which olad has no port
// patched to. The shared mock server is untouched. Live state (the
// metadata and the fixtures' colours) goes over the mock's per-page
// WebSocket, as in capture.spec.ts.

import { test, expect, type Page, type Route } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { FIT, STATUS } from "../mock-server/test-data";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DOCS_IMAGES = path.resolve(
  __dirname,
  "..",
  "..",
  "..",
  "..",
  "..",
  "docs",
  "src",
  "images",
);
mkdirSync(DOCS_IMAGES, { recursive: true });

const DESKTOP = { width: 1280, height: 800 };
const shot = (file: string) => path.join(DOCS_IMAGES, file);

// --- The rig ---------------------------------------------------------------

const BRICK = "PB15 PixelBrick";
const PAR = "LED Par";
const MOVER = "Spot Mover";

/** The PB15's modes, as the archive gives them. */
const MODES = [
  {
    name: "1: RGB",
    channel_count: 3,
    footprint: 3,
    capabilities: ["color"],
    cells: 0,
    channels: [
      [1, "red"],
      [2, "green"],
      [3, "blue"],
    ],
  },
  {
    name: "8: RGBS",
    channel_count: 4,
    footprint: 4,
    capabilities: ["color", "strobe"],
    cells: 0,
    strobe_range: { min_hz: 0.4, max_hz: 25 },
    channels: [
      [1, "red"],
      [2, "green"],
      [3, "blue"],
      [4, "strobe"],
    ],
  },
  {
    name: "9: RGBWS",
    channel_count: 5,
    footprint: 5,
    capabilities: ["color", "strobe"],
    cells: 0,
    strobe_range: { min_hz: 0.4, max_hz: 25 },
    channels: [
      [1, "red"],
      [2, "green"],
      [3, "blue"],
      [4, "white"],
      [5, "strobe"],
    ],
  },
  {
    name: "13: DIM RGBAWS",
    channel_count: 7,
    footprint: 7,
    capabilities: ["color", "dimmer", "strobe"],
    cells: 0,
    strobe_range: { min_hz: 0.4, max_hz: 25 },
    channels: [
      [1, "dimmer"],
      [2, "red"],
      [3, "green"],
      [4, "blue"],
      [5, "amber"],
      [6, "white"],
      [7, "strobe"],
    ],
  },
];
const FOOTPRINT: Record<string, number> = Object.fromEntries(
  MODES.map((m) => [m.name, m.footprint]),
);

interface Fixture {
  name: string;
  fixture_type: string;
  mode: string | null;
  universe: number;
  start_channel: number;
  tags: string[];
  position: [number, number, number];
  rotation: [number, number, number];
}

const FIXTURES: Fixture[] = [
  ...[-3, -1.5, 0, 1.5].map((x, i) => ({
    name: `Brick ${i + 1}`,
    fixture_type: BRICK,
    mode: "8: RGBS",
    universe: 1,
    start_channel: 1 + i * 4,
    tags: ["wash", "floor"],
    position: [x, 0.6, 0.1] as [number, number, number],
    rotation: [120, 0, 0] as [number, number, number],
  })),
  {
    name: "Brick 5",
    fixture_type: BRICK,
    mode: "9: RGBWS",
    universe: 1,
    start_channel: 16,
    tags: ["wash", "floor"],
    position: [3, 0.6, 0.1],
    rotation: [120, 0, 0],
  },
  ...[-2, 2].map((x, i) => ({
    name: `Par ${i + 1}`,
    fixture_type: PAR,
    mode: null,
    universe: 2,
    start_channel: 1 + i * 4,
    tags: ["wash", "back"],
    position: [x, 4, 4.5] as [number, number, number],
    rotation: [-25, 0, 0] as [number, number, number],
  })),
];
const FOCUS: Record<string, [number, number, number]> = {
  drummer: [0, 2.8, 1.4],
};

const footprintOf = (f: Fixture) => (f.mode ? FOOTPRINT[f.mode] : 4);

/** The live colours: bricks in a blue-to-magenta wash, pars amber. */
const LIVE_COLOURS: Record<string, Record<string, number>> = {
  "Brick 1": { red: 40, green: 60, blue: 255, strobe: 0 },
  "Brick 2": { red: 90, green: 40, blue: 255, strobe: 0 },
  "Brick 3": { red: 150, green: 30, blue: 240, strobe: 0 },
  "Brick 4": { red: 210, green: 20, blue: 220, strobe: 0 },
  "Brick 5": { red: 255, green: 20, blue: 180, white: 0, strobe: 0 },
  "Par 1": { red: 255, green: 150, blue: 20, dimmer: 230 },
  "Par 2": { red: 255, green: 150, blue: 20, dimmer: 230 },
};

const METADATA = {
  type: "metadata",
  fixtures: Object.fromEntries(
    FIXTURES.map((f) => [
      f.name,
      {
        tags: f.tags,
        type: f.fixture_type,
        mode: f.mode,
        capabilities:
          f.fixture_type === BRICK ? ["color", "strobe"] : ["color"],
        position: f.position,
        rotation: f.rotation,
        rig: f.fixture_type === BRICK ? "docs-brick/rig-v1.json" : null,
      },
    ]),
  ),
  venue: { name: "house", dir: null, focus_points: FOCUS },
};

function stateWith(extra: Record<string, unknown> = {}) {
  return {
    type: "state",
    fixtures: LIVE_COLOURS,
    active_effects: ["wash"],
    poses: {},
    cells: {},
    ...extra,
  };
}

const FIXTURE_TYPES = {
  [BRICK]: {
    fixture_type: {
      name: BRICK,
      channels: {},
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: null,
    extension: "gdtf",
    referential: true,
    rich: false,
    footprint: null,
    gdtf: {
      fixture: "PB15 PixelBrick",
      manufacturer: "Astera LED Technology",
      modes: MODES.length,
      beam: { type: "Wash", angle: 13 },
      thumbnail: null,
      used_by: 5,
      in_use: [
        { mode: "8: RGBS", count: 4 },
        { mode: "9: RGBWS", count: 1 },
      ],
    },
  },
  [PAR]: {
    fixture_type: {
      name: PAR,
      channels: { red: 1, green: 2, blue: 3, dimmer: 4 },
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "led_par.light",
    extension: "light",
    referential: false,
    rich: false,
    footprint: 4,
    gdtf: null,
  },
  [MOVER]: {
    fixture_type: {
      name: MOVER,
      channels: { pan: 1, tilt: 3, dimmer: 5, red: 6, green: 7, blue: 8 },
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "spot_mover.fixture",
    extension: "fixture",
    referential: false,
    rich: true,
    footprint: 8,
    gdtf: null,
  },
};

/** 4×4 matrix rows for a translation. */
const at = (x: number, y: number, z: number) => [
  [1, 0, 0, x],
  [0, 1, 0, y],
  [0, 0, 1, z],
  [0, 0, 0, 1],
];

/** A PixelBrick as its rig would draw it: a small box, one wash lens. */
const BRICK_RIG = {
  version: 1,
  fixture_type: BRICK,
  mode: "8: RGBS",
  nodes: [
    {
      name: "Body",
      parent: null,
      role: { kind: "body" },
      transform: at(0, 0, 0),
      shape: { shape: "primitive", kind: "Cube", size: [0.09, 0.09, 0.06] },
    },
    {
      name: "Lens",
      parent: 0,
      role: { kind: "beam" },
      transform: at(0, 0, -0.031),
      shape: {
        shape: "primitive",
        kind: "Cylinder",
        size: [0.07, 0.07, 0.004],
      },
    },
  ],
  pan: null,
  tilt: null,
  beams: [
    {
      node: 1,
      angle_deg: 13,
      field_deg: 25,
      kind: "Wash",
      flux_lm: 475,
      cct_k: 5500,
      radius_m: 0.035,
    },
  ],
  thumbnail: null,
  warnings: [],
};

/** A six-cell pixel bar: a long body, a lens and a beam per cell. */
const BAR_CELLS = 6;
const BAR_RIG = {
  version: 1,
  fixture_type: "Pixel Bar",
  mode: "6 pixels RGB",
  nodes: [
    {
      name: "Body",
      parent: null,
      role: { kind: "body" },
      transform: at(0, 0, 0),
      shape: { shape: "primitive", kind: "Cube", size: [1.0, 0.08, 0.08] },
    },
    ...Array.from({ length: BAR_CELLS }, (_, i) => ({
      name: String(i + 1),
      parent: 0,
      role: { kind: "cell", index: i },
      transform: at(-0.42 + i * 0.168, 0, -0.041),
      shape: {
        shape: "primitive",
        kind: "Cylinder",
        size: [0.07, 0.07, 0.004],
      },
    })),
    ...Array.from({ length: BAR_CELLS }, (_, i) => ({
      name: `Beam ${i + 1}`,
      parent: 1 + i,
      role: { kind: "beam" },
      transform: at(0, 0, -0.003),
      shape: { shape: "empty" },
    })),
  ],
  pan: null,
  tilt: null,
  beams: Array.from({ length: BAR_CELLS }, (_, i) => ({
    node: 1 + BAR_CELLS + i,
    angle_deg: 18,
    field_deg: 30,
    kind: "Wash",
    flux_lm: 300,
    cct_k: 6500,
    radius_m: 0.035,
  })),
  thumbnail: null,
  warnings: [],
};

const json = (route: Route, body: unknown) =>
  route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });

const decoded = (route: Route) => decodeURIComponent(route.request().url());

/** The fixture test's options for a type: the PB15's modes, or a
 *  hand-written type's channels. */
function testOptions(name: string) {
  const mode = (
    modeName: string | null,
    channels: [number, string][],
    strobe: { min_hz: number; max_hz: number } | null,
  ) => {
    const has = (c: string) => channels.some(([, n]) => n === c);
    const color = has("red") && has("green") && has("blue");
    return {
      name: modeName,
      drivable: true,
      error: null,
      footprint: channels.length,
      controls: {
        color,
        dimmer: has("dimmer") ? "channel" : color ? "folded" : null,
        strobe: has("strobe") ? strobe : null,
        pan: has("pan") ? { min: -270, max: 270 } : null,
        tilt: has("tilt") ? { min: -135, max: 135 } : null,
        levels: ["white", "amber"].filter(has),
      },
      channels: channels.map(([offset, n]) => ({ offset, name: n })),
    };
  };
  const brick = name === BRICK;
  return {
    fixture_type: name,
    gdtf: brick,
    modes: brick
      ? MODES.map((m) =>
          mode(
            m.name,
            m.channels as [number, string][],
            m.strobe_range ?? null,
          ),
        )
      : [
          mode(
            null,
            Object.entries(
              FIXTURE_TYPES[name as keyof typeof FIXTURE_TYPES].fixture_type
                .channels,
            ).map(([n, o]) => [o, n] as [number, string]),
            null,
          ),
        ],
    default_mode: brick ? "8: RGBS" : null,
    universes: [
      { universe: 1, name: "floor", patched: true },
      { universe: 2, name: "truss", patched: false },
    ],
    olad: { reachable: true, port: 9090 },
    available: true,
    unavailable: null,
    unavailable_message: null,
  };
}

const READINESS = {
  dmx: true,
  venue: {
    name: "house",
    fixtures: FIXTURES.length,
    placed: FIXTURES.length,
    focus_points: Object.keys(FOCUS),
  },
  venue_error: null,
  patch_warnings: [
    {
      kind: "overlap",
      message:
        "Brick 5 (universe 1, addresses 16–20) overlaps Brick 4 (13–16) at address 16.",
    },
  ],
  fixture_types: { in_use: [BRICK, PAR], unresolved: [] },
  groups: [
    { name: "front_wash", fixtures: 5, songs: ["Test Song Alpha"] },
    { name: "back_wash", fixtures: 2, songs: ["Test Song Alpha"] },
    { name: "movers", fixtures: 0, songs: ["Test Song Beta"] },
  ],
  shows: [
    { song: "Test Song Alpha", files: ["show.light"], warnings: [] },
    { song: "Test Song Beta", files: ["show.light"], warnings: [] },
  ],
  output: {
    universes: [1, 2],
    unconfigured: [],
    olad: { reachable: true, unpatched: [2] },
  },
};

const CONFIG_YAML = `songs: songs
profiles:
  - hostname: test-host
    audio:
      device: default
    dmx:
      universes:
        - universe: 1
          name: floor
        - universe: 2
          name: truss
      lighting:
        current_venue: house
        groups:
          front_wash:
            name: front_wash
            constraints:
              - AllOf: ["wash", "floor"]
          back_wash:
            name: back_wash
            constraints:
              - AllOf: ["wash", "back"]
              - FallbackTo: front_wash
          movers:
            name: movers
            constraints:
              - AnyOf: ["moving_head", "spot"]
              - MinCount: 2
samples: {}
`;

interface StatusExtras {
  test_output?: Record<string, unknown> | null;
  lighting_venue?: Record<string, unknown>;
  build_time?: () => string;
  /** The venue's fixtures as its file holds them (default: the rig's). */
  fixtures?: Fixture[];
}

/** Serves the whole rig: fixture types, the GDTF's facts and settings,
 *  the venue and its patch, readiness, the profile, and the status. */
async function rig(page: Page, status: StatusExtras = {}) {
  const fixtures = status.fixtures ?? FIXTURES;
  await page.route(/\/api\/lighting\/assets\/docs-(brick|bar)\//, (route) =>
    json(route, decoded(route).includes("docs-bar/") ? BAR_RIG : BRICK_RIG),
  );
  await page.route("**/api/status", (route) => {
    const body = structuredClone(STATUS) as typeof STATUS & {
      hardware: Record<string, unknown>;
    };
    body.hardware.dmx = { status: "connected", name: "OLA" };
    body.hardware.lighting_venue = status.lighting_venue ?? {
      name: "house",
      status: "ok",
      error: null,
    };
    body.hardware.test_output = status.test_output ?? null;
    if (status.build_time) body.build.build_time = status.build_time();
    return json(route, body);
  });
  await page.route("**/api/config/store", async (route) => {
    const res = await route.fetch();
    const data = await res.json();
    await route.fulfill({
      response: res,
      json: { ...data, yaml: CONFIG_YAML },
    });
  });
  await page.route("**/api/lighting/readiness*", (route) =>
    json(route, READINESS),
  );
  await page.route(/\/api\/lighting\/fixture-types(\/|\?|$)/, (route) => {
    const url = decoded(route);
    const m = url.match(/fixture-types\/([^/?]+)(\/[a-z]+)?/);
    if (!m) return json(route, { fixture_types: FIXTURE_TYPES, errors: [] });
    const [, name, sub] = m;
    const entry = FIXTURE_TYPES[name as keyof typeof FIXTURE_TYPES];
    if (!entry) return route.fallback();
    if (sub === "/gdtf") {
      return json(route, {
        archive: "lighting/library/pb15.gdtf",
        rig: "docs-brick/rig-v1.json",
        thumbnail: null,
        beam: {
          type: "Wash",
          beam_angle: 13,
          field_angle: 25,
          luminous_flux: 475,
          color_temperature: 5500,
          power: 12,
        },
        about:
          "Battery-powered uplight and spotlight; bricks connect into clusters.",
        venues: [
          {
            name: "house",
            fixtures: FIXTURES.filter((f) => f.fixture_type === BRICK).map(
              (f) => ({ name: f.name, mode: f.mode }),
            ),
          },
        ],
        inspection: {
          fixture: "PB15 PixelBrick",
          manufacturer: "Astera LED Technology",
          suggested_name: BRICK,
          fixture_types_dir: "lighting/fixture_types",
          modes: MODES,
        },
      });
    }
    if (sub === "/settings") {
      if (route.request().method() !== "GET")
        return json(route, {
          version: "v2",
          rename: null,
          venue_versions: {},
          config_references: [],
          venue_error: null,
        });
      return json(route, {
        name,
        movement: { max_pan_speed: null, max_tilt_speed: null },
        strobe_curve: null,
        strobe: { steps: 0, automatic: "declared" },
        version: "v1",
      });
    }
    return json(route, {
      ...entry,
      dsl:
        name === PAR
          ? 'fixture_type "LED Par" {\n  channels: 4\n  channel_map: { "red": 1, "green": 2, "blue": 3, "dimmer": 4 }\n}\n'
          : "",
    });
  });
  await page.route("**/api/lighting/fixture-test/options*", (route) => {
    const name = new URL(route.request().url()).searchParams.get(
      "fixture_type",
    );
    return json(route, testOptions(name ?? BRICK));
  });
  await page.route(/\/api\/lighting\/venues(\?.*)?$/, (route) =>
    json(route, {
      venues: {
        house: {
          name: "house",
          fixtures: Object.fromEntries(fixtures.map((f) => [f.name, f])),
          groups: {},
        },
        club: {
          name: "club",
          fixtures: {
            "Wash 1": {
              name: "Wash 1",
              fixture_type: PAR,
              universe: 1,
              start_channel: 1,
              tags: ["wash", "front"],
            },
          },
          groups: {},
        },
      },
    }),
  );
  await page.route(/\/api\/lighting\/venues\/house\/patch(\?.*)?$/, (route) =>
    json(route, {
      spans: fixtures.map((f) => ({
        fixture: f.name,
        universe: f.universe,
        address: f.start_channel,
        footprint: footprintOf(f),
        type: f.fixture_type,
        mode: f.mode,
      })),
      overlaps: [
        {
          universe: 1,
          fixtures: ["Brick 4", "Brick 5"],
          addresses: [16, 16],
        },
      ],
      overruns: [],
    }),
  );
  await page.route(/\/api\/lighting\/venues\/house(\?.*)?$/, (route) => {
    if (route.request().method() !== "GET")
      return json(route, { status: "saved", venue_error: null });
    return json(route, {
      venue: {
        name: "house",
        fixtures: Object.fromEntries(fixtures.map((f) => [f.name, f])),
        focus_points: FOCUS,
        source: null,
      },
      dsl: "",
    });
  });
}

async function pushWs(page: Page, wsId: string, msg: Record<string, unknown>) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

let counter = 0;

/** Opens `hash` on the rig with the venue's live state pushed. */
async function open(
  page: Page,
  hash: string,
  state: Record<string, unknown> = stateWith(),
) {
  const wsId = `docs-lighting-${++counter}-${Date.now()}`;
  await page.goto(`/?wsId=${wsId}${hash}`);
  // The mock's own burst comes first; ours replaces it.
  await page.waitForTimeout(400);
  await pushWs(page, wsId, METADATA);
  await pushWs(page, wsId, state);
  return wsId;
}

/** Hides what an element screenshot of a scrolled page would paint over. */
async function hideStickyNav(page: Page) {
  await page.addStyleTag({ content: ".topnav { display: none !important; }" });
}

test.describe.configure({ mode: "serial" });

test.beforeEach(async ({ page }) => {
  await page.setViewportSize(DESKTOP);
});

// --- Fixture types ---------------------------------------------------------

test("lighting-fixture-types", async ({ page }) => {
  await rig(page);
  await open(page, "#/lighting/fixtures");
  await expect(page.locator(".item-card")).toHaveCount(3);
  await page.waitForTimeout(300);
  await page.screenshot({ path: shot("lighting-fixture-types.png") });
});

test("lighting-fixture-page", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1250 });
  await rig(page);
  await open(page, `#/lighting/fixtures/${encodeURIComponent(BRICK)}`);
  await expect(page.getByTestId("ft-title")).toHaveText(BRICK);
  await page
    .getByTestId("ft-details-modes")
    .getByText("8: RGBS", { exact: false })
    .first()
    .click();
  await expect(page.getByTestId("ft-details-channels")).toBeVisible();
  await expect(page.getByTestId("ft-set-strobe")).toBeVisible();
  await page.waitForTimeout(1500);
  await page.screenshot({ path: shot("lighting-fixture-page.png") });
});

test("lighting-fixture-test", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1100 });
  await rig(page);
  await open(page, `#/lighting/fixtures/${encodeURIComponent(BRICK)}`);
  const section = page.getByTestId("fixture-test");
  await section.locator("summary").click();
  await expect(page.getByTestId("ftest-mode")).toHaveValue("8: RGBS");
  await page.getByTestId("ftest-address").fill("5");
  await page.getByTestId("ftest-send").check();
  await expect(page.getByTestId("ftest-live")).toBeVisible();
  await page.getByTestId("ftest-swatch-red").click();
  await expect(page.getByTestId("ftest-sending")).toContainText("255, 0, 0");
  await expect(page.getByTestId("ftest-help")).toBeVisible();
  await hideStickyNav(page);
  await page.waitForTimeout(300);
  await section.screenshot({ path: shot("lighting-fixture-test.png") });
  await page.getByTestId("ftest-send").uncheck();
});

// --- Venues ----------------------------------------------------------------

test("lighting-venue-editor", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  // Three rows: two bricks in different modes, the second running into the
  // third, and a hand-written par (no mode).
  const small: Fixture[] = [
    { ...FIXTURES[0], name: "Brick 1", mode: "8: RGBS", start_channel: 1 },
    {
      ...FIXTURES[1],
      name: "Brick 2",
      mode: "13: DIM RGBAWS",
      start_channel: 5,
    },
    { ...FIXTURES[5], name: "Par 1", universe: 1, start_channel: 11 },
  ];
  await rig(page, { fixtures: small });
  await open(page, "#/lighting/venues/house?edit");
  const form = page.locator(".editor-form");
  await expect(form).toBeVisible();
  await expect(page.getByTestId("venue-fixture-row")).toHaveCount(3);
  await hideStickyNav(page);
  await page.waitForTimeout(300);
  await form.screenshot({ path: shot("lighting-venue-editor.png") });
});

test("lighting-venue-plot", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await rig(page);
  await open(page, "#/lighting/venues/house");
  await expect(page.locator(".inspector")).toBeVisible();
  await page
    .locator(".inspector")
    .getByLabel("Brick 5", { exact: true })
    .check();
  await expect(page.getByTestId("insp-mode")).toBeVisible();
  await hideStickyNav(page);
  await page.waitForTimeout(500);
  // The card down to the inspector's Apply: the plot, the mode and the
  // patch strip; the aiming tools below are another picture.
  const card = (await page.locator(".stage-card").boundingBox())!;
  const apply = (await page
    .locator(".inspector")
    .getByRole("button", { name: "Apply" })
    .boundingBox())!;
  await page.screenshot({
    path: shot("lighting-venue-plot.png"),
    clip: {
      x: card.x,
      y: card.y,
      width: card.width,
      height: apply.y + apply.height + 16 - card.y,
    },
  });
});

/** The venue card in 3D, once the scene has drawn. */
async function venue3d(page: Page, state = stateWith()) {
  await rig(page);
  await open(page, "#/lighting/venues/house?view=3d", state);
  const viewport = page.locator(".stage3d__viewport");
  await expect(viewport).toHaveAttribute("data-renderer", /webgl|none/, {
    timeout: 15000,
  });
  await expect(viewport).toHaveAttribute(
    "data-fixtures",
    String(FIXTURES.length),
  );
  await page.waitForTimeout(1500);
}

test("stage-3d", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await venue3d(page);
  await hideStickyNav(page);
  await page.locator(".stage-card").screenshot({ path: shot("stage-3d.png") });
});

test("lighting-under-test", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await rig(page, {
    test_output: {
      universe: 1,
      address: 5,
      footprint: 4,
      fixture_type: BRICK,
      mode: "8: RGBS",
      expires_in_secs: 4,
    },
  });
  const colours = structuredClone(LIVE_COLOURS);
  colours["Brick 2"] = { red: 255, green: 0, blue: 0, strobe: 0 };
  await open(
    page,
    "#/lighting/venues/house",
    stateWith({ fixtures: colours, under_test: ["Brick 2"] }),
  );
  await expect(page.getByTestId("stage-test-output")).toBeVisible({
    timeout: 10000,
  });
  await expect(page.locator(".stage-card__viewport > canvas")).toHaveAttribute(
    "data-under-test",
    "Brick 2",
  );
  await hideStickyNav(page);
  await page.waitForTimeout(500);
  await page.locator(".stage-card").screenshot({
    path: shot("lighting-under-test.png"),
  });
});

// --- Pixel cells -------------------------------------------------------------

const BARS = ["Bar 1", "Bar 2", "Bar 3", "Bar 4"];
const CELLS_METADATA = {
  type: "metadata",
  fixtures: Object.fromEntries(
    BARS.map((name, i) => [
      name,
      {
        tags: ["bar", "truss"],
        type: "Pixel Bar",
        capabilities: ["color", "cells"],
        position: [-3.3 + i * 2.2, 3.5, 4.5],
        rotation: [-20, 0, 0],
        rig: "docs-bar/rig-v1.json",
        cells: Array.from({ length: BAR_CELLS }, (_, c) => ({
          name: String(c + 1),
          offset: [-0.42 + c * 0.168, 0, 0],
        })),
      },
    ]),
  ),
  venue: { name: "house", dir: null, focus_points: {} },
};

/** A rainbow along each bar, shifted a little bar by bar. */
function rainbowState() {
  const hue = (h: number) => {
    const f = (n: number) => {
      const k = (n + h * 6) % 6;
      return Math.round(255 * (1 - Math.max(0, Math.min(k, 4 - k, 1))));
    };
    return { red: f(5), green: f(3), blue: f(1) };
  };
  return {
    type: "state",
    fixtures: Object.fromEntries(
      BARS.map((n) => [n, { red: 255, green: 255, blue: 255, dimmer: 255 }]),
    ),
    active_effects: ["rainbow"],
    poses: {},
    cells: Object.fromEntries(
      BARS.map((n, b) => [
        n,
        Object.fromEntries(
          Array.from({ length: BAR_CELLS }, (_, c) => [
            String(c + 1),
            hue(((b * 2 + c) % 12) / 12),
          ]),
        ),
      ]),
    ),
  };
}

async function openCells(page: Page, hash: string) {
  await rig(page);
  const wsId = `docs-cells-${++counter}-${Date.now()}`;
  await page.goto(`/?wsId=${wsId}${hash}`);
  await page.waitForTimeout(400);
  await pushWs(page, wsId, CELLS_METADATA);
  await pushWs(page, wsId, rainbowState());
}

test("stage-plot-cells", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openCells(page, "#/lighting/venues/house");
  await expect(page.locator(".inspector")).toBeVisible();
  await hideStickyNav(page);
  await page.waitForTimeout(500);
  await page
    .locator(".stage-card__viewport")
    .screenshot({ path: shot("stage-plot-cells.png") });
});

test("stage-3d-cells", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openCells(page, "#/lighting/venues/house?view=3d");
  const viewport = page.locator(".stage3d__viewport");
  await expect(viewport).toHaveAttribute("data-renderer", /webgl|none/, {
    timeout: 15000,
  });
  await page.waitForTimeout(2000);
  await viewport.screenshot({ path: shot("stage-3d-cells.png") });
});

// --- Groups, Overview, Fit, MVR ---------------------------------------------

test("lighting-groups", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await rig(page);
  await open(page, "#/lighting/groups");
  await expect(page.getByText("front_wash").first()).toBeVisible();
  await page.getByText("back_wash", { exact: true }).first().click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: shot("lighting-groups.png") });
});

test("lighting-overview", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1100 });
  await rig(page);
  await open(page, "#/lighting");
  await expect(page.getByText("olad has no output port patched")).toBeVisible();
  await page.waitForTimeout(600);
  await page.screenshot({ path: shot("lighting-overview.png") });
});

/** The Fit shows venue's live state: the mock's fit-venue, placed. */
const FIT_METADATA = {
  type: "metadata",
  fixtures: Object.fromEntries(
    FIT.venue.fixtures.map((f) => [
      f.name,
      {
        tags: f.tags,
        type: f.type,
        capabilities: f.capabilities,
        position: f.position,
        rotation: [0, 0, 0],
        rig: null,
      },
    ]),
  ),
  venue: { name: "fit-venue", dir: null, focus_points: { center: [0, 2, 0] } },
};

test("lighting-fit", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  const wsId = `docs-fit-${Date.now()}`;
  await page.goto(`/?wsId=${wsId}#/lighting/fit?group=movers`);
  await page.waitForTimeout(400);
  await pushWs(page, wsId, FIT_METADATA);
  await pushWs(page, wsId, {
    type: "state",
    fixtures: {
      viper1: { red: 255, green: 255, blue: 255, dimmer: 180 },
      viper2: { red: 255, green: 255, blue: 255, dimmer: 180 },
      viper3: { red: 255, green: 255, blue: 255, dimmer: 180 },
      esprite1: { red: 60, green: 120, blue: 255, dimmer: 200 },
      par1: { red: 255, green: 140, blue: 20, dimmer: 220 },
    },
    active_effects: [],
    poses: {},
    cells: {},
  });
  await expect(page.getByText("Groups your shows use")).toBeVisible();
  await page.waitForTimeout(600);
  await page.screenshot({ path: shot("lighting-fit.png") });
});

test("lighting-mvr-review", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/#/lighting/import");
  await page.getByTestId("mvr-file").setInputFiles({
    name: "Kellys.mvr",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("PK-not-really-an-mvr"),
  });
  await page.getByTestId("mvr-continue").click();
  await page.getByTestId("mvr-use-suggestion").click();
  await page.getByTestId("mvr-to-review").click();
  await expect(page.getByTestId("mvr-step-review")).toHaveAttribute(
    "aria-current",
    "step",
  );
  await page.waitForTimeout(500);
  await page.screenshot({ path: shot("lighting-mvr-review.png") });
});

// --- Banners -----------------------------------------------------------------

/** The top nav and the banner line under it. */
async function navWithBanner(page: Page, banner: string, file: string) {
  const line = page.getByTestId(banner);
  await expect(line).toBeVisible({ timeout: 15000 });
  await page.waitForTimeout(200);
  const box = await line.boundingBox();
  if (!box) throw new Error(`${banner} has no bounding box`);
  await page.screenshot({
    path: shot(file),
    clip: { x: 0, y: 0, width: DESKTOP.width, height: box.y + box.height },
  });
}

test("banner-venue-failed", async ({ page }) => {
  await rig(page, {
    lighting_venue: {
      name: "house",
      status: "failed",
      error:
        'fixture "Brick 5" names mode "9: RGBW", which "PB15 PixelBrick" does not have (lighting/venues/house.venue)',
    },
  });
  await page.goto("/#/");
  await navWithBanner(page, "venue-failed-banner", "banner-venue-failed.png");
});

test("banner-test-output", async ({ page }) => {
  await rig(page, {
    test_output: {
      universe: 1,
      address: 5,
      footprint: 4,
      fixture_type: BRICK,
      mode: "8: RGBS",
      expires_in_secs: 4,
    },
  });
  await page.goto("/#/");
  await navWithBanner(page, "test-output-banner", "banner-test-output.png");
});

test("banner-ui-updated", async ({ page }) => {
  // The server's build changes after the first poll: a restart under the tab.
  let polls = 0;
  await rig(page, {
    build_time: () =>
      polls++ === 0 ? "2026-01-01T00:00:00Z" : "2026-02-02T00:00:00Z",
  });
  await page.goto("/#/");
  await navWithBanner(page, "ui-updated", "banner-ui-updated.png");
});
