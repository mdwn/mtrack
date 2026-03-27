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

//! Notification event types for the audio notification subsystem.

/// Events that trigger audio notifications during playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    /// Playback has entered a named section boundary.
    SectionEntering { section_name: String },
    /// The user has acknowledged a section loop (loop is armed).
    LoopArmed,
    /// The user has requested to break out of a section loop.
    BreakRequested,
    /// The section loop has been exited (playback continues normally).
    LoopExited,
}

impl NotificationEvent {
    /// Returns the override lookup key for this event.
    ///
    /// For `SectionEntering`, returns `"section:<name>"` which can fall back
    /// to `"section_entering"` if no per-section override exists.
    pub fn override_key(&self) -> String {
        match self {
            NotificationEvent::SectionEntering { section_name } => {
                format!("section:{}", section_name)
            }
            NotificationEvent::LoopArmed => "loop_armed".to_string(),
            NotificationEvent::BreakRequested => "break_requested".to_string(),
            NotificationEvent::LoopExited => "loop_exited".to_string(),
        }
    }

    /// Returns the generic fallback key for this event type.
    /// Only differs from `override_key()` for `SectionEntering`.
    pub fn fallback_key(&self) -> &'static str {
        match self {
            NotificationEvent::SectionEntering { .. } => "section_entering",
            NotificationEvent::LoopArmed => "loop_armed",
            NotificationEvent::BreakRequested => "break_requested",
            NotificationEvent::LoopExited => "loop_exited",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_key_section_entering() {
        let event = NotificationEvent::SectionEntering {
            section_name: "verse".to_string(),
        };
        assert_eq!(event.override_key(), "section:verse");
        assert_eq!(event.fallback_key(), "section_entering");
    }

    #[test]
    fn override_key_fixed_events() {
        assert_eq!(NotificationEvent::LoopArmed.override_key(), "loop_armed");
        assert_eq!(
            NotificationEvent::BreakRequested.override_key(),
            "break_requested"
        );
        assert_eq!(NotificationEvent::LoopExited.override_key(), "loop_exited");
    }

    #[test]
    fn fallback_key_matches_override_for_fixed_events() {
        assert_eq!(
            NotificationEvent::LoopArmed.override_key(),
            NotificationEvent::LoopArmed.fallback_key()
        );
        assert_eq!(
            NotificationEvent::BreakRequested.override_key(),
            NotificationEvent::BreakRequested.fallback_key()
        );
        assert_eq!(
            NotificationEvent::LoopExited.override_key(),
            NotificationEvent::LoopExited.fallback_key()
        );
    }
}
