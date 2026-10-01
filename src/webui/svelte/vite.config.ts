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

import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const proxyTarget = process.env.VITE_PROXY_TARGET || "http://127.0.0.1:8080";

/**
 * The UI's own build identity: the hash of every frontend input, which
 * `make build-ui` computes for its rebuild stamp and passes in as
 * MTRACK_UI_BUILD. The bundle carries it (`import.meta.env
 * .VITE_MTRACK_UI_BUILD`), and `build-info.json` beside it carries the same
 * value for the server to report, so an open tab can tell whether it is the
 * UI the server now serves. A build without it (a plain `npm run build`, the
 * dev server) has none, and the tab falls back to noticing the server's
 * build time change.
 */
const uiBuild = process.env.MTRACK_UI_BUILD?.trim() ?? "";

function buildInfo(): Plugin {
  return {
    name: "mtrack-build-info",
    apply: "build",
    generateBundle() {
      this.emitFile({
        type: "asset",
        fileName: "build-info.json",
        source: JSON.stringify({ ui_build: uiBuild || null }),
      });
    },
  };
}

export default defineConfig({
  plugins: [svelte(), buildInfo()],
  define: {
    "import.meta.env.VITE_MTRACK_UI_BUILD": JSON.stringify(uiBuild),
  },
  build: {
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        // Vite 8 bundles with Rolldown, whose manualChunks only accepts a
        // function (the Rollup object form is no longer supported). Group the
        // vendor libraries into a single cacheable chunk.
        manualChunks(id) {
          const vendorPkgs = [
            "svelte",
            "@connectrpc/connect",
            "@connectrpc/connect-web",
            "@bufbuild/protobuf",
            "yaml",
            "svelte-i18n",
          ];
          if (vendorPkgs.some((pkg) => id.includes(`/node_modules/${pkg}/`))) {
            return "vendor";
          }
          // three.js is the Stage 3D page's alone: its own chunk, loaded
          // with the page's dynamic import and nowhere else.
          if (id.includes("/node_modules/three/")) {
            return "three";
          }
        },
      },
    },
  },
  server: {
    proxy: {
      "/ws": {
        target: proxyTarget,
        ws: true,
      },
      "/api": {
        target: proxyTarget,
      },
      "/player.v1.PlayerService": {
        target: proxyTarget,
      },
    },
  },
});
