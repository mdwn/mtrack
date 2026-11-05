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

//! What the web UI reads from the player, as Rust types: the HTTP API's
//! payloads and the websocket's messages. Each module is light -- serde and
//! the player's other light types only -- so `crates/mtrack-rules-wasm`
//! compiles it as it is and the UI's TypeScript types are generated from
//! it (`tsify`), leaving the UI no copy of a payload's shape to drift.

pub mod fixture_test;
pub mod messages;
pub mod preview;
pub mod readiness;
pub mod songs;
pub mod status;
pub mod venue;
