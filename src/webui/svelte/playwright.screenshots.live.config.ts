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

// Playwright config for the live-data screenshot generator. Targets a real
// running mtrack instance (default http://localhost:8080) so the captured
// PNGs reflect realistic content — multiple songs, real playlists, and the
// hardware profiles installed on the operator's machine. State-controlled
// shots (lock chip, fake "now playing", etc.) live in the mock-server config
// instead.
//
// Override the target with MTRACK_URL=http://host:port npm run screenshots:live.

import { defineConfig, devices } from "@playwright/test";

const MTRACK_URL = process.env.MTRACK_URL ?? "http://localhost:8080";

export default defineConfig({
  testDir: "./e2e/screenshots",
  testMatch: /capture-live\.spec\.ts/,
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  timeout: 60000,
  use: {
    baseURL: MTRACK_URL,
    ...devices["Desktop Chrome"],
  },
  projects: [
    {
      name: "screenshots-live",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: MTRACK_URL,
      },
    },
  ],
});
