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

/** The comment on each line, as written: what must survive a save. */
function commentLines(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split("\n")) {
    const l = raw.trim();
    // Outside a quoted string, `#` or `//` starts a comment.
    let quoted = false;
    for (let i = 0; i < l.length; i++) {
      const c = l[i];
      if (c === '"') quoted = !quoted;
      if (!quoted && (c === "#" || (c === "/" && l[i + 1] === "/"))) {
        out.push(l.slice(i).trimEnd());
        break;
      }
    }
  }
  return out;
}

function lightFilesIn(dir: string): string[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".light"))
    .map((f) => resolve(dir, f));
}

test("a time of whole milliseconds is written as read", () => {
  for (const written of ["@1.005", "@0.001", "@00:00.003", "@2:03.5"]) {
    const source = `show "S" {\n    ${written}\n    a: static, color: "red", duration: 1s\n}\n`;
    const read = rules.document(source);
    const again = rules.document(serializeLightFile(read.document!));
    expect(again.error).toBeUndefined();
    expect(again.document!.shows[0].cues[0].timestamp).toEqual(
      read.document!.shows[0].cues[0].timestamp,
    );
  }
});

// A comment between the elements of each kind of list, the line it sits
// on as it must be written back.
const listComments: [string, string][] = [
  [
    'tempo {\n    bpm: 100\n    changes: [\n        @2/1 { bpm: 120, # c\n            transition: 2m }\n    ]\n}\nshow "S" {\n    @1/1\n    a: static, color: "red", duration: 1s\n}\n',
    "@2/1 { bpm: 120, # c",
  ],
  [
    'sequence "x"(\n    hit, # c\n    bed // d\n), length: 1measure {\n    @1/1\n    a: static, color: $hit, duration: 1beat\n}\n',
    'sequence "x"(hit, # c',
  ],
  [
    'show "S" {\n    @1/1\n    movers: move, pan: 45deg, # c\n        tilt: 10deg, duration: 1s\n}\n',
    "movers: move, pan: 45deg, # c",
  ],
  [
    'show "S" {\n    @1/1\n    master(layer: background, # c\n        intensity: 50%)\n}\n',
    "master(layer: background, # c",
  ],
  [
    'show "S" {\n    @1/1\n    sequence "x"(hit: "#FF0000", # c\n        bed: "#0000FF")\n}\n',
    'sequence "x"(hit: "#FF0000", # c',
  ],
  [
    'show "S" {\n    @1/1\n    front, # c\n    back: static, color: "red", duration: 1s\n}\n',
    "front, # c",
  ],
];

test("a comment between a list's elements is written back in place", () => {
  for (const [source, onLine] of listComments) {
    const read = rules.document(source);
    expect(read.error, source).toBeUndefined();
    const written = serializeLightFile(read.document!);
    const again = rules.document(written);
    expect(again.error, written).toBeUndefined();
    expect(stripped(again.document)).toEqual(stripped(read.document));
    expect(commentLines(written)).toEqual(commentLines(source));
    expect(
      written.split("\n").map((l) => l.trim()),
      written,
    ).toContain(onLine);
  }
});

test("a tempo start is written as read until its value or unit changes", () => {
  const read = rules.document(
    'tempo {\n    start: 500ms\n    bpm: 120\n}\nshow "S" {\n    @1/1\n}\n',
  ).document!;
  const tempo = read.tempo!;
  const withStart = (start: Partial<typeof tempo.start>) =>
    serializeLightFile({
      ...read,
      tempo: { ...tempo, start: { ...tempo.start, ...start } },
    });
  expect(serializeLightFile(read)).toContain("start: 500ms\n");
  // `500ms` ends with `s`: switched to seconds it is no longer that text.
  expect(withStart({ unit: "s" })).toContain("start: 500s\n");
  expect(withStart({ value: 250 })).toContain("start: 250ms\n");
  expect(withStart({ unit: "s", value: 0.5 })).toContain("start: 0.5s\n");
  const decimal = rules.document("tempo {\n    start: 0.50s\n}\n").document!;
  expect(serializeLightFile(decimal)).toContain("start: 0.50s\n");
});

test("each offset line keeps its own comments", () => {
  const source =
    'show "S" {\n    @1/1\n    offset 1 measures # a\n    # between\n    offset 2 measures # b\n}\n';
  const read = rules.document(source).document!;
  // Untouched, the file comes back as it was.
  expect(serializeLightFile(read)).toBe(source);
  // An edited sum is one line, every comment kept in order.
  const show = read.shows[0];
  const edited = serializeLightFile({
    ...read,
    shows: [{ ...show, cues: [{ ...show.cues[0], offset_measures: 5 }] }],
  });
  expect(edited).toBe(
    'show "S" {\n    @1/1\n    # a\n    # between\n    offset 5 measures # b\n}\n',
  );
});

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
      // Every comment line comes back, as written and in order; the bytes
      // differ in whitespace alone where the file's own spacing did.
      expect(commentLines(written)).toEqual(commentLines(source));
      const bytes = (text: string) =>
        text
          .split("\n")
          .map((l) => l.trim().replace(/\s+/g, " "))
          .filter((l) => l !== "");
      const differing = bytes(source).filter((l, i) => l !== bytes(written)[i]);
      if (process.env.MTRACK_LIGHT_DIFF) {
        console.log(
          `${path}: ${differing.length} lines differ beyond whitespace`,
        );
        for (const l of differing.slice(0, 6)) console.log(`  ${l}`);
      }
    });
  }
}
