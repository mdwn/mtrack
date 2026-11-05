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

// The editor's serializer writes back what the player's document model
// read (no browser): a file read as a document, written out and read
// again is the same document, line numbers aside, for every example show
// and, with `MTRACK_LIGHT_CORPUS=<dir>`, every `.light` file in a folder.
// A file the editor cannot hold (one that defines a venue, or gives a
// show its own tempo) is reported, not round-tripped.

import { test, expect } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { loadRules } from "./rules-node";
import { rules } from "../../src/lib/rules";
import { serializeLightFile } from "../../src/lib/lighting/serializer";

test.beforeAll(async () => {
  await loadRules();
});

/** The document with what the text's layout alone decides removed. */
function stripped(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(stripped);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) {
      if (k === "line") continue;
      out[k] = stripped(v);
    }
    return out;
  }
  return value;
}

function lightFilesIn(dir: string): string[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".light"))
    .map((f) => resolve(dir, f));
}

const folders = [resolve(process.cwd(), "../../../examples/lighting/shows")];
if (process.env.MTRACK_LIGHT_CORPUS) {
  folders.push(process.env.MTRACK_LIGHT_CORPUS);
}

for (const folder of folders) {
  for (const path of lightFilesIn(folder)) {
    test(`round-trips ${path.replace(/^.*\//, "")}`, () => {
      const source = readFileSync(path, "utf8");
      const read = rules.document(source, path);
      if (!read.document) {
        test.skip(true, `not an editor's file: ${read.error}`);
        return;
      }
      const written = serializeLightFile(read.document);
      const again = rules.document(written, path);
      expect(again.error, written).toBeUndefined();
      expect(stripped(again.document)).toEqual(stripped(read.document));
    });
  }
}
