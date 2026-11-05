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

// The player's own rules -- beat grid, tempo map, signatures, metronome
// accents, the light show parser and its layout -- compiled to WebAssembly
// (`crates/mtrack-rules-wasm`), with their argument and result types
// generated from the Rust structs. The UI calls these and never re-derives
// a rule of its own. `main.ts` loads the module before mounting, so every
// component may call `rules.*` synchronously; a test under node loads it
// from the file (`e2e/tests/rules-node.ts`).

import * as rules from "../gen/rules/mtrack_rules";

export { rules };
