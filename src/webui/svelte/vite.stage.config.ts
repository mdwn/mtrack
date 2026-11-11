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

// The stage-only page (stage.html, src/stage-main.ts) built by itself into
// dist-stage/, for the phone app to ship and show in a web view. Beside the
// normal build (vite.config.ts), which it does not touch.

import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

/**
 * MTRACK_STAGE_3D=1 builds the page with its Plot | 3D switch. Off until
 * the host can answer the 3D view's requests for /api/lighting/assets/.
 */
const threeD = process.env.MTRACK_STAGE_3D === "1";

/**
 * Leaves the 3D view and the show preview out of a build that never opens
 * them. The stage card loads both on demand, so they are chunks of their
 * own (three.js is most of a megabyte) that would ship and never be asked
 * for.
 */
function leaveOut(names: string[]): Plugin {
  const stub = "\0mtrack-stage-left-out";
  return {
    name: "mtrack-stage-leave-out",
    enforce: "pre",
    resolveId(source) {
      if (names.some((name) => source.endsWith(`/${name}`))) return stub;
    },
    load(id) {
      if (id === stub) return "export default undefined;";
    },
  };
}

export default defineConfig({
  plugins: [
    ...(threeD
      ? []
      : [leaveOut(["Stage3DView.svelte", "PreviewPanel.svelte"])]),
    svelte(),
  ],
  // Relative, so the page works from wherever the host serves it.
  base: "./",
  define: {
    "import.meta.env.VITE_STAGE_3D": JSON.stringify(threeD ? "1" : ""),
    // The stage page is not the web UI the server reports a build of.
    "import.meta.env.VITE_MTRACK_UI_BUILD": JSON.stringify(""),
  },
  build: {
    outDir: "dist-stage",
    emptyOutDir: true,
    // The rules module is fetched by its URL, never inlined.
    assetsInlineLimit: 0,
    rollupOptions: {
      input: "stage.html",
      output: {
        manualChunks(id) {
          if (id.includes("/node_modules/three/")) return "three";
        },
      },
    },
  },
});
