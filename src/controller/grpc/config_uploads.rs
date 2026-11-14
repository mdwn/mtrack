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

//! `ConfigService`'s upload calls: `BeginUpload` announces a file whose bytes
//! the web server is sent (`PUT /uploads/{id}`, see `webui::transfers`), and
//! `AddTrackFiles` and `AddSampleFile` use it. The plain functions behind
//! them are `songs_api::add_track_files` and `sample_files::add_sample_file`;
//! the GDTF and MVR uploads are used by `LightingService`.

use tonic::Status;

use super::config_songs::song_status;
use super::lighting_service::answer_status;
use crate::player::Player;
use crate::proto::player::v1 as proto;
use crate::webui::api::{lighting_api, mvr_api, sample_files, songs_api};
use crate::webui::transfers::{self, Purpose, TransferError};

/// A refused transfer call as a status.
pub(super) fn transfer_status(e: TransferError) -> Status {
    let message = e.to_string();
    match e {
        TransferError::Full => Status::resource_exhausted(message),
        TransferError::NotFound => Status::not_found(message),
        TransferError::AlreadySent | TransferError::Unusable(_) => {
            Status::failed_precondition(message)
        }
        TransferError::Unavailable(_) => Status::unavailable(message),
        TransferError::Io(_) => Status::internal(message),
    }
}

/// The claim of the upload `id` for `purpose`, which must be that and no
/// other.
pub(super) fn claim_one(
    player: &Player,
    id: &str,
    purpose: Purpose,
) -> Result<transfers::Claimed, Status> {
    player
        .transfers()
        .claim(&[id.to_string()], |p| *p == purpose)
        .map_err(transfer_status)?
        .pop()
        .ok_or_else(|| Status::internal("a claim answered nothing"))
}

/// `BeginUpload`: the lock (every purpose but an MVR inspection, a read, is
/// refused while locked, as the lock guard refuses a locked call), the
/// purpose's checks of the name, the size, and the id.
pub(super) fn begin(
    player: &Player,
    config_path: Option<&std::path::Path>,
    req: proto::BeginUploadRequest,
) -> Result<proto::BeginUploadResponse, Status> {
    let purpose =
        proto::UploadPurpose::try_from(req.purpose).unwrap_or(proto::UploadPurpose::Unspecified);
    if purpose != proto::UploadPurpose::MvrInspect {
        player
            .require_unlocked()
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
    }
    let size = req
        .size
        .ok_or_else(|| Status::invalid_argument("size is required"))?;
    if req.song.is_some() && purpose != proto::UploadPurpose::Track {
        return Err(Status::invalid_argument("song is only for a track"));
    }
    let file_name = req.file_name;
    let (purpose, file_name) = match purpose {
        proto::UploadPurpose::Unspecified => {
            return Err(Status::invalid_argument("purpose is required"))
        }
        proto::UploadPurpose::Track => {
            let song = req
                .song
                .ok_or_else(|| Status::invalid_argument("song is required for a track"))?;
            songs_api::check_track_upload(player, &song, &file_name).map_err(song_status)?;
            (Purpose::Track { song }, file_name)
        }
        proto::UploadPurpose::Sample => {
            let config_path =
                config_path.ok_or_else(|| Status::unimplemented("config store not available"))?;
            sample_files::check_sample_upload(config_path, &file_name).map_err(song_status)?;
            (Purpose::Sample, file_name)
        }
        proto::UploadPurpose::GdtfImport => {
            let name = archive_size_checked(size, lighting_api::check_gdtf_upload(&file_name))?;
            (Purpose::GdtfImport, name)
        }
        proto::UploadPurpose::MvrInspect => {
            let name = archive_size_checked(size, mvr_api::check_mvr_upload(&file_name))?;
            (Purpose::MvrInspect, name)
        }
        proto::UploadPurpose::MvrImport => {
            let name = archive_size_checked(size, mvr_api::check_mvr_upload(&file_name))?;
            (Purpose::MvrImport, name)
        }
    };
    let id = player
        .transfers()
        .begin_upload(purpose, file_name, size)
        .map_err(transfer_status)?;
    Ok(proto::BeginUploadResponse {
        path: format!("/uploads/{id}"),
        upload_id: id,
    })
}

/// A GDTF or MVR upload's name (or its refusal), and its size, which may
/// not pass what the transport allowed.
fn archive_size_checked(
    size: u64,
    name: Result<String, crate::webui::api::lighting_writes::Answer>,
) -> Result<String, Status> {
    let name = name.map_err(answer_status)?;
    if size > transfers::ARCHIVE_MAX_SIZE {
        return Err(Status::invalid_argument(format!(
            "{name} is larger than the {} MiB an archive may be",
            transfers::ARCHIVE_MAX_SIZE / (1024 * 1024)
        )));
    }
    Ok(name)
}

pub(super) async fn add_tracks(
    player: &Player,
    req: proto::AddTrackFilesRequest,
) -> Result<proto::AddTrackFilesResponse, Status> {
    let song = req.song;
    let files = player
        .transfers()
        .claim(
            &req.upload_ids,
            |p| matches!(p, Purpose::Track { song: begun } if *begun == song),
        )
        .map_err(transfer_status)?;
    let added = songs_api::add_track_files(player, &song, files)
        .await
        .map_err(song_status)?;
    Ok(proto::AddTrackFilesResponse {
        song: added.song,
        files: added
            .files
            .into_iter()
            .map(|f| proto::TrackFileAdded {
                file: f.file,
                replaced: f.replaced,
            })
            .collect(),
    })
}

pub(super) async fn add_sample(
    player: &Player,
    config_path: &std::path::Path,
    req: proto::AddSampleFileRequest,
) -> Result<proto::AddSampleFileResponse, Status> {
    let file = claim_one(player, &req.upload_id, Purpose::Sample)?;
    let added = sample_files::add_sample_file(config_path, file)
        .await
        .map_err(song_status)?;
    Ok(proto::AddSampleFileResponse {
        file: added.file,
        path: added.path,
    })
}

#[cfg(test)]
mod tests;
