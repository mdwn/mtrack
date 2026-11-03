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

// The player's grammar is named by a path from the crate it is compiled in:
// `mtrack_wasm` tells src/lighting/parser/grammar.rs it is this one.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(mtrack_wasm)");
    println!("cargo::rustc-cfg=mtrack_wasm");
    println!("cargo::rerun-if-changed=../../src/lighting/grammar.pest");
}
