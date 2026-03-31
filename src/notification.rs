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

//! Audio notification subsystem for section loop feedback.
//!
//! Provides audio notifications (tones and user-overridable sound files)
//! for section loop lifecycle events: entering a section, arming a loop,
//! requesting a break, and exiting a loop.

pub mod audio;
pub mod engine;
pub mod events;

pub use engine::NotificationEngine;
pub use events::NotificationEvent;
