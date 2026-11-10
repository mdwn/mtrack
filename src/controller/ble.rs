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

//! Bluetooth LE remote control. The codec frames envelopes for ATT
//! chunks and the dispatcher answers them by calling the player's gRPC
//! service in process; neither touches a radio.

pub mod codec;
// Nothing calls the dispatcher until the BLE driver exists.
#[allow(dead_code)]
pub(crate) mod dispatch;
