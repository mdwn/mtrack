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

use super::devices::DeviceError;
use std::sync::Arc;

use crate::player::{ControllerStatus, Player};

/// Restarts all controllers from the current config and answers their
/// status.
///
/// Refused during playback, to avoid disrupting active control surfaces.
pub(crate) async fn restart_controllers(
    player: &Arc<Player>,
) -> Result<Vec<ControllerStatus>, DeviceError> {
    if player.is_playing().await {
        return Err(DeviceError::WrongState(
            "Cannot restart controllers during playback".to_string(),
        ));
    }
    player
        .reload_controllers()
        .await
        .map_err(|e| DeviceError::Internal(e.to_string()))?;
    Ok(player.controller_statuses())
}
