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

import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: {
    chunkSizeWarningLimit: 600,
  },
  server: {
    proxy: {
      "/ws": {
        target: "http://127.0.0.1:8080",
        ws: true,
      },
      "/api": {
        target: "http://127.0.0.1:8080",
      },
      "/player.v1.PlayerService": {
        target: "http://127.0.0.1:8080",
      },
    },
  },
});
