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
//! The player's configuration. The files' types and their loaders are
//! `mtrack-core`'s; the store that edits them on a running player is here.

pub use mtrack_core::config::*;

pub mod store;
pub use self::store::ConfigStore;

#[cfg(test)]
mod tests {
    use super::Player;

    const LEGACY_YAML: &str = "songs: songs\nprofiles:\n  - hostname: rig\nstatus_events:\n  \
                               off_events: []\n  idling_events: []\n  playing_events: []\n";

    /// A draft being validated is not the running config: reading one gives
    /// no warning, so it cannot use up the one the loaded config gives.
    #[test]
    fn a_draft_gives_no_legacy_warning() {
        let (layer, lines) = crate::tui::logging::capture_layer(10);
        {
            use tracing_subscriber::layer::SubscriberExt;
            let subscriber = tracing_subscriber::Registry::default().with(layer);
            let _guard = tracing::subscriber::set_default(subscriber);
            let player = Player::deserialize_from_str(LEGACY_YAML).expect("config parses");
            assert!(player.status_events().is_some());
        }
        assert!(lines.lock().is_empty(), "{:?}", lines.lock());
    }
}
