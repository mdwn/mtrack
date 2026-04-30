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

// Separate Playwright config for the documentation screenshot generator.
// Reuses the same mock-server + dev-server stack as the e2e suite but
// targets the `e2e/screenshots/` directory and runs serially so the
// captured PNGs are deterministic.

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e/screenshots",
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  timeout: 60000,
  use: {
    baseURL: "http://127.0.0.1:5180",
    ...devices["Desktop Chrome"],
  },
  projects: [
    {
      name: "screenshots",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: "http://127.0.0.1:5180",
      },
    },
  ],
  webServer: [
    {
      command: "npx tsx e2e/mock-server/index.ts",
      port: 3111,
      reuseExistingServer: true,
      timeout: 120000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command:
        "VITE_PROXY_TARGET=http://127.0.0.1:3111 npx vite --host 127.0.0.1 --port 5180 --strictPort",
      port: 5180,
      reuseExistingServer: true,
      timeout: 120000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
