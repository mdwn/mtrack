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

//! `ConfigService`'s song calls: a song's config, its files, waveform and
//! tempo guess, and the browse and import calls. Each maps the request to
//! the plain functions in `webui::api::{songs_api, browse}` (which MCP calls
//! too) and a [`SongError`] to a status.

use tonic::{metadata::MetadataValue, Status};

use crate::player::Player;
use crate::proto::player::v1 as proto;
use crate::webui::api::browse;
use crate::webui::api::songs_api::{self, SongConfig, SongError};
use crate::webui::wire::songs as wire;

/// The metadata key of an `ALREADY_EXISTS` refusal of a song name that
/// another song holds: the name, as bytes.
pub const TAKEN_NAME_KEY: &str = "x-taken-name-bin";

/// A refused song call as a status, keeping the message the web API gave.
pub(super) fn song_status(e: SongError) -> Status {
    let message = e.message();
    match e {
        SongError::Invalid(_) | SongError::InvalidConfig(_) => Status::invalid_argument(message),
        SongError::NotFound(_) => Status::not_found(message),
        SongError::Forbidden(_) => Status::permission_denied(message),
        SongError::Exists(_) => Status::already_exists(message),
        SongError::Taken(taken) => {
            let mut status = Status::already_exists(message);
            status.metadata_mut().insert_bin(
                TAKEN_NAME_KEY,
                MetadataValue::from_bytes(taken.0.as_bytes()),
            );
            status
        }
        SongError::Precondition(_) | SongError::Unprocessable(_) => {
            Status::failed_precondition(message)
        }
        SongError::Internal(_) => Status::internal(message),
    }
}

pub(super) async fn get_config(
    player: &Player,
    req: proto::GetSongConfigRequest,
) -> Result<proto::GetSongConfigResponse, Status> {
    let config = songs_api::read_song_config(player, &req.name)
        .await
        .map_err(song_status)?;
    Ok(match config {
        SongConfig::File(yaml) => proto::GetSongConfigResponse {
            yaml,
            summary: None,
        },
        SongConfig::Gone(summary) => proto::GetSongConfigResponse {
            yaml: String::new(),
            summary: Some(proto::SongConfigSummary {
                name: summary.name,
                duration_ms: summary.duration_ms,
                duration_display: summary.duration_display,
                num_channels: u32::from(summary.num_channels),
                sample_format: summary.sample_format,
                tracks: summary.tracks,
            }),
        },
    })
}

pub(super) async fn create(
    player: &Player,
    req: proto::CreateSongRequest,
) -> Result<proto::CreateSongResponse, Status> {
    songs_api::create_song_and_reload(player, &req.name, req.yaml)
        .await
        .map_err(song_status)?;
    Ok(proto::CreateSongResponse { song: req.name })
}

pub(super) async fn save(
    player: &Player,
    req: proto::SaveSongConfigRequest,
) -> Result<proto::SaveSongConfigResponse, Status> {
    songs_api::save_song_and_reload(player, &req.name, req.yaml)
        .await
        .map_err(song_status)?;
    Ok(proto::SaveSongConfigResponse {})
}

pub(super) async fn delete(
    player: &Player,
    req: proto::DeleteSongRequest,
) -> Result<proto::DeleteSongResponse, Status> {
    songs_api::delete_song(player, &req.name)
        .await
        .map_err(song_status)?;
    Ok(proto::DeleteSongResponse { name: req.name })
}

pub(super) async fn waveform(
    player: &Player,
    req: proto::GetSongWaveformRequest,
) -> Result<proto::Waveform, Status> {
    let cache = player.watch_hub().waveform_cache().clone();
    let tracks = songs_api::song_waveform(player, &cache, &req.name)
        .await
        .map_err(song_status)?;
    Ok(proto::Waveform {
        song_name: req.name,
        tracks: tracks
            .into_iter()
            .map(|(name, peaks)| proto::WaveformTrack { name, peaks })
            .collect(),
    })
}

pub(super) fn guess_tempo(
    player: &Player,
    req: proto::GuessSongTempoRequest,
) -> Result<proto::GuessSongTempoResponse, Status> {
    let guess = songs_api::song_tempo_guess(player, &req.name).map_err(song_status)?;
    let [numerator, denominator] = guess.tempo.time_signature;
    Ok(proto::GuessSongTempoResponse {
        source: match guess.source {
            wire::TempoGuessSource::Midi => proto::TempoGuessSource::Midi,
            wire::TempoGuessSource::BeatGrid => proto::TempoGuessSource::BeatGrid,
        }
        .into(),
        tempo: Some(proto::GuessedTempo {
            start_seconds: guess.tempo.start_seconds,
            bpm: guess.tempo.bpm,
            time_signature_numerator: numerator,
            time_signature_denominator: denominator,
            changes: guess
                .tempo
                .changes
                .iter()
                .map(|c| proto::GuessedTempoChange {
                    measure: c.measure,
                    beat: c.beat,
                    bpm: c.bpm,
                    time_signature_numerator: c.time_signature[0],
                    time_signature_denominator: c.time_signature[1],
                    transition_beats: c.transition_beats,
                })
                .collect(),
            alignment_rms_ms: guess.tempo.alignment_rms_ms,
        }),
        config: Some(proto::SongTempo {
            bpm: guess.config.bpm,
            time_signature: guess.config.time_signature,
            start: guess.config.start,
            changes: guess
                .config
                .changes
                .into_iter()
                .map(|c| proto::SongTempoChange {
                    measure: c.measure,
                    beat: c.beat,
                    bpm: c.bpm,
                    time_signature: c.time_signature,
                    transition: c.transition.map(|t| proto::TempoTransition {
                        ramp: Some(match t {
                            crate::config::TransitionConfig::Beats(b) => {
                                proto::tempo_transition::Ramp::Beats(b)
                            }
                            crate::config::TransitionConfig::Measures(m) => {
                                proto::tempo_transition::Ramp::Measures(m)
                            }
                        }),
                    }),
                })
                .collect(),
        }),
    })
}

fn file_kind(kind: crate::songs::FileKind) -> proto::FileKind {
    match kind {
        crate::songs::FileKind::Directory => proto::FileKind::Directory,
        crate::songs::FileKind::Audio => proto::FileKind::Audio,
        crate::songs::FileKind::Midi => proto::FileKind::Midi,
        crate::songs::FileKind::Lighting => proto::FileKind::Lighting,
        crate::songs::FileKind::Other => proto::FileKind::Other,
    }
}

pub(super) async fn list_files(
    player: &Player,
    req: proto::ListSongFilesRequest,
) -> Result<proto::ListSongFilesResponse, Status> {
    let files = songs_api::song_files(player, &req.name)
        .await
        .map_err(song_status)?;
    Ok(proto::ListSongFilesResponse {
        files: files
            .into_iter()
            .map(|f| proto::DirectoryFile {
                name: f.name,
                kind: file_kind(f.kind).into(),
            })
            .collect(),
    })
}

pub(super) async fn import_file(
    player: &Player,
    config_path: &std::path::Path,
    req: proto::ImportSongFileRequest,
) -> Result<proto::ImportSongFileResponse, Status> {
    let imported = songs_api::import_file_to_song(
        player,
        config_path,
        &req.name,
        &req.path,
        req.filename.as_deref(),
    )
    .await
    .map_err(song_status)?;
    Ok(proto::ImportSongFileResponse {
        file: imported.file,
        song: imported.song,
    })
}

pub(super) fn browse_directory(
    config_path: &std::path::Path,
    req: proto::BrowseDirectoryRequest,
) -> Result<proto::BrowseDirectoryResponse, Status> {
    let listing = browse::browse_directory(config_path, &req.path).map_err(song_status)?;
    Ok(proto::BrowseDirectoryResponse {
        path: listing.path,
        root: listing.root,
        entries: listing
            .entries
            .into_iter()
            .map(|e| proto::BrowseEntry {
                name: e.name,
                path: e.path,
                kind: file_kind(e.kind).into(),
                is_dir: e.is_dir,
            })
            .collect(),
    })
}

pub(super) async fn create_in_directory(
    player: &Player,
    config_path: &std::path::Path,
    req: proto::CreateSongInDirectoryRequest,
) -> Result<proto::CreateSongInDirectoryResponse, Status> {
    let created =
        browse::create_song_in_directory(player, config_path, &req.path, req.name.as_deref())
            .await
            .map_err(song_status)?;
    Ok(proto::CreateSongInDirectoryResponse {
        path: created.path,
        song: created.song,
    })
}

pub(super) async fn bulk_import(
    player: &Player,
    config_path: &std::path::Path,
    req: proto::BulkImportSongsRequest,
) -> Result<proto::BulkImportSongsResponse, Status> {
    let imported = browse::bulk_import(player, config_path, &req.path)
        .await
        .map_err(song_status)?;
    Ok(proto::BulkImportSongsResponse {
        created: imported.created,
        skipped: imported.skipped,
        failed: imported
            .failed
            .into_iter()
            .map(|f| proto::ImportFailure {
                name: f.name,
                error: f.error,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests;
