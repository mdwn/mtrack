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

//! Configuration for the notification audio subsystem.
//!
//! Supports global overrides (in the profile) and per-song overrides (in song YAML).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Global notification audio configuration.
///
/// Configured in the hardware profile to override default notification tones
/// with custom audio files. All paths are relative to the config base directory
/// unless absolute.
///
/// ```yaml
/// notifications:
///   loop_armed: /path/to/custom_loop_armed.wav
///   break_requested: /path/to/custom_break.wav
///   loop_exited: /path/to/custom_exited.wav
///   section_entering: /path/to/default_section_enter.wav
///   sections:
///     verse: /path/to/verse_announce.wav
///     chorus: /path/to/chorus_announce.wav
/// ```
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct NotificationConfig {
    /// Override audio for the "loop armed" notification.
    #[serde(default)]
    loop_armed: Option<String>,
    /// Override audio for the "break requested" notification.
    #[serde(default)]
    break_requested: Option<String>,
    /// Override audio for the "loop exited" notification.
    #[serde(default)]
    loop_exited: Option<String>,
    /// Override audio for the generic "section entering" notification.
    #[serde(default)]
    section_entering: Option<String>,
    /// Per-section-name audio overrides.
    #[serde(default)]
    sections: HashMap<String, String>,
}

impl NotificationConfig {
    /// Returns a map of event key → file path for the configured overrides.
    pub fn event_overrides(&self) -> HashMap<String, String> {
        let mut overrides = HashMap::new();
        if let Some(ref path) = self.loop_armed {
            overrides.insert("loop_armed".to_string(), path.clone());
        }
        if let Some(ref path) = self.break_requested {
            overrides.insert("break_requested".to_string(), path.clone());
        }
        if let Some(ref path) = self.loop_exited {
            overrides.insert("loop_exited".to_string(), path.clone());
        }
        if let Some(ref path) = self.section_entering {
            overrides.insert("section_entering".to_string(), path.clone());
        }
        overrides
    }

    /// Returns the per-section-name audio overrides.
    pub fn section_overrides(&self) -> &HashMap<String, String> {
        &self.sections
    }
}

/// Per-song notification audio overrides.
///
/// Configured in the song YAML to override notifications for that specific song.
///
/// ```yaml
/// notification_audio:
///   loop_armed: custom_armed.wav
///   sections:
///     intro: intro_announce.wav
///     bridge: bridge_announce.wav
/// ```
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct SongNotificationConfig {
    /// Override audio for the "loop armed" notification.
    #[serde(default)]
    loop_armed: Option<String>,
    /// Override audio for the "break requested" notification.
    #[serde(default)]
    break_requested: Option<String>,
    /// Override audio for the "loop exited" notification.
    #[serde(default)]
    loop_exited: Option<String>,
    /// Override audio for the generic "section entering" notification.
    #[serde(default)]
    section_entering: Option<String>,
    /// Per-section-name audio overrides.
    #[serde(default)]
    sections: HashMap<String, String>,
}

impl SongNotificationConfig {
    /// Returns a map of event key → file path for the configured overrides.
    pub fn event_overrides(&self) -> HashMap<String, String> {
        let mut overrides = HashMap::new();
        if let Some(ref path) = self.loop_armed {
            overrides.insert("loop_armed".to_string(), path.clone());
        }
        if let Some(ref path) = self.break_requested {
            overrides.insert("break_requested".to_string(), path.clone());
        }
        if let Some(ref path) = self.loop_exited {
            overrides.insert("loop_exited".to_string(), path.clone());
        }
        if let Some(ref path) = self.section_entering {
            overrides.insert("section_entering".to_string(), path.clone());
        }
        overrides
    }

    /// Returns the per-section-name audio overrides.
    pub fn section_overrides(&self) -> &HashMap<String, String> {
        &self.sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    #[test]
    fn notification_config_defaults() {
        let config = NotificationConfig::default();
        assert!(config.event_overrides().is_empty());
        assert!(config.section_overrides().is_empty());
    }

    #[test]
    fn notification_config_serde() {
        let yaml = r#"
            loop_armed: /audio/armed.wav
            break_requested: /audio/break.wav
            sections:
              verse: /audio/verse.wav
              chorus: /audio/chorus.wav
        "#;

        let config: NotificationConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let overrides = config.event_overrides();
        assert_eq!(overrides.get("loop_armed").unwrap(), "/audio/armed.wav");
        assert_eq!(
            overrides.get("break_requested").unwrap(),
            "/audio/break.wav"
        );
        assert!(!overrides.contains_key("loop_exited"));

        let sections = config.section_overrides();
        assert_eq!(sections.get("verse").unwrap(), "/audio/verse.wav");
        assert_eq!(sections.get("chorus").unwrap(), "/audio/chorus.wav");
    }

    #[test]
    fn song_notification_config_serde() {
        let yaml = r#"
            loop_armed: custom_armed.wav
            sections:
              intro: intro.wav
        "#;

        let config: SongNotificationConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let overrides = config.event_overrides();
        assert_eq!(overrides.get("loop_armed").unwrap(), "custom_armed.wav");
        assert_eq!(overrides.len(), 1);

        assert_eq!(
            config.section_overrides().get("intro").unwrap(),
            "intro.wav"
        );
    }
}
