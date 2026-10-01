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

/**
 * Journeys against the real binary. Every test gets its own throwaway project
 * directory and its own `mtrack start` on a free port, unlocked, with no
 * hardware: what a first-time user's laptop looks like. Nothing is shared
 * between tests, so they run in parallel.
 *
 * The synthetic GDTF and MVR are built here, at test time, from the very
 * strings the Rust tests use (read out of the Rust sources), so the two
 * suites cannot drift and no manufacturer's archive is checked in.
 */

import { test as base, expect } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

/** The repository root (this file is src/webui/svelte/e2e/journeys/). */
export const REPO = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../../..",
);

/** The binary under test: `MTRACK_BIN`, else the debug build (which serves
 *  the UI from `src/webui/svelte/dist/` on disk, so a UI rebuild needs no
 *  Rust rebuild). */
export const BINARY =
  process.env.MTRACK_BIN ?? path.join(REPO, "target/debug/mtrack");

/** A raw string constant out of a Rust source file: `NAME: &str = r#"…"#`. */
function rustString(file: string, name: string): string {
  const source = fs.readFileSync(path.join(REPO, file), "utf8");
  const start = source.indexOf(`${name}: &str = r#"`);
  if (start < 0) throw new Error(`no ${name} in ${file}`);
  const from = source.indexOf('r#"', start) + 3;
  return source.slice(from, source.indexOf('"#', from));
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(data: Buffer): number {
  let c = 0xffffffff;
  for (const byte of data) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of stored (uncompressed) entries — all a GDTF or MVR reader needs. */
export function zip(entries: [string, Buffer][]): Buffer {
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    locals.push(local, nameBytes, data);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, nameBytes);
    offset += 30 + nameBytes.length + data.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

/** The Rust tests' synthetic GDTF ("Synth Brick": "8: RGBS", "Mover 16bit"…). */
export function synthGdtf(): Buffer {
  const xml = rustString(
    "src/lighting/gdtf/description.rs",
    "SYNTHETIC_DESCRIPTION",
  );
  return zip([["description.xml", Buffer.from(xml)]]);
}

/** The web API tests' synthetic MVR: two bricks of the synthetic GDTF, a
 *  fixture whose GDTF is missing, and a focus point. Its scenery object is
 *  left out (it names a mesh the test does not build). */
export function synthMvr(): Buffer {
  const scene = rustString("src/webui/api/mvr_api.rs", "SYNTH_SCENE").replace(
    /<SceneObject[\s\S]*?<\/SceneObject>\n?/,
    "",
  );
  return zip([
    ["GeneralSceneDescription.xml", Buffer.from(scene)],
    ["Astera_PB15.gdtf", synthGdtf()],
  ]);
}

/** A project on disk with a running mtrack over it. */
export interface Project {
  dir: string;
  url: string;
  /** A project file's text, by path relative to the project. */
  read(rel: string): string;
  write(rel: string, text: string): void;
  exists(rel: string): boolean;
  /** Stops the server and starts it again on the same project and port:
   *  a cold boot, which is when a file changed by hand is read. */
  restart(): Promise<void>;
}

// No hardware: the DMX engine runs on the null client (no olad), which is
// what keeps the lighting system, its banner and its reloads real.
const CONFIG = `songs: songs
dmx:
  null_client: true
  universes:
    - universe: 1
      name: main
  lighting:
    current_venue: house
    directories:
      fixture_types: lighting/fixture_types
      venues: lighting/venues
`;

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const port = (server.address() as net.AddressInfo).port;
      server.close(() => resolve(port));
    });
  });
}

async function waitUp(url: string, child: ChildProcess, log: string[]) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null)
      throw new Error(`mtrack exited early:\n${log.join("")}`);
    try {
      // Up, and the hardware (the DMX engine and its lighting system) done
      // initialising, so a journey's first look is at a settled player.
      const res = await fetch(`${url}/api/status`);
      if (res.ok && (await res.json())?.hardware?.init_done) return;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`mtrack did not come up:\n${log.join("")}`);
}

/** Starts a fresh project. `files` are written into it first (paths
 *  relative to the project). */
export async function startProject(
  files: Record<string, string | Buffer> = {},
): Promise<{ project: Project; stop: () => Promise<void> }> {
  if (!fs.existsSync(BINARY))
    throw new Error(
      `no mtrack binary at ${BINARY}: build one (make test-journeys does)`,
    );
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mtrack-journey-"));
  for (const sub of ["songs", "lighting/fixture_types", "lighting/venues"])
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  fs.writeFileSync(path.join(dir, "mtrack.yaml"), CONFIG);
  for (const [rel, body] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), body);
  }
  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const log: string[] = [];
  let child: ChildProcess;
  const boot = async () => {
    child = spawn(
      BINARY,
      ["start", dir, "--web-port", String(port), "--web-address", "127.0.0.1"],
      {
        env: { ...process.env, MTRACK_HOSTNAME: "journey", RUST_LOG: "warn" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout?.on("data", (d) => log.push(String(d)));
    child.stderr?.on("data", (d) => log.push(String(d)));
    await waitUp(url, child, log);
    // It comes up locked, as a show machine should; a journey edits.
    await fetch(`${url}/api/lock`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ locked: false }),
    });
  };
  const halt = async () => {
    if (child.exitCode !== null) return;
    child.kill("SIGTERM");
    await new Promise((r) => {
      const timer = setTimeout(() => {
        child.kill("SIGKILL");
        r(null);
      }, 5000);
      child.once("exit", () => {
        clearTimeout(timer);
        r(null);
      });
    });
  };
  await boot();
  const project: Project = {
    dir,
    url,
    read: (rel) => fs.readFileSync(path.join(dir, rel), "utf8"),
    write: (rel, text) => fs.writeFileSync(path.join(dir, rel), text),
    exists: (rel) => fs.existsSync(path.join(dir, rel)),
    restart: async () => {
      await halt();
      await boot();
    },
  };
  const stop = async () => {
    await halt();
    if (!process.env.MTRACK_JOURNEY_KEEP)
      fs.rmSync(dir, { recursive: true, force: true });
  };
  return { project, stop };
}

/** A test with its own project and server; the page's base URL is it. */
export const test = base.extend<{
  files: Record<string, string | Buffer>;
  project: Project;
}>({
  files: [{}, { option: true }],
  project: async ({ files }, use) => {
    const { project, stop } = await startProject(files);
    await use(project);
    await stop();
  },
  baseURL: async ({ project }, use) => use(project.url),
});

export { expect };

/** GET a JSON API path of the project. */
export async function api<T = Record<string, unknown>>(
  project: Project,
  rel: string,
): Promise<T> {
  const res = await fetch(`${project.url}/api${rel}`);
  if (!res.ok) throw new Error(`GET ${rel}: ${res.status} ${await res.text()}`);
  return res.json() as Promise<T>;
}
