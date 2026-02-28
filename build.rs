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
use std::{env, error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("player_descriptor.bin"))
        .compile_protos(&["src/proto/player/v1/player.proto"], &["src/proto"])?;

    // Ensure the Svelte dist directory exists so rust-embed compiles without
    // Node.js. A placeholder index.html is created when the real build output
    // is missing.
    let dist_dir = PathBuf::from("src/webui/svelte/dist");
    if !dist_dir.exists() {
        std::fs::create_dir_all(&dist_dir)?;
        std::fs::write(
            dist_dir.join("index.html"),
            "<!DOCTYPE html><html><body><p>Run <code>npm run build</code> in src/webui/svelte/ to build the frontend.</p></body></html>",
        )?;
    }

    Ok(())
}
