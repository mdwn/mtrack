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

//! The web server's two routes for file bytes (web UI on gRPC design,
//! section 5), the other half of `BeginUpload` and `ExportMvr`:
//!
//! - `PUT /uploads/{upload_id}` streams the body into the staging file of an
//!   upload `BeginUpload` announced.
//! - `GET /downloads/{id}` answers an archive `ExportMvr` staged.
//!
//! They are not lock-checked: the id is the permission, and it was issued under
//! the lock policy. They are on the web
//! server only; the gRPC port serves neither.

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use super::server::WebUiState;
use super::transfers::{content_disposition, TransferError};

/// How long a body may go without a byte before the upload is dropped: an
/// upload being received is not dropped mid-transfer, but one that has stopped
/// is not a transfer, and would hold one of the 32 places until the connection
/// went.
const IDLE_BODY: std::time::Duration = std::time::Duration::from_secs(60);

/// The routes. The default body limit is off for the upload: the declared
/// size, known only once the id is looked up, is the limit.
pub fn router() -> Router<WebUiState> {
    Router::new()
        .route(
            "/uploads/{upload_id}",
            put(put_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/downloads/{download_id}", get(get_download))
}

fn refusal(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}

fn transfer_refusal(e: TransferError) -> Response {
    let status = match &e {
        TransferError::NotFound => StatusCode::NOT_FOUND,
        TransferError::AlreadySent => StatusCode::CONFLICT,
        TransferError::Full => StatusCode::TOO_MANY_REQUESTS,
        TransferError::Unusable(_) => StatusCode::CONFLICT,
        TransferError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        TransferError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    refusal(status, e.to_string())
}

/// `PUT /uploads/{upload_id}`: the bytes of an announced upload, once.
///
/// 404 for an id that is not known (or has expired), 409 for one that has
/// its bytes already (or is getting them), 413 for a body longer than the
/// size announced: reading stops, and the upload and its file are discarded.
/// A body shorter than announced is received; the call that claims the
/// upload refuses it. A body that stops for a minute is dropped (408). 204
/// when the body is in.
async fn put_upload(
    State(state): State<WebUiState>,
    Path(upload_id): Path<String>,
    request: Request,
) -> Response {
    let receive = match state.player.transfers().receive(&upload_id) {
        Ok(receive) => receive,
        Err(e) => return transfer_refusal(e),
    };
    let too_long = || {
        refusal(
            StatusCode::PAYLOAD_TOO_LARGE,
            "The body is longer than the upload said it would be",
        )
    };
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|length| length > receive.size()) {
        return too_long();
    }

    // Created new, in the staging directory, named by the id alone.
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receive.path())
        .await
    {
        Ok(file) => file,
        Err(e) => {
            return refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to stage the upload: {e}"),
            )
        }
    };
    let mut received: u64 = 0;
    let mut body = request.into_body().into_data_stream();
    loop {
        let next = match tokio::time::timeout(IDLE_BODY, body.next()).await {
            Ok(next) => next,
            Err(_) => {
                return refusal(
                    StatusCode::REQUEST_TIMEOUT,
                    "No bytes came for a minute, so the upload was dropped",
                )
            }
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                return refusal(
                    StatusCode::BAD_REQUEST,
                    format!("Failed to read the body: {e}"),
                )
            }
        };
        received += chunk.len() as u64;
        if received > receive.size() {
            return too_long();
        }
        if let Err(e) = file.write_all(&chunk).await {
            return refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to stage the upload: {e}"),
            );
        }
    }
    if let Err(e) = file.flush().await {
        return refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to stage the upload: {e}"),
        );
    }
    drop(file);
    receive.finish(received);
    StatusCode::NO_CONTENT.into_response()
}

/// `GET /downloads/{download_id}`: a staged archive, as an attachment, as
/// often as asked until it expires. 404 for an id that is not known (or has
/// expired).
async fn get_download(
    State(state): State<WebUiState>,
    Path(download_id): Path<String>,
) -> Response {
    let (path, file_name) = match state.player.transfers().download(&download_id) {
        Ok(found) => found,
        Err(e) => return transfer_refusal(e),
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        // Expired between the lookup and the read.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return transfer_refusal(TransferError::NotFound)
        }
        Err(e) => {
            return refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read the download: {e}"),
            )
        }
    };
    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_disposition(&file_name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

#[cfg(test)]
mod tests;
