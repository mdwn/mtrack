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
//! A rig's side of the mtrack hub: pairing with it, uploading the project
//! this rig holds, and pulling the project's current revision.
//!
//! Built only with the `hub` feature, which is not a default one: nothing in
//! a released player reaches this. There is no UI for it yet, only the `mtrack
//! hub` subcommands the feature adds.
//!
//! A project is its files ([`mtrack_core::project::Manifest`]). The hub holds
//! each file once, by its SHA-256, and a revision is a manifest. Uploading
//! sends the files the hub lacks and then the manifest; pulling fetches the
//! files this rig lacks and then puts them in place ([`apply`]), keeping what
//! they replaced so the pull can be undone with no network.
//!
//! A pull is made so as not to leave the rig unable to play. The files are
//! fetched beside the project and checked by hash; the project as it would
//! be is loaded with the player's own loader; what could stop a move
//! part-way is looked for first; and only then is anything in the project
//! moved. The moves are journaled, so one cut short is finished by the next
//! hub command, or by the player before it reads the project. What it does
//! not survive is a move that keeps failing once the moves have begun: the
//! project then stays part-way until the cause is dealt with ([`apply`]).
//!
//! Not here yet: the running player is told nothing of a pull, and its lock
//! is not asked. These work on a project's directory, with the player
//! stopped.

pub mod apply;
pub mod client;
pub mod protocol;
#[cfg(test)]
pub mod standin;
pub mod state;
pub mod sync;

/// The hub a rig pairs with unless it is told another.
pub const DEFAULT_HUB: &str = "https://hub.nonchord.com";
