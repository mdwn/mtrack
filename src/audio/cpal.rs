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
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;

mod device;
mod manager;
mod profiler;
pub(crate) mod stream;

/// A shared notify handle: a boolean flag protected by a mutex with a condvar for signaling.
type CondvarNotify = Arc<(Mutex<bool>, Condvar)>;

// Re-export public types so external callers see the same paths as before.
pub use device::list_device_info;
pub use device::{AudioDeviceInfo, Device, SupportedFormat};
