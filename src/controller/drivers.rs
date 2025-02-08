// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::{error::Error, sync::Arc};

use crate::{config, controller, player::Player};

use super::Driver;

/// Creates a controller driver from the config.
pub(super) fn driver(
    config: config::Controller,
    player: Arc<Player>,
) -> Result<Arc<dyn Driver>, Box<dyn Error>> {
    match config {
        config::Controller::Grpc(config) => {
            Ok(Arc::new(controller::grpc::Driver::new(config, player)?))
        }
        config::Controller::Midi(config) => {
            Ok(Arc::new(controller::midi::Driver::new(config, player)?))
        }
        config::Controller::Keyboard => Ok(Arc::new(controller::keyboard::Driver::new(player))),
        _ => Err("unexpected config arm".into()),
    }
}
