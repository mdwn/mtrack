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

// The player's rules module loaded under node (no browser): from the
// module file `make gen-wasm` writes, for the specs that hold the UI's
// drawing helpers and its serializer to the player's answers.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import init from "../../src/gen/rules/mtrack_rules";

let loaded: Promise<unknown> | undefined;

/** Loads the module once; every spec awaits this before asking it. The
 *  runner's working directory is the web UI's (`playwright.config.ts`). */
export function loadRules(): Promise<unknown> {
  loaded ??= init({
    module_or_path: readFileSync(
      resolve(process.cwd(), "src/gen/rules/mtrack_rules_bg.wasm"),
    ),
  });
  return loaded;
}
