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

//! `GetLightingAsset`: a file of the lighting asset store as a stream of
//! chunks, for a client (a phone on Bluetooth) that cannot fetch it over
//! HTTP. The rules are the web UI's, which read the file the same way.

use std::{io::Write, path::PathBuf, sync::Arc};

use flate2::{write::GzEncoder, Compression};
use futures_util::{stream::BoxStream, StreamExt};
use tonic::Status;

use crate::lighting::asset_store::{read_asset, AssetError};
use crate::proto::player::v1::AssetChunk;

/// The most data one chunk carries. Small enough that a chunk fits a
/// Bluetooth message with room to spare, and that a phone on a slow link
/// sees progress.
pub(super) const CHUNK_SIZE: usize = 8 * 1024;

pub(super) type AssetStream = BoxStream<'static, Result<AssetChunk, Status>>;

/// Opens the asset at `path` of the store of the project at `project` (none
/// when the player has no config file to find one by) as a stream of
/// chunks, gzipped if `gzip`.
pub(super) async fn open(
    project: Option<PathBuf>,
    path: String,
    gzip: bool,
) -> Result<AssetStream, Status> {
    let Some(project) = project else {
        return Err(Status::not_found("No asset store"));
    };
    let bytes = tokio::task::spawn_blocking(move || {
        let asset = read_asset(&project, &path).map_err(refusal)?;
        if gzip {
            compress(&asset.bytes)
        } else {
            Ok(asset.bytes)
        }
    })
    .await
    .map_err(|e| Status::internal(format!("reading the asset failed: {e}")))??;
    Ok(chunked(bytes))
}

/// The status for an asset that was not read.
fn refusal(error: AssetError) -> Status {
    use crate::webui::safe_path::SafePathError;
    match error {
        AssetError::NotAnAsset => Status::not_found("Not an asset type"),
        AssetError::NoStore => Status::not_found("No asset store"),
        AssetError::Unreadable => Status::not_found("Asset not found"),
        AssetError::Path(SafePathError::NotFound) => Status::not_found("Asset not found"),
        AssetError::Path(SafePathError::InvalidName | SafePathError::OutsideRoot) => {
            Status::invalid_argument("Invalid asset path")
        }
        AssetError::Path(_) => Status::internal("The asset store could not be read"),
    }
}

fn compress(bytes: &[u8]) -> Result<Vec<u8>, Status> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .map_err(|e| Status::internal(format!("compressing the asset failed: {e}")))
}

/// `bytes` as chunks, cut as the stream is read: the first says the total.
/// An empty file is one empty chunk, so that the total is told.
fn chunked(bytes: Vec<u8>) -> AssetStream {
    let bytes = Arc::new(bytes);
    futures_util::stream::unfold(Some(0usize), move |at| {
        let bytes = bytes.clone();
        async move {
            let at = at?;
            let end = (at + CHUNK_SIZE).min(bytes.len());
            let chunk = AssetChunk {
                data: bytes[at..end].to_vec(),
                total_size: if at == 0 { bytes.len() as u64 } else { 0 },
            };
            Some((Ok(chunk), (end < bytes.len()).then_some(end)))
        }
    })
    .boxed()
}
