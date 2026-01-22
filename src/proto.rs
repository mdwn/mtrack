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
// Include the generated player code.
pub mod player {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/player.v1.rs"));

        pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
            tonic::include_file_descriptor_set!("player_descriptor");
    }
}
