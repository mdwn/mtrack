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
//! A hub for tests: the rig's protocol ([`super::protocol`]) answered from
//! memory, on a port of this machine. It keeps files by project and hash and
//! revisions in order, as the hub does, and counts what it is asked for.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use mtrack_core::project::{hex, Manifest};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::protocol::{
    ApiError, MissingRequest, MissingResponse, Named, NewRevision, Paired, PairingRequest,
    PairingStarted, ProjectState, Revision, RigReport, TokenRequest, Upload, API,
    AUTHORIZATION_PENDING, INVALID_PROJECT, NOT_CURRENT,
};

#[derive(Default)]
struct Project {
    name: String,
    revisions: Vec<(Revision, Manifest)>,
    blobs: HashMap<String, Vec<u8>>,
}

struct Pairing {
    user_code: String,
    rig_name: String,
    /// The project a person approved it for.
    project: Option<String>,
}

#[derive(Default)]
struct Inner {
    address: String,
    pairings: HashMap<String, Pairing>,
    /// Token to (rig, project).
    tokens: HashMap<String, (Named, String)>,
    projects: BTreeMap<String, Project>,
    /// The revision each rig last said it was on.
    reports: HashMap<String, Option<String>>,
    downloads: Vec<String>,
    uploads: Vec<String>,
    /// Hashes served with the wrong bytes.
    corrupt: HashSet<String>,
    /// The next revision is refused as a project that does not load.
    refuse_next: Option<String>,
    counter: u64,
}

impl Inner {
    fn next(&mut self, what: &str) -> String {
        self.counter += 1;
        format!("{what}-{}", self.counter)
    }
}

/// A running stand-in hub. It stops when dropped.
pub struct StandIn {
    /// Its address, as a rig is given one.
    pub url: String,
    inner: Arc<Mutex<Inner>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.server.abort();
    }
}

type Shared = State<Arc<Mutex<Inner>>>;

fn refuse(status: StatusCode, error: &str, message: Option<String>) -> Response {
    (
        status,
        Json(ApiError {
            error: error.to_string(),
            message,
        }),
    )
        .into_response()
}

/// The rig and project of the request's token, when it is one of ours.
fn rig_of(inner: &Inner, headers: &HeaderMap) -> Option<(Named, String)> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| inner.tokens.get(token).cloned())
}

fn unauthorized() -> Response {
    refuse(StatusCode::UNAUTHORIZED, "unauthorized", None)
}

async fn start_pairing(State(inner): Shared, Json(request): Json<PairingRequest>) -> Response {
    let mut inner = inner.lock();
    let device_code = inner.next("device");
    let user_code = format!("CODE-{}", inner.counter);
    inner.pairings.insert(
        device_code.clone(),
        Pairing {
            user_code: user_code.clone(),
            rig_name: request.rig_name,
            project: None,
        },
    );
    Json(PairingStarted {
        device_code,
        user_code,
        verification_uri: format!("{}/pair", inner.address),
        expires_in: 600,
        interval: 0,
    })
    .into_response()
}

async fn pairing_token(State(inner): Shared, Json(request): Json<TokenRequest>) -> Response {
    let mut inner = inner.lock();
    let Some(pairing) = inner.pairings.get(&request.device_code) else {
        return refuse(StatusCode::BAD_REQUEST, "expired_token", None);
    };
    let Some(project) = pairing.project.clone() else {
        return refuse(StatusCode::BAD_REQUEST, AUTHORIZATION_PENDING, None);
    };
    let rig_name = pairing.rig_name.clone();
    inner.pairings.remove(&request.device_code);
    let rig = Named {
        id: inner.next("rig"),
        name: rig_name,
    };
    let token = inner.next("token");
    inner
        .tokens
        .insert(token.clone(), (rig.clone(), project.clone()));
    let name = inner.projects[&project].name.clone();
    Json(Paired {
        token,
        rig,
        project: Named { id: project, name },
    })
    .into_response()
}

async fn project(State(inner): Shared, headers: HeaderMap) -> Response {
    let inner = inner.lock();
    let Some((_, id)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    let project = &inner.projects[&id];
    Json(ProjectState {
        project: Named {
            id: id.clone(),
            name: project.name.clone(),
        },
        revision: project
            .revisions
            .last()
            .map(|(revision, _)| revision.clone()),
    })
    .into_response()
}

async fn manifest(State(inner): Shared, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let inner = inner.lock();
    let Some((_, project)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    match inner.projects[&project]
        .revisions
        .iter()
        .find(|(revision, _)| revision.id == id)
    {
        Some((_, manifest)) => Json(manifest.clone()).into_response(),
        None => refuse(StatusCode::NOT_FOUND, "not_found", None),
    }
}

async fn missing(
    State(inner): Shared,
    headers: HeaderMap,
    Json(request): Json<MissingRequest>,
) -> Response {
    let inner = inner.lock();
    let Some((_, project)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    let held = &inner.projects[&project].blobs;
    Json(MissingResponse {
        missing: request
            .sha256
            .into_iter()
            .filter(|sha256| !held.contains_key(sha256))
            .map(|sha256| Upload {
                url: format!("{}{API}/blobs/{sha256}", inner.address),
                sha256,
                headers: BTreeMap::from([("x-standin".to_string(), "upload".to_string())]),
            })
            .collect(),
    })
    .into_response()
}

async fn put_blob(
    State(inner): Shared,
    headers: HeaderMap,
    Path(sha256): Path<String>,
    body: Bytes,
) -> Response {
    let mut inner = inner.lock();
    let Some((_, project)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    if headers.get("x-standin").is_none() {
        return refuse(StatusCode::BAD_REQUEST, "missing_header", None);
    }
    if hex(&Sha256::digest(&body)) != sha256 {
        return refuse(StatusCode::BAD_REQUEST, "hash_mismatch", None);
    }
    inner.uploads.push(sha256.clone());
    inner
        .projects
        .get_mut(&project)
        .expect("a paired project")
        .blobs
        .insert(sha256, body.to_vec());
    StatusCode::NO_CONTENT.into_response()
}

async fn get_blob(
    State(inner): Shared,
    headers: HeaderMap,
    Path(sha256): Path<String>,
) -> Response {
    let mut inner = inner.lock();
    let Some((_, project)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    let Some(bytes) = inner.projects[&project].blobs.get(&sha256).cloned() else {
        return refuse(StatusCode::NOT_FOUND, "not_found", None);
    };
    inner.downloads.push(sha256.clone());
    if inner.corrupt.contains(&sha256) {
        let mut wrong = bytes;
        wrong.reverse();
        wrong.push(b'!');
        return wrong.into_response();
    }
    bytes.into_response()
}

async fn create_revision(
    State(inner): Shared,
    headers: HeaderMap,
    Json(request): Json<NewRevision>,
) -> Response {
    let mut inner = inner.lock();
    let Some((_, id)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    if let Some(message) = inner.refuse_next.take() {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            INVALID_PROJECT,
            Some(message),
        );
    }
    let project = &inner.projects[&id];
    let current = project
        .revisions
        .last()
        .map(|(revision, _)| revision.id.clone());
    if request.base != current {
        return refuse(StatusCode::CONFLICT, NOT_CURRENT, None);
    }
    if let Err(problem) = request.manifest.validate() {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            INVALID_PROJECT,
            Some(problem.to_string()),
        );
    }
    if let Some(lacking) = request
        .manifest
        .files
        .values()
        .find(|entry| !project.blobs.contains_key(&entry.sha256))
    {
        return refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_blob",
            Some(lacking.sha256.clone()),
        );
    }
    let revision = Revision {
        id: inner.next("revision"),
        number: inner.projects[&id].revisions.len() as u64 + 1,
    };
    inner
        .projects
        .get_mut(&id)
        .expect("a paired project")
        .revisions
        .push((revision.clone(), request.manifest));
    (StatusCode::CREATED, Json(revision)).into_response()
}

async fn report(
    State(inner): Shared,
    headers: HeaderMap,
    Json(report): Json<RigReport>,
) -> Response {
    let mut inner = inner.lock();
    let Some((rig, _)) = rig_of(&inner, &headers) else {
        return unauthorized();
    };
    inner.reports.insert(rig.id, report.revision);
    StatusCode::NO_CONTENT.into_response()
}

impl StandIn {
    /// Starts a hub with one empty project, named `project`.
    pub async fn start(project: &str) -> StandIn {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mut inner = Inner {
            address: url.clone(),
            ..Inner::default()
        };
        inner.projects.insert(
            "project-1".to_string(),
            Project {
                name: project.to_string(),
                ..Project::default()
            },
        );
        let inner = Arc::new(Mutex::new(inner));
        let api = Router::new()
            .route("/pairings", post(start_pairing))
            .route("/pairings/token", post(pairing_token))
            .route("/project", get(project_route))
            .route("/revisions/{id}/manifest", get(manifest))
            .route("/blobs/missing", post(missing))
            .route("/blobs/{sha256}", put(put_blob).get(get_blob))
            .route("/revisions", post(create_revision))
            .route("/rig", put(report))
            .layer(axum::extract::DefaultBodyLimit::disable())
            .with_state(inner.clone());
        let app = Router::new().nest(API, api);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        StandIn { url, inner, server }
    }

    /// A person enters every code shown so far, for the hub's project.
    pub fn approve_all(&self) {
        for pairing in self.inner.lock().pairings.values_mut() {
            pairing.project = Some("project-1".to_string());
        }
    }

    /// The codes rigs are showing.
    pub fn codes(&self) -> Vec<String> {
        self.inner
            .lock()
            .pairings
            .values()
            .map(|pairing| pairing.user_code.clone())
            .collect()
    }

    /// The hashes of the files fetched from it, in order.
    pub fn downloads(&self) -> Vec<String> {
        self.inner.lock().downloads.clone()
    }

    /// The hashes of the files sent to it, in order.
    pub fn uploads(&self) -> Vec<String> {
        self.inner.lock().uploads.clone()
    }

    /// How many revisions the project has.
    pub fn revisions(&self) -> usize {
        self.inner.lock().projects["project-1"].revisions.len()
    }

    /// The revision the rig named `rig` last said it was on.
    pub fn reported(&self, rig: &str) -> Option<String> {
        let inner = self.inner.lock();
        let id = inner
            .tokens
            .values()
            .find(|(named, _)| named.name == rig)
            .map(|(named, _)| named.id.clone())?;
        inner.reports.get(&id).cloned().flatten()
    }

    /// Serves the file of this hash with the wrong bytes from now on.
    pub fn corrupt(&self, sha256: &str) {
        self.inner.lock().corrupt.insert(sha256.to_string());
    }

    /// Serves the file of this hash as it is again.
    pub fn uncorrupt(&self, sha256: &str) {
        self.inner.lock().corrupt.remove(sha256);
    }

    /// Refuses the next revision as a project that does not load.
    pub fn refuse_next(&self, message: &str) {
        self.inner.lock().refuse_next = Some(message.to_string());
    }

    /// Makes `manifest` the project's current revision as it stands, checked
    /// by nothing: what a hub gone wrong might send.
    pub fn force_revision(&self, manifest: Manifest, blobs: &[&[u8]]) {
        let mut inner = self.inner.lock();
        let revision = Revision {
            id: inner.next("revision"),
            number: inner.projects["project-1"].revisions.len() as u64 + 1,
        };
        let project = inner.projects.get_mut("project-1").unwrap();
        for blob in blobs {
            project
                .blobs
                .insert(hex(&Sha256::digest(blob)), blob.to_vec());
        }
        project.revisions.push((revision, manifest));
    }
}

use project as project_route;
