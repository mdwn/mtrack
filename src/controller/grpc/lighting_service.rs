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

//! `LightingService`: the lighting pages' calls, over the same player as
//! `PlayerService`. It is served on the gRPC port and by the web UI's
//! gRPC-Web, behind the same lock; Bluetooth does not carry it. Each RPC
//! maps its request to the plain functions in `webui::api::{lighting_api,
//! fixture_test}` (which MCP calls too, where it shares them) and an
//! [`Answer`] to a status.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use tonic::{metadata::MetadataValue, Request, Response, Status};

use super::lighting_convert as convert;
use crate::player::Player;
use crate::proto::player::v1::{
    self as proto, lighting_service_server::LightingService, DeleteLightShowRequest,
    DeleteLightShowResponse, EvaluateSongLightingRequest, FixtureTestAnswer, FixtureTestOptions,
    GetFixtureTestOptionsRequest, GetLightShowRequest, GetLightShowResponse, GetLightingFitRequest,
    GetLightingReadinessRequest, LightingFit, LightingPreview, LightingReadiness,
    ListLightingGroupsRequest, ListLightingGroupsResponse, SaveLightShowRequest,
    SaveLightShowResponse, StartFixtureTestRequest, StopFixtureTestRequest,
    StopFixtureTestResponse, ValidateLightShowRequest, ValidateLightShowResponse,
};
use crate::webui::api::fixture_test::{self, ControlInput, TestDirs, TestInput};
use crate::webui::api::lighting_api;
use crate::webui::api::lighting_writes::Answer;
use crate::webui::api::songs_api;

/// The metadata key of a refused fixture test that says why, in a word:
/// `playing`, `no_dmx`, `no_universes`, `universe`, `address`, `controls`,
/// `type`, `mode`, `raw` or `files`.
pub const REFUSAL_REASON_KEY: &str = "x-refusal-reason";

/// The lighting service over a player.
pub(crate) struct LightingServer {
    player: Arc<Player>,
}

impl LightingServer {
    pub(crate) fn new(player: Arc<Player>) -> Self {
        Self { player }
    }

    /// The player config's path, canonical when it can be: what the files
    /// the lighting pages write are found from. A player with no config
    /// store has none.
    #[allow(clippy::result_large_err)]
    fn config_path(&self) -> Result<PathBuf, Status> {
        let store = self
            .player
            .config_store()
            .ok_or_else(|| Status::unimplemented("config store not available"))?;
        let path = store.path().to_path_buf();
        Ok(path.canonicalize().unwrap_or(path))
    }

    /// The songs directory, as the player last resolved it.
    fn songs_path(&self) -> PathBuf {
        songs_api::songs_dir(&self.player)
    }
}

/// A refused call as a status, keeping the message the web API gave. A
/// refusal that names its reason (a fixture test's) carries it in
/// [`REFUSAL_REASON_KEY`].
pub(super) fn answer_status(answer: Answer) -> Status {
    let message = answer.message();
    let mut status = match answer.status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            Status::invalid_argument(message)
        }
        StatusCode::FORBIDDEN => Status::permission_denied(message),
        StatusCode::NOT_FOUND => Status::not_found(message),
        StatusCode::CONFLICT | StatusCode::LOCKED => Status::failed_precondition(message),
        s if s.is_server_error() => Status::internal(message),
        _ => Status::unknown(message),
    };
    if let Some(reason) = answer.body["reason"]
        .as_str()
        .and_then(|reason| reason.parse::<MetadataValue<_>>().ok())
    {
        status.metadata_mut().insert(REFUSAL_REASON_KEY, reason);
    }
    status
}

/// A refusal of a fixture test request that never reached the plain
/// functions, for a `reason`.
fn refused(reason: &'static str, message: String) -> Status {
    answer_status(Answer {
        status: StatusCode::BAD_REQUEST,
        body: serde_json::json!({"error": message, "reason": reason}),
    })
}

fn test_dirs(dir: Option<String>, venues_dir: Option<String>) -> TestDirs {
    TestDirs { dir, venues_dir }
}

/// The request's controls as the plain functions read them: a strobe with
/// no rate is off.
fn control_input(c: proto::FixtureTestControls) -> ControlInput {
    ControlInput {
        color: c.color,
        dimmer: c.dimmer,
        strobe: c.strobe.map(|strobe| strobe.hertz),
        pan: c.pan,
        tilt: c.tilt,
        levels: c.levels.into_iter().collect(),
    }
}

#[tonic::async_trait]
impl LightingService for LightingServer {
    async fn get_light_show(
        &self,
        request: Request<GetLightShowRequest>,
    ) -> Result<Response<GetLightShowResponse>, Status> {
        let req = request.into_inner();
        let source = lighting_api::read_light_show(&self.songs_path(), &req.name)
            .await
            .map_err(answer_status)?;
        Ok(Response::new(GetLightShowResponse { source }))
    }

    async fn save_light_show(
        &self,
        request: Request<SaveLightShowRequest>,
    ) -> Result<Response<SaveLightShowResponse>, Status> {
        let req = request.into_inner();
        lighting_api::put_light_show(
            &self.player,
            &self.config_path()?,
            &self.songs_path(),
            &req.name,
            req.source,
        )
        .await
        .map_err(answer_status)?;
        Ok(Response::new(SaveLightShowResponse {}))
    }

    async fn delete_light_show(
        &self,
        request: Request<DeleteLightShowRequest>,
    ) -> Result<Response<DeleteLightShowResponse>, Status> {
        let req = request.into_inner();
        lighting_api::delete_light_show(&self.config_path()?, &self.songs_path(), &req.name)
            .await
            .map_err(answer_status)?;
        Ok(Response::new(DeleteLightShowResponse {}))
    }

    async fn validate_light_show(
        &self,
        request: Request<ValidateLightShowRequest>,
    ) -> Result<Response<ValidateLightShowResponse>, Status> {
        let req = request.into_inner();
        let validation = lighting_api::validate_light_show_text(
            &self.player,
            &self.songs_path(),
            &req.source,
            req.song.as_deref(),
            req.file.as_deref(),
        )
        .map_err(answer_status)?;
        Ok(Response::new(ValidateLightShowResponse {
            valid: validation.valid,
            errors: validation.errors.unwrap_or_default(),
        }))
    }

    async fn get_lighting_readiness(
        &self,
        _: Request<GetLightingReadinessRequest>,
    ) -> Result<Response<LightingReadiness>, Status> {
        let report = lighting_api::readiness(self.player.clone())
            .await
            .map_err(answer_status)?;
        Ok(Response::new(convert::readiness(&report)))
    }

    async fn get_lighting_fit(
        &self,
        _: Request<GetLightingFitRequest>,
    ) -> Result<Response<LightingFit>, Status> {
        let report = lighting_api::fit(self.player.clone())
            .await
            .map_err(answer_status)?;
        Ok(Response::new(convert::fit_report(&report)))
    }

    async fn evaluate_song_lighting(
        &self,
        request: Request<EvaluateSongLightingRequest>,
    ) -> Result<Response<LightingPreview>, Status> {
        let req = request.into_inner();
        let preview = lighting_api::evaluate_song(&self.player, &req.song, &req.times)
            .await
            .map_err(answer_status)?;
        Ok(Response::new(convert::preview(&preview)))
    }

    async fn list_lighting_groups(
        &self,
        _: Request<ListLightingGroupsRequest>,
    ) -> Result<Response<ListLightingGroupsResponse>, Status> {
        let groups = lighting_api::lighting_groups(&self.player, &self.config_path()?)
            .await
            .map_err(answer_status)?;
        Ok(Response::new(convert::groups(&groups)))
    }

    async fn start_fixture_test(
        &self,
        request: Request<StartFixtureTestRequest>,
    ) -> Result<Response<FixtureTestAnswer>, Status> {
        let req = request.into_inner();
        let universe = u16::try_from(req.universe).map_err(|_| {
            refused(
                "universe",
                format!(
                    "universe {} is not one of the running profile's DMX universes",
                    req.universe
                ),
            )
        })?;
        let address = u16::try_from(req.address)
            .map_err(|_| refused("address", "the address must be 1 to 512".to_string()))?;
        let input = TestInput {
            fixture_type: req.fixture_type,
            mode: req.mode,
            universe,
            address,
            controls: control_input(req.controls.unwrap_or_default()),
            raw: req.raw.into_iter().collect(),
        };
        let answer = fixture_test::start_test(
            &self.player,
            &self.config_path()?,
            &test_dirs(req.dir, req.venues_dir),
            input,
        )
        .await
        .map_err(answer_status)?;
        Ok(Response::new(convert::test_answer(&answer)))
    }

    async fn stop_fixture_test(
        &self,
        _: Request<StopFixtureTestRequest>,
    ) -> Result<Response<StopFixtureTestResponse>, Status> {
        fixture_test::stop_test(&self.player);
        Ok(Response::new(StopFixtureTestResponse { active: false }))
    }

    async fn get_fixture_test_options(
        &self,
        request: Request<GetFixtureTestOptionsRequest>,
    ) -> Result<Response<FixtureTestOptions>, Status> {
        let req = request.into_inner();
        let options = fixture_test::test_options(
            &self.player,
            &self.config_path()?,
            &req.fixture_type,
            &test_dirs(req.dir, req.venues_dir),
        )
        .await
        .map_err(answer_status)?;
        Ok(Response::new(convert::test_options(&options)))
    }
}

#[cfg(test)]
mod tests;
