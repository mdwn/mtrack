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

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::router;
use crate::webui::api::test_helpers::test_state;
use crate::webui::server::WebUiState;
use crate::webui::transfers::{Purpose, TransferError};

fn track() -> Purpose {
    Purpose::Track {
        song: "Song".to_string(),
    }
}

fn is_track(p: &Purpose) -> bool {
    matches!(p, Purpose::Track { .. })
}

async fn send(
    state: &WebUiState,
    request: Request<Body>,
) -> (StatusCode, Vec<u8>, http::HeaderMap) {
    let response = router()
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        body.collect().await.unwrap().to_bytes().to_vec(),
        parts.headers,
    )
}

fn put(id: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(format!("/uploads/{id}"))
        .body(body)
        .unwrap()
}

/// A body of `chunks`, with no length known up front.
fn streamed(chunks: Vec<Vec<u8>>) -> Body {
    Body::from_stream(futures_util::stream::iter(
        chunks.into_iter().map(Ok::<_, std::io::Error>),
    ))
}

fn staged_files(state: &WebUiState) -> usize {
    std::fs::read_dir(state.player.transfers().staging_dir().unwrap())
        .unwrap()
        .count()
}

#[tokio::test]
async fn the_bytes_are_staged_and_the_claim_has_them() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 6).unwrap();

    let (status, body, _) = send(&state, put(&id, Body::from("abcdef"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body:?}");
    let claimed = transfers.claim(&[id], is_track).unwrap();
    assert_eq!(claimed[0].read().unwrap(), b"abcdef");
}

#[tokio::test]
async fn a_body_of_many_chunks_is_streamed_whole() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 9).unwrap();
    let chunks = vec![b"abc".to_vec(), b"def".to_vec(), b"ghi".to_vec()];
    let (status, _, _) = send(&state, put(&id, streamed(chunks))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let claimed = transfers.claim(&[id], is_track).unwrap();
    assert_eq!(claimed[0].read().unwrap(), b"abcdefghi");
}

/// The default body limit (2 MiB) does not apply: the declared size is the
/// limit.
#[tokio::test]
async fn a_body_past_the_default_limit_is_taken_when_it_was_declared() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let size = 5 * 1024 * 1024;
    let id = transfers
        .begin_upload(track(), "a.wav".into(), size as u64)
        .unwrap();
    let (status, _, _) = send(&state, put(&id, Body::from(vec![7u8; size]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let claimed = transfers.claim(&[id], is_track).unwrap();
    assert_eq!(
        std::fs::metadata(claimed[0].path().unwrap()).unwrap().len(),
        size as u64
    );
}

#[tokio::test]
async fn an_unknown_id_is_not_found() {
    let (state, _dir) = test_state();
    for id in ["0123456789abcdef0123456789abcdef", "..%2Fx", "nope"] {
        let (status, body, _) = send(&state, put(id, Body::from("x"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
        assert!(String::from_utf8_lossy(&body).contains("error"));
    }
    assert_eq!(staged_files(&state), 0);
}

#[tokio::test]
async fn a_second_put_to_the_same_id_is_a_conflict_and_keeps_the_first() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 3).unwrap();
    assert_eq!(
        send(&state, put(&id, Body::from("abc"))).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&state, put(&id, Body::from("xyz"))).await.0,
        StatusCode::CONFLICT
    );
    let claimed = transfers.claim(&[id], is_track).unwrap();
    assert_eq!(claimed[0].read().unwrap(), b"abc");
}

#[tokio::test]
async fn a_body_longer_than_declared_is_refused_and_discarded() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();

    // Known to be too long from the length it states.
    let id = transfers.begin_upload(track(), "a.wav".into(), 3).unwrap();
    let (status, _, _) = send(&state, put(&id, Body::from("abcd"))).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(staged_files(&state), 0);
    assert_eq!(
        transfers.claim(&[id], is_track).err(),
        Some(TransferError::NotFound),
        "the upload is spent"
    );

    // Found to be too long as it streams in.
    let id = transfers.begin_upload(track(), "a.wav".into(), 5).unwrap();
    let chunks = vec![b"abc".to_vec(), b"def".to_vec(), b"ghi".to_vec()];
    let (status, _, _) = send(&state, put(&id, streamed(chunks))).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(staged_files(&state), 0);
    assert_eq!(transfers.pending(), 0);
}

#[tokio::test]
async fn a_body_shorter_than_declared_is_taken_and_the_claim_refuses_it() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 10).unwrap();
    let (status, _, _) = send(&state, put(&id, Body::from("short"))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(matches!(
        transfers.claim(&[id], is_track),
        Err(TransferError::Unusable(_))
    ));
    assert_eq!(staged_files(&state), 0);
}

/// A client that goes away mid-body leaves nothing behind.
#[tokio::test]
async fn a_body_that_fails_midway_discards_the_upload() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 6).unwrap();
    let body = Body::from_stream(futures_util::stream::iter(vec![
        Ok(axum::body::Bytes::from_static(b"abc")),
        Err(std::io::Error::other("connection reset")),
    ]));
    let (status, _, _) = send(&state, put(&id, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(staged_files(&state), 0);
    assert_eq!(transfers.pending(), 0);
}

/// No lock check here: the id is the permission, and it was issued under
/// the lock policy.
#[tokio::test]
async fn a_put_is_not_refused_by_the_lock() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 1).unwrap();
    state.player.set_locked(true);
    assert_eq!(
        send(&state, put(&id, Body::from("x"))).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn the_staging_file_is_named_by_the_id_alone() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers
        .begin_upload(track(), "../../evil name.wav".into(), 1)
        .unwrap();
    send(&state, put(&id, Body::from("x"))).await;
    let names: Vec<String> = std::fs::read_dir(transfers.staging_dir().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![id]);
}

#[tokio::test]
async fn uploads_are_not_served_under_api() {
    let (state, _dir) = test_state();
    let id = state
        .player
        .transfers()
        .begin_upload(track(), "a.wav".into(), 1)
        .unwrap();
    let app = axum::Router::new()
        .nest("/api", crate::webui::api::guarded_router(&state))
        .merge(router())
        .with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/uploads/{id}"))
                .body(Body::from("x"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(state.player.transfers().pending(), 1);
}

#[tokio::test]
async fn a_download_is_an_attachment_fetched_repeatedly() {
    let (state, _dir) = test_state();
    let id = state
        .player
        .transfers()
        .stage_download("kellys.mvr".to_string(), b"the archive")
        .unwrap();
    for _ in 0..3 {
        let (status, body, headers) = send(
            &state,
            Request::get(format!("/downloads/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"the archive");
        assert_eq!(headers["content-type"], "application/octet-stream");
        assert_eq!(
            headers["content-disposition"],
            "attachment; filename=\"kellys.mvr\""
        );
    }
}

#[tokio::test]
async fn a_download_that_is_unknown_or_an_upload_is_not_found() {
    let (state, _dir) = test_state();
    let upload = state
        .player
        .transfers()
        .begin_upload(track(), "a.wav".into(), 1)
        .unwrap();
    for id in ["nope", "0123456789abcdef0123456789abcdef", upload.as_str()] {
        let (status, _, _) = send(
            &state,
            Request::get(format!("/downloads/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_download_expires_ten_minutes_after_it_was_staged() {
    let (state, _dir) = test_state();
    let id = state
        .player
        .transfers()
        .stage_download("a.mvr".to_string(), b"x")
        .unwrap();
    let get = || {
        Request::get(format!("/downloads/{id}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(send(&state, get()).await.0, StatusCode::OK);
    tokio::time::advance(std::time::Duration::from_secs(10 * 60)).await;
    assert_eq!(send(&state, get()).await.0, StatusCode::NOT_FOUND);
}

/// A body that stops is not a transfer: after a minute without a byte the
/// upload is dropped, and its place is free again.
#[tokio::test(start_paused = true)]
async fn a_body_that_stops_is_dropped_after_a_minute() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 6).unwrap();
    let body = Body::from_stream(
        futures_util::stream::once(async {
            Ok::<_, std::io::Error>(axum::body::Bytes::from("abc"))
        })
        .chain(futures_util::stream::pending()),
    );
    let request = tokio::spawn({
        let state = state.clone();
        async move { send(&state, put(&id, body)).await }
    });
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    let (status, _, _) = request.await.unwrap();
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(transfers.pending(), 0);
    assert_eq!(staged_files(&state), 0);
}

/// A client that says up front how long its body is has it refused at once
/// when that is more than announced, before a byte is read or staged.
#[tokio::test]
async fn a_length_stated_past_the_declared_size_is_refused_before_reading() {
    let (state, _dir) = test_state();
    let transfers = state.player.transfers();
    let id = transfers.begin_upload(track(), "a.wav".into(), 3).unwrap();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/uploads/{id}"))
        .header("content-length", "4000000000")
        .body(Body::from("a"))
        .unwrap();
    let (status, _, _) = send(&state, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(staged_files(&state), 0);
    assert_eq!(transfers.pending(), 0);
}
