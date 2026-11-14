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

//! Which lighting layers the loaded shows use, so a change to the layer
//! list that would orphan one is refused (design §3.5).

use std::collections::BTreeSet;

use super::Player;
use crate::config;

/// A layer a profile change would remove, and the shows that name it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LayerLoss {
    pub layer: String,
    /// Where it is used, each as `song (file, show "name")`.
    pub shows: Vec<String>,
}

/// The refusal for a profile change that removes or renames layers some
/// shows use: what every surface that saves a profile says.
pub fn layer_loss_message(losses: &[LayerLoss]) -> String {
    let parts: Vec<String> = losses
        .iter()
        .map(|loss| format!("'{}' (used by {})", loss.layer, loss.shows.join("; ")))
        .collect();
    format!(
        "the lighting layers cannot be saved: removing or renaming a layer a show uses would \
         leave that show unable to load; {}. Change those shows first.",
        parts.join(", ")
    )
}

impl Player {
    /// The layers `old` has that `new` lacks (a removed layer, or the old
    /// name of a renamed one) that loaded shows name, each with the shows
    /// that use it. Empty when nothing is lost. `old` is the profile before
    /// the change, `None` for a profile that is new.
    pub fn layer_losses(
        &self,
        old: Option<&config::Profile>,
        new: &config::Profile,
    ) -> Vec<LayerLoss> {
        let layers_of = |profile: &config::Profile| -> Vec<String> {
            config::lighting::layer_set(profile.dmx().and_then(|d| d.lighting()))
                .names()
                .to_vec()
        };
        let Some(old) = old else {
            return Vec::new();
        };
        let kept = layers_of(new);
        let removed: BTreeSet<String> = layers_of(old)
            .into_iter()
            .filter(|name| !kept.contains(name))
            .collect();
        if removed.is_empty() {
            return Vec::new();
        }
        self.shows_using_layers(&removed)
    }

    /// The loaded shows that name any of `names` as a layer, an effect's
    /// `layer:` or a layer command's, by the names of the layers the shows
    /// were parsed against (the player's own).
    pub fn shows_using_layers(&self, names: &BTreeSet<String>) -> Vec<LayerLoss> {
        let layers = self.layers();
        let mut uses: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
        for song in self.songs().sorted_list() {
            for dsl in song.dsl_lighting_shows() {
                let file = crate::util::filename_display(dsl.file_path()).to_string();
                for (show_name, show) in dsl.shows().iter() {
                    let named = show.cues.iter().flat_map(|cue| {
                        cue.effects
                            .iter()
                            .filter_map(|e| e.layer)
                            .chain(cue.layer_commands.iter().filter_map(|c| c.layer))
                    });
                    for layer in named {
                        if let Some(name) = layers.name(layer).filter(|n| names.contains(*n)) {
                            uses.entry(name.to_string())
                                .or_default()
                                .insert(format!("{} ({file}, show \"{show_name}\")", song.name()));
                        }
                    }
                }
            }
        }
        uses.into_iter()
            .map(|(layer, shows)| LayerLoss {
                layer,
                shows: shows.into_iter().collect(),
            })
            .collect()
    }
}

impl Player {
    /// What a save of `new` over the profile at `index` of the config store
    /// would cost the loaded shows in layers: empty to go ahead.
    pub async fn layer_losses_at(&self, index: usize, new: &config::Profile) -> Vec<LayerLoss> {
        let Some(store) = self.config_store() else {
            return Vec::new();
        };
        let current = store.read_config().await;
        let old = current.profile_list().and_then(|list| list.get(index));
        self.layer_losses(old, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::LayerSet;
    use crate::player::SongSources;
    use crate::testutil::player::{assets_player_with_devices, no_devices};

    /// A songs directory with one song whose show uses `accents` and
    /// `blinders` (the effect and a `clear`).
    fn songs_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let song = dir.path().join("songs/one");
        std::fs::create_dir_all(&song).unwrap();
        crate::testutil::write_wav(song.join("t.wav"), vec![vec![0_i32; 441]], 44100).unwrap();
        std::fs::write(
            song.join("song.yaml"),
            "name: One\ntracks:\n  - name: t\n    file: t.wav\nlighting:\n  - file: show.light\n",
        )
        .unwrap();
        std::fs::write(
            song.join("show.light"),
            "show \"main\" {\n    @00:00.000\n    all: static color: \"red\", duration: 1s, layer: accents\n    clear(layer: blinders)\n}\n",
        )
        .unwrap();
        dir
    }

    fn player_on(dir: &tempfile::TempDir, layers: &[&str]) -> Player {
        let player = assets_player_with_devices(no_devices()).unwrap();
        player.set_layers(LayerSet::new(layers.iter().map(|l| l.to_string()).collect()).unwrap());
        player
            .reload_songs_from(&SongSources {
                songs: dir.path().join("songs"),
                playlists_dir: None,
                legacy_playlist: None,
            })
            .unwrap();
        player
    }

    fn profile(layers: &str) -> config::Profile {
        let mut temp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::io::Write::write_all(
            &mut temp,
            format!("songs: s\nprofiles:\n  - dmx:\n      universes: []\n      lighting:\n        layers: {layers}\n")
                .as_bytes(),
        )
        .unwrap();
        config::Player::deserialize(temp.path())
            .unwrap()
            .all_profiles()[0]
            .clone()
    }

    #[test]
    fn songs_load_against_the_players_layers() {
        let dir = songs_dir();
        let player = player_on(&dir, &["background", "accents", "blinders"]);
        assert!(player.songs().get("One").is_ok());
        // The same files against the default three do not load.
        let plain = player_on(&dir, &["background", "midground", "foreground"]);
        assert!(plain.songs().get("One").is_err());
        let songs = plain.songs();
        let failure = &songs.failures()[0];
        assert!(
            failure.error().contains("unknown layer 'accents'"),
            "{}",
            failure.error()
        );
    }

    #[test]
    fn removing_or_renaming_a_layer_a_show_uses_is_a_loss_listing_the_shows() {
        let dir = songs_dir();
        let player = player_on(&dir, &["background", "accents", "blinders"]);
        let old = profile("[background, accents, blinders]");

        // Removed.
        let losses = player.layer_losses(Some(&old), &profile("[background, blinders]"));
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0].layer, "accents");
        assert_eq!(losses[0].shows, ["One (show.light, show \"main\")"]);
        let message = layer_loss_message(&losses);
        assert!(message.contains("'accents'"), "{message}");
        assert!(message.contains("show.light"), "{message}");

        // Renamed: the old name is lost, a layer command counts as a use.
        let losses = player.layer_losses(Some(&old), &profile("[background, accents, glare]"));
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0].layer, "blinders");

        // Reordered, added to, or a layer nobody uses removed: nothing lost.
        for ok in [
            "[blinders, accents, background]",
            "[background, accents, blinders, extra]",
        ] {
            assert!(
                player.layer_losses(Some(&old), &profile(ok)).is_empty(),
                "{ok}"
            );
        }
        let with_spare = profile("[background, spare, accents, blinders]");
        assert!(player
            .layer_losses(
                Some(&with_spare),
                &profile("[background, accents, blinders]")
            )
            .is_empty());
        // A new profile has nothing to lose.
        assert!(player.layer_losses(None, &profile("[x]")).is_empty());
    }
}
