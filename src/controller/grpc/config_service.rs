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

//! `ConfigService`: what the editor pages use, over the same player and
//! config store as `PlayerService`. It is served on the gRPC port and by the
//! web UI's gRPC-Web, behind the same lock; Bluetooth does not carry it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::warn;

use crate::config;
use crate::player::{ConfigChange, Player};
use crate::proto::player::v1::{
    self as proto, config_service_server::ConfigService, CalibrationNoiseFloor, CalibrationResult,
    CancelCalibrationRequest, CancelCalibrationResponse, CaptureCalibrationRequest,
    CaptureCalibrationResponse, DeleteProfileFileRequest, GetProfileFileRequest,
    GetProfileFileResponse, ListAudioDevicesRequest, ListAudioDevicesResponse,
    ListMidiDevicesRequest, ListMidiDevicesResponse, ListProfileFilesRequest,
    ListProfileFilesResponse, ProbeAudioDeviceRequest, ProbeAudioDeviceResponse, ProfileFileSaved,
    RestartControllersRequest, RestartControllersResponse, SaveProfileFileRequest,
    StartCalibrationRequest, StopCalibrationRequest, UpdateConfigResponse, UpdateMetronomeRequest,
    UpdateSamplesRequest,
};
use crate::webui::api::bluetooth::{self, BluetoothError};
use crate::webui::api::devices::{self, CalibrateStart, DeviceError};
use crate::webui::api::profiles::{self, ProfileFileError, ProfileSave};
use crate::webui::api::status;
use crate::webui::wire::devices::{ProbeAudioRequest, ProbeKind};

/// The metadata key of an `ABORTED` save of a profile file that says the
/// version the file is at.
pub const CURRENT_VERSION_KEY: &str = "x-current-version";

/// The editor service over a player.
pub(crate) struct ConfigServer {
    player: Arc<Player>,
}

impl ConfigServer {
    pub(crate) fn new(player: Arc<Player>) -> Self {
        Self { player }
    }

    /// The player's config store, or the refusal of a player that has none.
    #[allow(clippy::result_large_err)]
    fn store(&self) -> Result<Arc<config::ConfigStore>, Status> {
        self.player
            .config_store()
            .ok_or_else(|| Status::unimplemented("config store not available"))
    }

    /// The profiles directory the config sets, and the project it is in. A
    /// config that sets none (or a player with no config store, which has
    /// none to read) has no profiles directory: the caller's request needs
    /// the config set up first, so `FAILED_PRECONDITION`.
    #[allow(clippy::result_large_err)]
    async fn profiles_location(&self) -> Result<(PathBuf, PathBuf), Status> {
        let none = || Status::failed_precondition("No profiles directory configured");
        let store = self.player.config_store().ok_or_else(none)?;
        let dir = store
            .read_config()
            .await
            .resolved_profiles_dir(store.path())
            .ok_or_else(none)?;
        let keep = |path: PathBuf| path.canonicalize().unwrap_or(path);
        Ok((keep(dir), keep(crate::util::project_dir_of(store.path()))))
    }
}

#[tonic::async_trait]
impl ConfigService for ConfigServer {
    async fn update_samples(
        &self,
        request: Request<UpdateSamplesRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let req = request.into_inner();
        if req.samples_json.trim().is_empty() {
            return Err(Status::invalid_argument("missing samples"));
        }
        let samples: HashMap<String, config::SampleDefinition> =
            serde_json::from_str(&req.samples_json)
                .map_err(|e| Status::invalid_argument(format!("invalid samples: {e}")))?;

        let store = self.store()?;
        let snapshot = store
            .update_samples(samples, req.max_sample_voices, &req.expected_checksum)
            .await
            .map_err(super::config_error_to_status)?;
        let outcome = self.player.apply_config_change().await;
        super::snapshot_to_update_response(snapshot, outcome)
    }

    async fn update_metronome(
        &self,
        request: Request<UpdateMetronomeRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let req = request.into_inner();
        // Empty or `null` removes the defaults.
        let metronome: Option<config::MetronomeDefaults> = if req.metronome_json.trim().is_empty() {
            None
        } else {
            serde_json::from_str(&req.metronome_json)
                .map_err(|e| Status::invalid_argument(format!("invalid metronome defaults: {e}")))?
        };
        if let Some(defaults) = metronome.as_ref() {
            defaults.validate().map_err(Status::invalid_argument)?;
        }

        let store = self.store()?;
        let default_enabled = metronome.as_ref().is_some_and(|m| m.enabled);
        let snapshot = store
            .update_metronome(metronome, &req.expected_checksum)
            .await
            .map_err(super::config_error_to_status)?;
        let mut outcome = self.player.apply_config_change().await;
        // The default-on flag is applied at song load; refresh both. A reload
        // that fails is answered with the save, its new checksum included,
        // and the reason beside the hardware's outcome, as any save's failed
        // reload is: an error would leave the caller the old checksum of a
        // config already saved.
        self.player.set_default_metronome(default_enabled);
        if let Err(e) = self.player.reload_songs().await {
            warn!("Reloading songs after a metronome change failed: {}", e);
            outcome.songs_error = Some(e.to_string());
        }
        super::snapshot_to_update_response(snapshot, outcome)
    }

    async fn list_profile_files(
        &self,
        _: Request<ListProfileFilesRequest>,
    ) -> Result<Response<ListProfileFilesResponse>, Status> {
        let (dir, _) = self.profiles_location().await?;
        let files = profiles::list(&dir).await.map_err(profile_file_status)?;
        Ok(Response::new(ListProfileFilesResponse {
            files: files.into_iter().map(info_to_proto).collect(),
        }))
    }

    async fn get_profile_file(
        &self,
        request: Request<GetProfileFileRequest>,
    ) -> Result<Response<GetProfileFileResponse>, Status> {
        let req = request.into_inner();
        let (dir, _) = self.profiles_location().await?;
        let file = profiles::read(&dir, &req.filename)
            .await
            .map_err(profile_file_status)?;
        Ok(Response::new(GetProfileFileResponse {
            profile_json: file.profile.to_string(),
            yaml: file.yaml,
            version: file.version,
        }))
    }

    async fn save_profile_file(
        &self,
        request: Request<SaveProfileFileRequest>,
    ) -> Result<Response<ProfileFileSaved>, Status> {
        let req = request.into_inner();
        let (dir, project) = self.profiles_location().await?;
        let body: serde_json::Value = serde_json::from_str(&req.profile_json)
            .map_err(|e| Status::invalid_argument(format!("Invalid profile: {e}")))?;
        let saved = profiles::save(
            &self.player,
            ProfileSave {
                profiles_dir: &dir,
                project: &project,
                filename: &req.filename,
                body,
                expected_version: req.expected_version.filter(|version| !version.is_empty()),
                create: req.create,
            },
        )
        .await
        .map_err(profile_file_status)?;
        Ok(Response::new(profile_file_saved(
            "saved",
            req.filename,
            Some(saved.version),
            saved.reload,
        )))
    }

    async fn delete_profile_file(
        &self,
        request: Request<DeleteProfileFileRequest>,
    ) -> Result<Response<ProfileFileSaved>, Status> {
        let req = request.into_inner();
        let (dir, _) = self.profiles_location().await?;
        let reload = profiles::delete(&self.player, &dir, &req.filename)
            .await
            .map_err(profile_file_status)?;
        Ok(Response::new(profile_file_saved(
            "deleted",
            req.filename,
            None,
            reload,
        )))
    }

    async fn list_audio_devices(
        &self,
        _: Request<ListAudioDevicesRequest>,
    ) -> Result<Response<ListAudioDevicesResponse>, Status> {
        let devices = devices::list_audio_devices().await.map_err(device_status)?;
        Ok(Response::new(ListAudioDevicesResponse {
            devices: devices
                .into_iter()
                .map(|d| proto::AudioDeviceInfo {
                    name: d.name,
                    max_channels: u32::from(d.max_channels),
                    host_name: d.host_name,
                    supported_sample_rates: d.supported_sample_rates,
                    supported_formats: d
                        .supported_formats
                        .into_iter()
                        .map(|f| proto::SupportedFormat {
                            sample_format: f.sample_format,
                            bits_per_sample: f.bits_per_sample,
                        })
                        .collect(),
                    virtual_node: d.virtual_node,
                    channels_known: d.channels_known,
                })
                .collect(),
        }))
    }

    async fn probe_audio_device(
        &self,
        request: Request<ProbeAudioDeviceRequest>,
    ) -> Result<Response<ProbeAudioDeviceResponse>, Status> {
        let req = request.into_inner();
        let bits_per_sample = req
            .bits_per_sample
            .map(|bits| {
                u16::try_from(bits)
                    .map_err(|_| Status::invalid_argument("bits_per_sample is out of range"))
            })
            .transpose()?;
        let buffer_size = req
            .buffer_size
            .map(|size| {
                usize::try_from(size)
                    .map_err(|_| Status::invalid_argument("buffer_size is out of range"))
            })
            .transpose()?;
        let stream_buffer_size = req.stream_buffer_size.map(stream_buffer_size).transpose()?;
        let probe = devices::probe_audio_device(
            &self.player,
            ProbeAudioRequest {
                device: req.device,
                sample_rate: req.sample_rate,
                sample_format: req.sample_format,
                bits_per_sample,
                buffer_size,
                stream_buffer_size,
            },
        )
        .await
        .map_err(device_status)?;
        Ok(Response::new(ProbeAudioDeviceResponse {
            outcome: match probe.outcome {
                ProbeKind::Streaming => proto::ProbeOutcome::Streaming,
                ProbeKind::OpenedButSilent => proto::ProbeOutcome::OpenedButSilent,
                ProbeKind::CouldNotOpen => proto::ProbeOutcome::CouldNotOpen,
                ProbeKind::OpenedCannotReport => proto::ProbeOutcome::OpenedCannotReport,
                ProbeKind::InUse => proto::ProbeOutcome::InUse,
            }
            .into(),
            ok: probe.ok,
            reason: probe.reason.flatten(),
            callbacks: probe.callbacks,
            health: probe
                .health
                .flatten()
                .as_ref()
                .map(super::watch::audio_output_to_proto),
        }))
    }

    async fn list_midi_devices(
        &self,
        _: Request<ListMidiDevicesRequest>,
    ) -> Result<Response<ListMidiDevicesResponse>, Status> {
        let devices = devices::list_midi_devices().await.map_err(device_status)?;
        Ok(Response::new(ListMidiDevicesResponse {
            devices: devices
                .into_iter()
                .map(|d| proto::MidiDeviceInfo {
                    name: d.name,
                    has_input: d.has_input,
                    has_output: d.has_output,
                })
                .collect(),
        }))
    }

    async fn start_calibration(
        &self,
        request: Request<StartCalibrationRequest>,
    ) -> Result<Response<CalibrationNoiseFloor>, Status> {
        let req = request.into_inner();
        let channel = u16::try_from(req.channel)
            .map_err(|_| Status::invalid_argument("channel is out of range"))?;
        let bits_per_sample = req
            .bits_per_sample
            .map(|bits| {
                u16::try_from(bits)
                    .map_err(|_| Status::invalid_argument("bits_per_sample is out of range"))
            })
            .transpose()?;
        let floor = devices::calibrate_start(CalibrateStart {
            device: req.device,
            channel,
            duration: req.duration.unwrap_or(devices::DEFAULT_NOISE_DURATION),
            sample_rate: req.sample_rate,
            sample_format: req.sample_format,
            bits_per_sample,
        })
        .await
        .map_err(device_status)?;
        Ok(Response::new(CalibrationNoiseFloor {
            peak: floor.peak,
            rms: floor.rms,
            low_freq_energy: floor.low_freq_energy,
            channel: u32::from(floor.channel),
            sample_rate: floor.sample_rate,
            device_channels: u32::from(floor.device_channels),
        }))
    }

    async fn capture_calibration(
        &self,
        _: Request<CaptureCalibrationRequest>,
    ) -> Result<Response<CaptureCalibrationResponse>, Status> {
        devices::calibrate_capture().map_err(device_status)?;
        Ok(Response::new(CaptureCalibrationResponse {}))
    }

    async fn stop_calibration(
        &self,
        _: Request<StopCalibrationRequest>,
    ) -> Result<Response<CalibrationResult>, Status> {
        let c = devices::calibrate_stop().map_err(device_status)?;
        Ok(Response::new(CalibrationResult {
            channel: u32::from(c.channel),
            threshold: c.threshold,
            gain: c.gain,
            scan_time_ms: c.scan_time_ms,
            retrigger_time_ms: c.retrigger_time_ms,
            highpass_freq: c.highpass_freq,
            dynamic_threshold_decay_ms: c.dynamic_threshold_decay_ms,
            num_hits_detected: c.num_hits_detected as u64,
            noise_floor_peak: c.noise_floor_peak,
            max_hit_amplitude: c.max_hit_amplitude,
        }))
    }

    async fn cancel_calibration(
        &self,
        _: Request<CancelCalibrationRequest>,
    ) -> Result<Response<CancelCalibrationResponse>, Status> {
        devices::calibrate_cancel();
        Ok(Response::new(CancelCalibrationResponse {}))
    }

    async fn restart_controllers(
        &self,
        _: Request<RestartControllersRequest>,
    ) -> Result<Response<RestartControllersResponse>, Status> {
        let controllers = status::restart_controllers(&self.player)
            .await
            .map_err(device_status)?;
        Ok(Response::new(RestartControllersResponse {
            controllers: controllers
                .iter()
                .map(super::watch::controller_to_proto)
                .collect(),
        }))
    }

    async fn get_song_config(
        &self,
        request: Request<proto::GetSongConfigRequest>,
    ) -> Result<Response<proto::GetSongConfigResponse>, Status> {
        super::config_songs::get_config(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn create_song(
        &self,
        request: Request<proto::CreateSongRequest>,
    ) -> Result<Response<proto::CreateSongResponse>, Status> {
        super::config_songs::create(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn save_song_config(
        &self,
        request: Request<proto::SaveSongConfigRequest>,
    ) -> Result<Response<proto::SaveSongConfigResponse>, Status> {
        super::config_songs::save(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn delete_song(
        &self,
        request: Request<proto::DeleteSongRequest>,
    ) -> Result<Response<proto::DeleteSongResponse>, Status> {
        super::config_songs::delete(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn get_song_waveform(
        &self,
        request: Request<proto::GetSongWaveformRequest>,
    ) -> Result<Response<proto::Waveform>, Status> {
        super::config_songs::waveform(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn guess_song_tempo(
        &self,
        request: Request<proto::GuessSongTempoRequest>,
    ) -> Result<Response<proto::GuessSongTempoResponse>, Status> {
        super::config_songs::guess_tempo(&self.player, request.into_inner()).map(Response::new)
    }

    async fn list_song_files(
        &self,
        request: Request<proto::ListSongFilesRequest>,
    ) -> Result<Response<proto::ListSongFilesResponse>, Status> {
        super::config_songs::list_files(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn import_song_file(
        &self,
        request: Request<proto::ImportSongFileRequest>,
    ) -> Result<Response<proto::ImportSongFileResponse>, Status> {
        let store = self.store()?;
        super::config_songs::import_file(&self.player, store.path(), request.into_inner())
            .await
            .map(Response::new)
    }

    async fn browse_directory(
        &self,
        request: Request<proto::BrowseDirectoryRequest>,
    ) -> Result<Response<proto::BrowseDirectoryResponse>, Status> {
        let store = self.store()?;
        super::config_songs::browse_directory(store.path(), request.into_inner()).map(Response::new)
    }

    async fn create_song_in_directory(
        &self,
        request: Request<proto::CreateSongInDirectoryRequest>,
    ) -> Result<Response<proto::CreateSongInDirectoryResponse>, Status> {
        let store = self.store()?;
        super::config_songs::create_in_directory(&self.player, store.path(), request.into_inner())
            .await
            .map(Response::new)
    }

    async fn bulk_import_songs(
        &self,
        request: Request<proto::BulkImportSongsRequest>,
    ) -> Result<Response<proto::BulkImportSongsResponse>, Status> {
        let store = self.store()?;
        super::config_songs::bulk_import(&self.player, store.path(), request.into_inner())
            .await
            .map(Response::new)
    }

    async fn begin_upload(
        &self,
        request: Request<proto::BeginUploadRequest>,
    ) -> Result<Response<proto::BeginUploadResponse>, Status> {
        let store = self.player.config_store();
        super::config_uploads::begin(
            &self.player,
            store.as_ref().map(|store| store.path()),
            request.into_inner(),
        )
        .map(Response::new)
    }

    async fn add_track_files(
        &self,
        request: Request<proto::AddTrackFilesRequest>,
    ) -> Result<Response<proto::AddTrackFilesResponse>, Status> {
        super::config_uploads::add_tracks(&self.player, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn add_sample_file(
        &self,
        request: Request<proto::AddSampleFileRequest>,
    ) -> Result<Response<proto::AddSampleFileResponse>, Status> {
        let store = self.store()?;
        super::config_uploads::add_sample(&self.player, store.path(), request.into_inner())
            .await
            .map(Response::new)
    }

    async fn get_bluetooth(
        &self,
        _: Request<proto::GetBluetoothRequest>,
    ) -> Result<Response<proto::BluetoothStatus>, Status> {
        Ok(Response::new(bluetooth_to_proto(
            bluetooth::status_of(&self.player).await,
        )))
    }

    async fn set_bluetooth_pairing(
        &self,
        request: Request<proto::SetBluetoothPairingRequest>,
    ) -> Result<Response<proto::BluetoothStatus>, Status> {
        let req = request.into_inner();
        bluetooth::set_pairing(&self.player, req.open, req.seconds)
            .await
            .map(|status| Response::new(bluetooth_to_proto(status)))
            .map_err(bluetooth_status)
    }

    async fn forget_bluetooth_device(
        &self,
        request: Request<proto::ForgetBluetoothDeviceRequest>,
    ) -> Result<Response<proto::BluetoothStatus>, Status> {
        bluetooth::forget(&self.player, &request.into_inner().address)
            .await
            .map(|status| Response::new(bluetooth_to_proto(status)))
            .map_err(bluetooth_status)
    }
}

/// A refusal of a pairing call as a status: Bluetooth not running is
/// `FAILED_PRECONDITION`, an address that is none `INVALID_ARGUMENT`, one
/// that is not a paired phone `NOT_FOUND`, a radio stack that refused
/// `UNAVAILABLE`. The message is the one the REST route gave.
fn bluetooth_status(e: BluetoothError) -> Status {
    let message = e.to_string();
    match e {
        BluetoothError::NotRunning(_) => Status::failed_precondition(message),
        BluetoothError::BadAddress(_) => Status::invalid_argument(message),
        BluetoothError::NotPaired(_) => Status::not_found(message),
        BluetoothError::Adapter(_) => Status::unavailable(message),
    }
}

fn bluetooth_to_proto(
    status: crate::webui::wire::bluetooth::BluetoothStatus,
) -> proto::BluetoothStatus {
    proto::BluetoothStatus {
        configured: status.configured,
        running: status.running,
        error: status.error,
        pairing: Some(proto::BluetoothPairingWindow {
            open: status.pairing.open,
            seconds_left: status.pairing.seconds_left,
        }),
        devices: status
            .devices
            .into_iter()
            .map(|p| proto::BluetoothPhone {
                address: p.address,
                name: p.name,
                connected: p.connected,
            })
            .collect(),
    }
}

/// A refusal of a device, calibration or restart call as a status: a request
/// that is not valid is `INVALID_ARGUMENT`, a device that is not there
/// `NOT_FOUND`, a call the player's state does not fit (playing, or no
/// calibration session) `FAILED_PRECONDITION`, a device that would not open
/// `UNAVAILABLE`. The message is the one the REST route gave.
fn device_status(e: DeviceError) -> Status {
    let message = e.to_string();
    match e {
        DeviceError::Invalid(_) => Status::invalid_argument(message),
        DeviceError::NotFound(_) => Status::not_found(message),
        DeviceError::WrongState(_) => Status::failed_precondition(message),
        DeviceError::Unavailable(_) => Status::unavailable(message),
        DeviceError::Internal(_) => Status::internal(message),
    }
}

/// A stream buffer size as the config holds it. A message that names
/// neither a kind nor a number is not valid.
#[allow(clippy::result_large_err)]
fn stream_buffer_size(size: proto::StreamBufferSize) -> Result<config::StreamBufferSize, Status> {
    use proto::stream_buffer_size::Size;
    match size.size {
        Some(Size::Kind(kind)) => match proto::StreamBufferKind::try_from(kind) {
            Ok(proto::StreamBufferKind::Default) => Ok(config::StreamBufferSize::Default),
            Ok(proto::StreamBufferKind::Min) => Ok(config::StreamBufferSize::Min),
            _ => Err(Status::invalid_argument("unknown stream_buffer_size kind")),
        },
        Some(Size::Frames(frames)) => usize::try_from(frames)
            .map(config::StreamBufferSize::Fixed)
            .map_err(|_| Status::invalid_argument("stream_buffer_size is out of range")),
        None => Err(Status::invalid_argument(
            "stream_buffer_size names neither a kind nor a number of frames",
        )),
    }
}

/// A refusal of a profile file call as a status. A save made against a
/// version the file is no longer at, or a create onto a file already there,
/// is `ABORTED`: someone else got there first. The version the file is at is
/// in the metadata beside the first.
fn profile_file_status(e: ProfileFileError) -> Status {
    let message = e.to_string();
    match e {
        ProfileFileError::Invalid(_) => Status::invalid_argument(message),
        ProfileFileError::NotFound(_) => Status::not_found(message),
        ProfileFileError::Forbidden(_) => Status::permission_denied(message),
        ProfileFileError::Changed { current, .. } => {
            let mut status = Status::aborted(message);
            if let Some(version) = current.and_then(|version| version.parse().ok()) {
                status.metadata_mut().insert(CURRENT_VERSION_KEY, version);
            }
            status
        }
        ProfileFileError::Exists(_) => Status::aborted(message),
        ProfileFileError::CaseClash(_) => Status::already_exists(message),
        ProfileFileError::Internal(_) => Status::internal(message),
    }
}

fn info_to_proto(
    info: crate::webui::wire::config::ProfileFileInfo,
) -> crate::proto::player::v1::ProfileFileInfo {
    crate::proto::player::v1::ProfileFileInfo {
        file: info.file,
        filename: info.filename,
        addressable: info.addressable,
        error: info.error,
        hostname: info.hostname,
        has_audio: info.has_audio,
        has_midi: info.has_midi,
        has_dmx: info.has_dmx,
        has_trigger: info.has_trigger,
        has_controllers: info.has_controllers,
    }
}

/// A profile file written or deleted, with what became of the hardware
/// reload, as an `UpdateConfigResponse` says it.
fn profile_file_saved(
    status: &str,
    filename: String,
    version: Option<String>,
    outcome: ConfigChange,
) -> ProfileFileSaved {
    ProfileFileSaved {
        status: status.to_string(),
        filename,
        version,
        applied: outcome.applied(),
        message: outcome.message().unwrap_or_default(),
        reload: outcome.kind().to_string(),
        songs_error: outcome.songs_error.clone(),
        awaits_reload: outcome.awaits,
        awaits_process: outcome
            .awaits
            .map(|_| crate::player::process_id().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;
    use std::sync::Arc;

    use tonic::{Code, Request};

    use super::{ConfigServer, CURRENT_VERSION_KEY};
    use crate::config;
    use crate::player::Player;
    use crate::proto::player::v1::{
        config_service_server::ConfigService, DeleteProfileFileRequest, GetProfileFileRequest,
        ListProfileFilesRequest, ProfileFileInfo, ProfileFileSaved, SaveProfileFileRequest,
        UpdateConfigResponse, UpdateMetronomeRequest, UpdateSamplesRequest,
    };
    use crate::testutil::{eventually, player::assets_player};

    type Fallible<T = ()> = Result<T, Box<dyn Error>>;

    const HOST_A: &str =
        "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n";

    /// A project on disk, a config store over its config file, a player
    /// that has the store, and the service over it.
    struct Served {
        dir: tempfile::TempDir,
        player: Arc<Player>,
        store: Arc<config::ConfigStore>,
        server: ConfigServer,
    }

    impl Served {
        /// The profiles directory, which the config sets.
        fn profiles(&self) -> std::path::PathBuf {
            self.dir.path().join("profiles")
        }

        /// A file of the profiles directory.
        fn file(&self, name: &str) -> std::path::PathBuf {
            self.profiles().join(name)
        }

        fn write(&self, name: &str, content: &str) {
            std::fs::write(self.file(name), content).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.file(name)).unwrap()
        }

        /// The config's checksum: the one a save presents.
        async fn checksum(&self) -> String {
            self.store.read_yaml().await.unwrap().1
        }

        async fn save(&self, filename: &str, profile: serde_json::Value) -> SaveResult {
            self.save_with(filename, profile, None, false).await
        }

        async fn save_with(
            &self,
            filename: &str,
            profile: serde_json::Value,
            expected_version: Option<&str>,
            create: bool,
        ) -> SaveResult {
            self.server
                .save_profile_file(Request::new(SaveProfileFileRequest {
                    filename: filename.to_string(),
                    profile_json: profile.to_string(),
                    expected_version: expected_version.map(str::to_string),
                    create,
                }))
                .await
                .map(|r| r.into_inner())
        }

        async fn get(
            &self,
            filename: &str,
        ) -> Result<(serde_json::Value, String, String), tonic::Status> {
            let got = self
                .server
                .get_profile_file(Request::new(GetProfileFileRequest {
                    filename: filename.to_string(),
                }))
                .await?
                .into_inner();
            Ok((
                serde_json::from_str(&got.profile_json).unwrap(),
                got.yaml,
                got.version,
            ))
        }

        async fn list(&self) -> Result<Vec<ProfileFileInfo>, tonic::Status> {
            Ok(self
                .server
                .list_profile_files(Request::new(ListProfileFilesRequest {}))
                .await?
                .into_inner()
                .files)
        }

        async fn delete(&self, filename: &str) -> Result<ProfileFileSaved, tonic::Status> {
            Ok(self
                .server
                .delete_profile_file(Request::new(DeleteProfileFileRequest {
                    filename: filename.to_string(),
                }))
                .await?
                .into_inner())
        }

        async fn update_samples(
            &self,
            samples_json: &str,
            checksum: String,
        ) -> Result<UpdateConfigResponse, tonic::Status> {
            Ok(self
                .server
                .update_samples(Request::new(UpdateSamplesRequest {
                    samples_json: samples_json.to_string(),
                    max_sample_voices: None,
                    expected_checksum: checksum,
                }))
                .await?
                .into_inner())
        }
    }

    type SaveResult = Result<ProfileFileSaved, tonic::Status>;

    /// A served project whose config sets `profiles_dir: profiles` (made,
    /// and empty) over `player`, which gets the store and is unlocked.
    async fn served_on(player: Arc<Player>, config: &str) -> Fallible<Served> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join("profiles"))?;
        let path = dir.path().join("mtrack.yaml");
        std::fs::write(&path, config)?;
        let store = Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path,
        ));
        player.set_config_store(store.clone());
        player.set_locked(false);
        Ok(Served {
            dir,
            server: ConfigServer::new(player.clone()),
            player,
            store,
        })
    }

    /// A served project on a player with no devices.
    async fn served() -> Fallible<Served> {
        let player = assets_player().audio(None).ready().await?;
        served_on(player, "songs: assets/songs\nprofiles_dir: profiles\n").await
    }

    /// A served project on a player with a mock audio device.
    async fn served_with_audio(playlist: Option<&str>) -> Fallible<Served> {
        let mut builder = assets_player();
        if let Some(yaml) = playlist {
            builder = builder.playlist_yaml(yaml);
        }
        served_on(
            builder.ready().await?,
            "songs: assets/songs\nprofiles_dir: profiles\n",
        )
        .await
    }

    fn audio(device: &str) -> serde_json::Value {
        serde_json::json!({"audio": {"device": device, "track_mappings": {"drums": [1]}}})
    }

    fn host_a(device: &str) -> serde_json::Value {
        serde_json::json!({
            "hostname": "host-a",
            "audio": {"device": device, "track_mappings": {"drums": [1]}}
        })
    }

    // ---- UpdateSamples ----

    /// `PUT /api/config/samples` updated the definitions: the answer is the
    /// config after it, with the checksum the next save presents.
    #[tokio::test(flavor = "multi_thread")]
    async fn samples_are_replaced_and_answered_with_the_config() -> Fallible {
        let served = served().await?;
        let before = served.checksum().await;
        let saved = served
            .update_samples(
                r#"{"kick": {"file": "samples/kick.wav"},
                    "snare": {"file": "samples/snare.wav", "retrigger": "polyphonic"}}"#,
                before.clone(),
            )
            .await?;
        assert!(saved.yaml.contains("kick") && saved.yaml.contains("snare"));
        assert_ne!(saved.checksum, before);
        assert_eq!(saved.yaml, served.store.read_yaml().await?.0);
        assert!(saved.applied);
        assert_eq!(saved.reload, "applied");

        // Replaced, not merged, and with the voice limit beside them.
        let limit = served
            .server
            .update_samples(Request::new(UpdateSamplesRequest {
                samples_json: r#"{"hat": {"file": "samples/hat.wav"}}"#.to_string(),
                max_sample_voices: Some(8),
                expected_checksum: saved.checksum,
            }))
            .await?
            .into_inner();
        assert!(limit.yaml.contains("hat") && !limit.yaml.contains("kick"));
        assert!(
            limit.yaml.contains("max_sample_voices: 8"),
            "{}",
            limit.yaml
        );
        Ok(())
    }

    /// A save that sends no checksum is as stale as a wrong one (it was a 400
    /// "missing expected_checksum"); samples that are missing or do not parse
    /// are the caller's to fix. None of them changes the config.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_samples_save_that_is_stale_or_not_valid_is_refused() -> Fallible {
        let served = served().await?;
        let before = served.store.read_yaml().await?;

        let stale = served
            .update_samples("{}", String::new())
            .await
            .unwrap_err();
        assert_eq!(stale.code(), Code::FailedPrecondition);
        let stale = served
            .update_samples("{}", "wrong-checksum".to_string())
            .await
            .unwrap_err();
        assert_eq!(stale.code(), Code::FailedPrecondition);

        let missing = served
            .update_samples("", before.1.clone())
            .await
            .unwrap_err();
        assert_eq!(missing.code(), Code::InvalidArgument);
        assert!(missing.message().contains("missing samples"), "{missing}");
        for bad in ["not json", r#"{"kick": 3}"#, "[]"] {
            let e = served
                .update_samples(bad, before.1.clone())
                .await
                .unwrap_err();
            assert_eq!(e.code(), Code::InvalidArgument, "{bad}");
            assert!(e.message().contains("invalid samples"), "{e}");
        }
        assert_eq!(served.store.read_yaml().await?, before);
        Ok(())
    }

    /// A player with no config store has nothing to save to (it was a 503).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_without_a_config_store_is_unavailable() -> Fallible {
        let player = assets_player().audio(None).ready().await?;
        let server = ConfigServer::new(player);
        let e = server
            .update_samples(Request::new(UpdateSamplesRequest {
                samples_json: "{}".to_string(),
                max_sample_voices: None,
                expected_checksum: "abc".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::Unimplemented);
        let e = server
            .list_profile_files(Request::new(ListProfileFilesRequest {}))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::FailedPrecondition);
        Ok(())
    }

    /// A save in an auto play's gap reset the hardware, and the song failed to
    /// start when the gap ended (#578): the gap counts as playing. The save
    /// is made, and its reload waits for the song; the reply says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_while_an_auto_play_waits_is_saved_and_waits() -> Fallible {
        let served =
            served_with_audio(Some("songs:\n- Song 1\n- song: Song 3\n  auto_play: 10s\n")).await?;
        served.player.next().await;
        assert!(served.player.auto_play_status().is_some(), "waiting");

        let saved = served.update_samples("{}", served.checksum().await).await?;
        assert!(!saved.applied, "{saved:?}");
        assert!(saved.message.contains("idle"), "{}", saved.message);
        assert_eq!(saved.reload, "deferred");
        assert!(
            served.player.hardware_status().init_done,
            "the hardware was reset in the gap"
        );
        assert!(served.player.cancel_waiting_auto_play(), "still waiting");
        Ok(())
    }

    /// A paused song is a show in progress, which a reload waits for, as for a
    /// playing one. The save is made, the reply says the reload waits (for
    /// the generation a client clears its notice by), and it runs once the
    /// song is stopped; a play landing before a save's reload is no different
    /// (#578).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_while_a_song_is_paused_is_saved_and_applied_once_stopped() -> Fallible {
        let served = served_with_audio(None).await?;
        let device = served.player.audio_device().expect("device").to_mock()?;
        // Held paused, which counts as playing: the test song is short, and a
        // slow runner reached the save after it had ended. A pause landing
        // after the end finds nothing; start again.
        loop {
            served.player.play().await?;
            eventually(|| device.is_playing(), "Song never started playing");
            if served.player.pause().await?.is_some() {
                break;
            }
        }

        let saved = served.update_samples("{}", served.checksum().await).await?;
        assert!(!saved.applied, "{saved:?}");
        assert_eq!(saved.awaits_reload, Some(1), "{saved:?}");
        assert_eq!(
            saved.awaits_process,
            Some(crate::player::process_id().to_string())
        );
        assert!(served.player.is_paused(), "the pause was kept");
        assert!(
            served.player.audio_device().is_some(),
            "the hardware was reset under the pause"
        );
        served.player.stop().await;
        eventually(
            || served.player.audio_device().is_none(),
            "the saved config was never applied",
        );
        Ok(())
    }

    /// A save restarted every controller after its reload had left the
    /// unchanged ones running: each OSC, gRPC and MCP client lost its
    /// connection over a change that was not its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_leaves_the_unchanged_controllers_running() -> Fallible {
        let served = served_with_audio(None).await?;
        let grpc = config::Controller::Grpc(config::GrpcController::new(0));
        let path = served.dir.path().join("mtrack.yaml");
        let store = Arc::new(config::ConfigStore::new(
            config::Player::new(
                vec![grpc.clone()],
                None,
                None,
                None,
                std::collections::HashMap::new(),
                "songs",
            ),
            path,
        ));
        served.player.set_config_store(store.clone());
        served.player.start_controllers(vec![grpc]);
        let tasks = served.player.controller_tasks();
        assert_eq!(tasks.len(), 1);

        let (_, checksum) = store.read_yaml().await?;
        let saved = served.update_samples("{}", checksum).await?;
        assert!(saved.applied, "{saved:?}");
        served.player.await_hardware_ready().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !tasks[0].is_finished(),
            "an unchanged controller was restarted by a save"
        );
        assert_eq!(served.player.controller_statuses()[0].status, "running");
        served.player.shutdown_controllers();
        Ok(())
    }

    /// A position cued by a seek while stopped read as a pause, and a save was
    /// refused as "playing or paused", though nothing plays and no lights are
    /// held.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_after_a_seek_while_stopped_is_not_refused() -> Fallible {
        let served = served_with_audio(None).await?;
        served
            .player
            .seek_to(std::time::Duration::from_millis(200))
            .await?;
        assert!(!served.player.is_playing().await);
        let saved = served.update_samples("{}", served.checksum().await).await?;
        assert!(saved.applied, "{saved:?}");
        Ok(())
    }

    // ---- UpdateMetronome ----

    /// The defaults are saved, `null` (or empty) removes them, defaults that
    /// do not parse or validate are refused, and a stale checksum is.
    #[tokio::test(flavor = "multi_thread")]
    async fn metronome_defaults_are_saved_removed_and_validated() -> Fallible {
        let served = served().await?;
        let update = |json: &str, checksum: String| {
            let request = UpdateMetronomeRequest {
                metronome_json: json.to_string(),
                expected_checksum: checksum,
            };
            let server = &served.server;
            async move {
                server
                    .update_metronome(Request::new(request))
                    .await
                    .map(|r| r.into_inner())
            }
        };

        let saved = update(
            r#"{"enabled": true, "volume": 0.5}"#,
            served.checksum().await,
        )
        .await?;
        assert!(saved.yaml.contains("metronome"), "{}", saved.yaml);
        assert!(saved.yaml.contains("volume: 0.5"), "{}", saved.yaml);
        assert_eq!(saved.yaml, served.store.read_yaml().await?.0);
        assert!(served.player.default_metronome(), "the default is on");

        let removed = update("null", saved.checksum.clone()).await?;
        assert!(!removed.yaml.contains("metronome"), "{}", removed.yaml);
        assert!(!served.player.default_metronome());
        let removed = update("", removed.checksum).await?;
        assert!(!removed.yaml.contains("metronome"), "{}", removed.yaml);

        let before = served.store.read_yaml().await?;
        let stale = update(r#"{"enabled": true}"#, "stale".to_string())
            .await
            .unwrap_err();
        assert_eq!(stale.code(), Code::FailedPrecondition);
        for bad in ["not json", r#"{"enabled": "yes"}"#, r#"{"volume": -1.0}"#] {
            let e = update(bad, before.1.clone()).await.unwrap_err();
            assert_eq!(e.code(), Code::InvalidArgument, "{bad}");
        }
        assert_eq!(served.store.read_yaml().await?, before);
        Ok(())
    }

    /// A metronome save whose song reload fails is still answered with the
    /// save and its new checksum, and the reason: an error left the UI the old
    /// checksum, and its next save was refused as stale.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_metronome_save_whose_song_reload_fails_answers_with_the_checksum() -> Fallible {
        let project = crate::testutil::player::playlist_project().await?;
        let server = ConfigServer::new(project.player.clone());
        let (_, checksum) = project.store.read_yaml().await?;
        // The songs directory gone from under the player.
        std::fs::remove_dir_all(project.dir.path().join("assets/songs"))?;
        let saved = server
            .update_metronome(Request::new(UpdateMetronomeRequest {
                metronome_json: r#"{"enabled": true}"#.to_string(),
                expected_checksum: checksum.clone(),
            }))
            .await?
            .into_inner();
        let (_, now) = project.store.read_yaml().await?;
        assert_eq!(saved.checksum, now);
        assert_ne!(now, checksum);
        assert!(!saved.applied);
        // The hardware's outcome is carried beside the songs', not hidden.
        assert_eq!(saved.reload, "applied");
        assert!(saved.songs_error.is_some(), "{saved:?}");
        Ok(())
    }

    // ---- ListProfileFiles / GetProfileFile ----

    #[tokio::test(flavor = "multi_thread")]
    async fn profile_files_are_listed_with_their_sections() -> Fallible {
        let served = served().await?;
        assert!(served.list().await?.is_empty(), "an empty directory");

        served.write(
            "01-host-a.yaml",
            "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n",
        );
        served.write(
            "02-host-b.yml",
            "hostname: host-b\nmidi:\n  device: midi-b\n",
        );
        let listed = served.list().await?;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].filename, "01-host-a");
        assert_eq!(listed[0].hostname.as_deref(), Some("host-a"));
        assert!(listed[0].has_audio && !listed[0].has_midi);
        assert_eq!(listed[1].filename, "02-host-b");
        assert_eq!(listed[1].hostname.as_deref(), Some("host-b"));
        assert!(listed[1].has_midi);
        Ok(())
    }

    /// A config that sets no profiles directory has none to list, read, save
    /// into or delete from (it was a 503): `FAILED_PRECONDITION`.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_profiles_directory_configured_is_a_failed_precondition() -> Fallible {
        let player = assets_player().audio(None).ready().await?;
        let served = served_on(player, "songs: assets/songs\n").await?;
        let code = |e: tonic::Status| (e.code(), e.message().to_string());
        let expected = (
            Code::FailedPrecondition,
            "No profiles directory configured".to_string(),
        );
        assert_eq!(code(served.list().await.unwrap_err()), expected);
        assert_eq!(code(served.get("a").await.unwrap_err()), expected);
        assert_eq!(
            code(served.save("a", audio("d")).await.unwrap_err()),
            expected
        );
        assert_eq!(code(served.delete("a").await.unwrap_err()), expected);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_profile_file_is_read_with_its_version() -> Fallible {
        let served = served().await?;
        served.write("host-a.yaml", HOST_A);
        let (profile, yaml, version) = served.get("host-a").await?;
        assert_eq!(profile["hostname"], "host-a");
        assert!(yaml.contains("host-a"));
        assert!(!version.is_empty());

        let missing = served.get("nonexistent").await.unwrap_err();
        assert_eq!(missing.code(), Code::NotFound);
        Ok(())
    }

    // ---- SaveProfileFile ----

    #[tokio::test(flavor = "multi_thread")]
    async fn a_profile_file_is_created() -> Fallible {
        let served = served().await?;
        let saved = served
            .save(
                "new-host",
                serde_json::json!({"hostname": "new-host", "audio": {"device": "dev-x", "track_mappings": {"drums": [1]}}}),
            )
            .await?;
        assert_eq!(saved.status, "saved");
        assert_eq!(saved.filename, "new-host");
        assert!(served.file("new-host.yaml").exists());
        assert_eq!(saved.version, Some(served.get("new-host").await?.2));
        Ok(())
    }

    /// Saving a profile file must reach the config the player reloads from.
    ///
    /// `Player::deserialize` copies `profiles_dir` files into the in-memory
    /// config at startup, and the hardware reload re-initialises from that
    /// copy. Writing the file alone left the save acknowledged, followed by a
    /// hardware reload that rebuilt everything from the profile as it was at
    /// boot -- so a trigger disabled through the editor stayed live until the
    /// process restarted, which is what #380 was reported as.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_saved_profile_file_updates_the_config_the_reload_reads() -> Fallible {
        let served = served().await?;
        served.write(
            "rig.yaml",
            "hostname: rig\naudio:\n  device: dev-x\n  track_mappings:\n    drums: [1]\n",
        );
        // So the store starts from what is on disk, the way startup does.
        served.store.reload_from_disk().await?;
        let loaded = served.store.read_config().await;
        assert_eq!(
            loaded.profile_list().unwrap_or_default().len(),
            1,
            "the profile file should be in the store to begin with"
        );
        drop(loaded);

        let saved = served
            .save(
                "rig",
                serde_json::json!({"hostname": "rig", "audio": {"device": "dev-changed", "track_mappings": {"drums": [1]}}}),
            )
            .await?;
        assert!(saved.applied, "{saved:?}");
        assert_eq!(saved.reload, "applied");

        // The store -- not just the file -- must reflect the edit, because
        // that is what the hardware reload reads.
        let after = served.store.read_config().await;
        let profiles = after.profile_list().unwrap_or_default();
        let device = profiles
            .first()
            .and_then(|p| p.audio_config())
            .map(|ac| ac.audio().device().to_string());
        assert_eq!(
            device.as_deref(),
            Some("dev-changed"),
            "the reload would re-initialise from the pre-save profile"
        );
        Ok(())
    }

    /// #523: a save that leaves sections out keeps them in the file; a
    /// section sent as `null` is removed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_keeps_omitted_sections_and_removes_null_ones() -> Fallible {
        let served = served().await?;
        served.write(
            "rig.yaml",
            "hostname: rig\naudio:\n  device: dev-x\n  track_mappings:\n    drums: [1]\ntrigger:\n  device: dev-x\n  inputs:\n    - kind: audio\n      channel: 12\n      sample: kick\nstatus_events:\n  off_events: []\n  idling_events: []\n  playing_events: []\n",
        );

        served
            .save(
                "rig",
                serde_json::json!({"controllers": [{"kind": "grpc"}]}),
            )
            .await?;
        let written = served.read("rig.yaml");
        assert!(written.contains("kind: grpc"), "{written}");
        assert!(
            written.contains("channel: 12"),
            "trigger dropped: {written}"
        );
        assert!(
            written.contains("off_events"),
            "status_events dropped: {written}"
        );
        assert!(written.contains("dev-x"), "audio dropped: {written}");

        served
            .save("rig", serde_json::json!({"trigger": null}))
            .await?;
        let written = served.read("rig.yaml");
        assert!(!written.contains("channel: 12"), "trigger kept: {written}");
        assert!(written.contains("kind: grpc"), "{written}");
        Ok(())
    }

    /// A profile that is not valid is the caller's to fix, and nothing is
    /// written.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_profile_that_is_not_valid_is_an_invalid_argument() -> Fallible {
        let served = served().await?;
        // Controllers should be an array, not a string.
        let e = served
            .save("bad", serde_json::json!({"controllers": "not-an-array"}))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::InvalidArgument);
        assert!(e.message().contains("Invalid profile"), "{e}");
        // A mapping to channel 0 (channels count from 1) fails the profile's
        // own validation.
        let e = served
            .save(
                "bad",
                serde_json::json!({"audio": {"device": "d", "track_mappings": {"kick": [0]}}}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::InvalidArgument);
        // Not JSON at all.
        let e = served
            .server
            .save_profile_file(Request::new(SaveProfileFileRequest {
                filename: "bad".to_string(),
                profile_json: "not json".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::InvalidArgument);
        assert!(!served.file("bad.yaml").exists());
        Ok(())
    }

    /// A save against the version the file was read at is made; one against
    /// a version since replaced is refused, `ABORTED` with the version the
    /// file is at in the metadata, and the file left as it was.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stale_profile_version_is_aborted_and_the_current_one_saves() -> Fallible {
        let served = served().await?;
        served.write("host-a.yaml", HOST_A);
        let (_, _, v1) = served.get("host-a").await?;

        let saved = served
            .save_with("host-a", host_a("dev-b"), Some(&v1), false)
            .await?;
        let v2 = saved.version.expect("the new version");
        assert_ne!(v1, v2);
        let after_first = served.read("host-a.yaml");
        assert!(after_first.contains("dev-b"));

        let e = served
            .save_with("host-a", host_a("dev-c"), Some(&v1), false)
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::Aborted);
        assert_eq!(
            e.metadata()
                .get(CURRENT_VERSION_KEY)
                .map(|v| v.to_str().unwrap()),
            Some(v2.as_str())
        );
        assert!(e.message().contains("changed since you loaded it"), "{e}");
        assert_eq!(served.read("host-a.yaml"), after_first);

        served
            .save_with("host-a", host_a("dev-c"), Some(&v2), false)
            .await?;

        // A version for a file that is not there is as stale, and says so by
        // sending none.
        let e = served
            .save_with("ghost", host_a("dev-c"), Some("abc"), false)
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::Aborted);
        assert!(e.metadata().get(CURRENT_VERSION_KEY).is_none());
        assert!(!served.file("ghost.yaml").exists());

        // An empty version is none: whatever the file holds is saved over.
        served
            .save_with("host-a", host_a("dev-d"), Some(""), false)
            .await?;
        assert!(served.read("host-a.yaml").contains("dev-d"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saving_a_profile_keeps_keys_the_editor_does_not_know() -> Fallible {
        let served = served().await?;
        served.write(
            "host-a.yaml",
            "hostname: host-a\nvenue_notes: front of house\naudio:\n  device: dev-a\n  house_tweak: 3\n  track_mappings:\n    drums: [1]\n",
        );
        served.save("host-a", host_a("dev-b")).await?;
        let text = served.read("host-a.yaml");
        assert!(text.contains("dev-b"), "{text}");
        assert!(text.contains("venue_notes"), "{text}");
        assert!(text.contains("house_tweak"), "{text}");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_yml_profile_is_saved_in_place() -> Fallible {
        let served = served().await?;
        served.write("host-a.yml", HOST_A);
        served.save("host-a", host_a("dev-b")).await?;
        assert!(!served.file("host-a.yaml").exists());
        assert!(served.read("host-a.yml").contains("dev-b"));
        Ok(())
    }

    /// Addressed by name, a profile's file is its `.yaml`, else its `.yml`:
    /// a read and a delete take the `.yaml` while both are there, then the
    /// `.yml`; a save keeps the `.yml`'s sections the body leaves out. The
    /// list shows every file the config loads, both of them here.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_profile_lookup_takes_the_names_file() -> Fallible {
        let served = served().await?;
        served.write("host-a.yaml", HOST_A);
        served.write(
            "host-a.yml",
            "hostname: host-a\naudio:\n  device: dev-y\n  track_mappings:\n    drums: [1]\n\
             midi:\n  device: midi-y\n",
        );
        assert_eq!(
            config::list_profile_files(&served.profiles())?,
            vec![served.file("host-a.yaml"), served.file("host-a.yml")]
        );
        let listed = served.list().await?;
        assert_eq!(listed.len(), 2);
        assert!(!listed[0].has_midi && listed[1].has_midi);

        assert!(served.get("host-a").await?.1.contains("dev-a"));
        served.delete("host-a").await?;
        assert!(!served.file("host-a.yaml").exists());
        assert!(served.file("host-a.yml").exists());
        assert!(served.get("host-a").await?.1.contains("dev-y"));

        served.save("host-a", host_a("dev-b")).await?;
        assert!(!served.file("host-a.yaml").exists());
        let saved = served.read("host-a.yml");
        assert!(saved.contains("dev-b"), "{saved}");
        assert!(saved.contains("midi-y"), "{saved}");
        Ok(())
    }

    /// Two files sharing a stem are each listed by their full file name, and
    /// each is read, saved and deleted by it: `stage.yml` is not taken for
    /// `stage.yaml`. A new profile named as a file there but for case
    /// (`Stage` beside `stage.yaml`) is refused, the file left alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn same_stem_profiles_are_addressed_by_their_file_names() -> Fallible {
        let served = served().await?;
        let profile = |device: &str| {
            format!("audio:\n  device: {device}\n  track_mappings:\n    drums: [1]\n")
        };
        served.write(
            "stage.yaml",
            &format!("hostname: pi-a\n{}", profile("dev-a")),
        );
        served.write("stage.yml", &profile("catch-all"));

        let listed: Vec<_> = served
            .list()
            .await?
            .into_iter()
            .map(|p| (p.file, p.filename))
            .collect();
        assert_eq!(
            listed,
            [
                ("stage.yaml".to_string(), "stage".to_string()),
                ("stage.yml".to_string(), "stage".to_string())
            ]
        );
        assert!(served.get("stage.yml").await?.1.contains("catch-all"));
        assert!(served.get("stage.yaml").await?.1.contains("dev-a"));

        served.save("stage.yml", audio("dev-y")).await?;
        assert!(served.read("stage.yml").contains("dev-y"));
        assert!(served.read("stage.yaml").contains("dev-a"));

        let clash = served.save("Stage", audio("dev-z")).await.unwrap_err();
        assert_eq!(clash.code(), Code::AlreadyExists);
        assert!(
            clash.message().contains("names differ only in case"),
            "{clash}"
        );
        assert!(served.read("stage.yaml").contains("dev-a"));
        assert!(!served.file("Stage.yaml").exists());

        served.delete("stage.yml").await?;
        assert!(!served.file("stage.yml").exists());
        assert!(served.file("stage.yaml").exists());
        Ok(())
    }

    /// A full file name not listed is tried as a stem (`stage.yaml` with only
    /// `stage.yaml.yaml` there reads that file); a save to `x.yml` creates
    /// `x.yml`. A file whose name is not UTF-8 is listed, but not as one the
    /// API can address.
    #[tokio::test(flavor = "multi_thread")]
    async fn profiles_by_full_name_stem_and_unaddressable() -> Fallible {
        let served = served().await?;
        served.write("stage.yaml.yaml", HOST_A);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::OsStr::from_bytes(b"caf\xe9.yaml");
            std::fs::write(served.profiles().join(name), HOST_A)?;
        }

        assert!(served.get("stage.yaml").await?.1.contains("dev-a"));
        assert!(!served.file("stage.yaml").exists());

        served.save("x.yml", audio("dev-x")).await?;
        assert!(served.read("x.yml").contains("dev-x"));
        assert!(!served.file("x.yml.yaml").exists());

        let addressable: Vec<_> = served
            .list()
            .await?
            .into_iter()
            .map(|p| (p.file, p.addressable))
            .collect();
        let mut expected = vec![
            ("stage.yaml.yaml".to_string(), true),
            ("x.yml".to_string(), true),
        ];
        if cfg!(target_os = "linux") {
            expected.insert(0, ("caf\u{fffd}.yaml".to_string(), false));
        }
        assert_eq!(addressable, expected);
        Ok(())
    }

    /// A create is only a create: `stage.yaml` with only `stage.yaml.yaml`
    /// there, or `stage` with only `stage.yml`, is refused (`ABORTED`) and the
    /// file there left alone; a name with no file is created. A file that
    /// does not parse is listed, with why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_create_never_rewrites_another_profile() -> Fallible {
        let served = served().await?;
        served.write("stage.yaml.yaml", HOST_A);
        served.write("solo.yml", HOST_A);

        for (name, file) in [("stage.yaml", "stage.yaml.yaml"), ("solo", "solo.yml")] {
            let e = served
                .save_with(name, audio("dev-new"), None, true)
                .await
                .unwrap_err();
            assert_eq!(e.code(), Code::Aborted, "{name}");
            assert!(
                e.message()
                    .contains(&format!("a profile file `{file}` already exists")),
                "{name}: {e}"
            );
            assert_eq!(served.read(file), HOST_A);
        }
        assert!(!served.file("stage.yaml").exists());

        served
            .save_with("fresh", audio("dev-new"), None, true)
            .await?;
        assert!(served.read("fresh.yaml").contains("dev-new"));

        // Written now: the running config is read back from the directory
        // after a save, and a file that does not parse would fail that.
        served.write("broken.yaml", "audio: [not, a, profile\n");
        let listed = served.list().await?;
        let broken = listed
            .iter()
            .find(|p| p.file == "broken.yaml")
            .expect("the broken file is listed");
        assert!(broken.error.is_some(), "{broken:?}");
        assert!(!broken.has_audio);
        Ok(())
    }

    /// A file made between a new profile's lookup and its write (another
    /// writer racing the save) is refused by the filesystem, `ABORTED`, and
    /// left as that writer made it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_create_beaten_to_its_file_is_aborted() -> Fallible {
        let served = served().await?;
        let racing = served.profiles().canonicalize()?.join("raced.yaml");
        *crate::webui::api::profiles::RACE.lock().unwrap() = Some((racing.clone(), "theirs\n"));
        let e = served.save("raced", audio("dev")).await.unwrap_err();
        assert_eq!(e.code(), Code::Aborted);
        assert!(
            e.message()
                .contains("a profile file `raced.yaml` already exists"),
            "{e}"
        );
        assert_eq!(std::fs::read_to_string(&racing)?, "theirs\n");
        Ok(())
    }

    // ---- DeleteProfileFile ----

    #[tokio::test(flavor = "multi_thread")]
    async fn a_profile_file_is_deleted_and_dropped_from_the_running_config() -> Fallible {
        let served = served().await?;
        served.write("host-a.yaml", HOST_A);
        served.store.reload_from_disk().await?;
        assert_eq!(
            served
                .store
                .read_config()
                .await
                .profile_list()
                .unwrap_or_default()
                .len(),
            1
        );

        let deleted = served.delete("host-a").await?;
        assert_eq!(deleted.status, "deleted");
        assert_eq!(deleted.filename, "host-a");
        assert_eq!(deleted.version, None);
        assert!(deleted.applied, "{deleted:?}");
        assert!(!served.file("host-a.yaml").exists());
        assert!(served
            .store
            .read_config()
            .await
            .profile_list()
            .unwrap_or_default()
            .is_empty());

        let e = served.delete("host-a").await.unwrap_err();
        assert_eq!(e.code(), Code::NotFound);
        Ok(())
    }

    /// A name that leaves the profiles directory is refused on every call,
    /// and nothing outside is read, written or deleted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_path_that_leaves_the_profiles_directory_is_refused() -> Fallible {
        let served = served().await?;
        let outside = served.dir.path().join("evil.yaml");
        std::fs::write(&outside, HOST_A)?;
        for name in ["../evil", "..", "a/b", "", "../evil.yaml"] {
            assert_eq!(
                served.get(name).await.unwrap_err().code(),
                Code::InvalidArgument,
                "{name:?}"
            );
            assert_eq!(
                served
                    .save(name, serde_json::json!({"hostname": "evil"}))
                    .await
                    .unwrap_err()
                    .code(),
                Code::InvalidArgument,
                "{name:?}"
            );
            assert_eq!(
                served.delete(name).await.unwrap_err().code(),
                Code::InvalidArgument,
                "{name:?}"
            );
        }
        assert_eq!(std::fs::read_to_string(&outside)?, HOST_A);
        assert!(!served.dir.path().join("evil.yaml.yaml").exists());
        assert!(Path::new(&outside).exists());
        Ok(())
    }

    // ---- The reply ----

    /// A profile file's reply says which reload it was, as an
    /// `UpdateConfigResponse` does: applied, deferred (with the generation it
    /// waits for and the process that counts it) or failed with the reason,
    /// and a song reload that failed beside it.
    #[test]
    fn a_profile_files_reply_says_what_became_of_it() {
        use crate::player::{ConfigApply, ConfigChange};
        let reply = |change: ConfigChange| {
            super::profile_file_saved("saved", "a".to_string(), Some("v".to_string()), change)
        };

        let applied = reply(ConfigApply::Applied.into());
        assert!(applied.applied);
        assert_eq!(applied.reload, "applied");
        assert!(applied.message.is_empty());
        assert_eq!(applied.version.as_deref(), Some("v"));
        assert_eq!(applied.awaits_reload, None);

        let mut deferred = ConfigChange::from(ConfigApply::Deferred);
        deferred.awaits = Some(3);
        deferred.songs_error = Some("no songs".to_string());
        let deferred = reply(deferred);
        assert!(!deferred.applied);
        assert_eq!(deferred.reload, "deferred");
        assert!(deferred.message.contains("idle"), "{}", deferred.message);
        assert_eq!(deferred.awaits_reload, Some(3));
        assert_eq!(
            deferred.awaits_process,
            Some(crate::player::process_id().to_string())
        );
        assert_eq!(deferred.songs_error.as_deref(), Some("no songs"));

        let failed = reply(ConfigApply::Failed("no store".into()).into());
        assert!(!failed.applied);
        assert_eq!(failed.reload, "failed");
        assert!(
            failed.message.contains("saved") && failed.message.contains("no store"),
            "{}",
            failed.message
        );
    }

    // ---- Devices, the audio probe, calibration and the controller restart ----

    use crate::proto::player::v1 as pb;

    fn probe_request(device: &str) -> pb::ProbeAudioDeviceRequest {
        pb::ProbeAudioDeviceRequest {
            device: device.to_string(),
            ..Default::default()
        }
    }

    /// `GET /api/devices/audio` answered a list, never a failure for a
    /// machine with no devices.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_audio_devices_are_listed() -> Fallible {
        let served = served().await?;
        served
            .server
            .list_audio_devices(Request::new(pb::ListAudioDevicesRequest {}))
            .await?;
        Ok(())
    }

    /// `GET /api/devices/midi` answered a list, and an empty one when the
    /// MIDI subsystem is unavailable (no ALSA on CI), not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_midi_devices_are_listed_or_none_when_midi_is_unavailable() -> Fallible {
        let served = served().await?;
        served
            .server
            .list_midi_devices(Request::new(pb::ListMidiDevicesRequest {}))
            .await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn probe_without_a_device_is_rejected() -> Fallible {
        let served = served().await?;
        let refused = served
            .server
            .probe_audio_device(Request::new(probe_request("   ")))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert_eq!(refused.message(), "device is required");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn probe_of_a_missing_device_reports_why_it_could_not_open() -> Fallible {
        let served = served().await?;
        let probe = served
            .server
            .probe_audio_device(Request::new(probe_request(
                "definitely-not-a-real-device-for-probing",
            )))
            .await?
            .into_inner();
        assert_eq!(probe.outcome(), pb::ProbeOutcome::CouldNotOpen);
        assert!(!probe.ok);
        // The reason is the whole point: "it failed" sends nobody anywhere.
        let reason = probe.reason.unwrap_or_default();
        assert!(
            reason.contains("definitely-not-a-real-device-for-probing"),
            "reason should name the device, got {reason:?}"
        );
        Ok(())
    }

    /// Probing the device the player already holds must not open it a second
    /// time. It would be refused, and reporting "could not be opened" for the
    /// device currently playing is the worst answer this call could give.
    #[tokio::test(flavor = "multi_thread")]
    async fn probe_of_the_held_device_reports_it_in_use_rather_than_opening_it() -> Fallible {
        let served = served_with_audio(None).await?;
        let probe = served
            .server
            .probe_audio_device(Request::new(probe_request("mock-device")))
            .await?
            .into_inner();
        assert_eq!(probe.outcome(), pb::ProbeOutcome::InUse);
        assert!(probe.ok);
        Ok(())
    }

    /// The guard is about *this* device, not about holding any device at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn probe_of_another_device_is_not_confused_by_the_held_one() -> Fallible {
        let served = served_with_audio(None).await?;
        let probe = served
            .server
            .probe_audio_device(Request::new(probe_request("some-other-device")))
            .await?
            .into_inner();
        assert_ne!(probe.outcome(), pb::ProbeOutcome::InUse);
        Ok(())
    }

    /// Format settings must reach the probe: a device that opens at one format
    /// and refuses another is the failure this call exists for, so probing at
    /// defaults would answer a question nobody asked.
    #[tokio::test(flavor = "multi_thread")]
    async fn probe_accepts_the_format_settings_beside_the_device() -> Fallible {
        let served = served().await?;
        for stream_buffer_size in [
            pb::stream_buffer_size::Size::Kind(pb::StreamBufferKind::Min.into()),
            pb::stream_buffer_size::Size::Kind(pb::StreamBufferKind::Default.into()),
            pb::stream_buffer_size::Size::Frames(1024),
        ] {
            let probe = served
                .server
                .probe_audio_device(Request::new(pb::ProbeAudioDeviceRequest {
                    device: "mock-probe-target".to_string(),
                    sample_rate: Some(48000),
                    sample_format: Some("int".to_string()),
                    bits_per_sample: Some(24),
                    buffer_size: Some(2048),
                    stream_buffer_size: Some(pb::StreamBufferSize {
                        size: Some(stream_buffer_size),
                    }),
                }))
                .await?
                .into_inner();
            // A mock opens but cannot report liveness.
            assert_eq!(probe.outcome(), pb::ProbeOutcome::OpenedCannotReport);
            assert!(probe.ok);
        }
        Ok(())
    }

    /// A stream buffer size that names nothing, or a setting out of range, is
    /// refused rather than probed at something else.
    #[tokio::test(flavor = "multi_thread")]
    async fn probe_refuses_settings_that_are_not_valid() -> Fallible {
        let served = served().await?;
        for request in [
            pb::ProbeAudioDeviceRequest {
                stream_buffer_size: Some(pb::StreamBufferSize { size: None }),
                ..probe_request("mock-probe-target")
            },
            pb::ProbeAudioDeviceRequest {
                stream_buffer_size: Some(pb::StreamBufferSize {
                    size: Some(pb::stream_buffer_size::Size::Kind(0)),
                }),
                ..probe_request("mock-probe-target")
            },
            pb::ProbeAudioDeviceRequest {
                bits_per_sample: Some(70_000),
                ..probe_request("mock-probe-target")
            },
        ] {
            let refused = served
                .server
                .probe_audio_device(Request::new(request))
                .await
                .unwrap_err();
            assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
        }
        Ok(())
    }

    /// A served project whose player is playing an hour-long song through a
    /// mock device, `mock-device`.
    async fn served_playing() -> Fallible<Served> {
        use crate::player::PlayerDevices;
        use crate::songs::{Song, Songs};
        let mut map = std::collections::HashMap::new();
        map.insert(
            "Song A".to_string(),
            Arc::new(Song::new_for_test_lasting(
                "Song A",
                &["kick", "snare"],
                std::time::Duration::from_secs(3600),
            )),
        );
        let mut playlists = std::collections::HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            crate::playlist::from_songs(Arc::new(Songs::new(map)))?,
        );
        let device = Arc::new(crate::audio::mock::Device::get("mock-device"));
        let player = Arc::new(Player::new_with_devices(
            PlayerDevices {
                audio: Some(device.clone()),
                mappings: Some(Arc::new(std::collections::HashMap::new())),
                ..crate::testutil::player::no_devices()
            },
            playlists,
            "all_songs".to_string(),
            None,
        )?);
        let served = served_on(player, "songs: assets/songs\nprofiles_dir: profiles\n").await?;
        served.player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        Ok(served)
    }

    /// Opening a second interface mid-song risks an xrun on the one that is
    /// playing, so another device is not probed during playback; the held one
    /// is still answered, from its health.
    #[tokio::test(flavor = "multi_thread")]
    async fn probe_of_another_device_is_refused_during_playback() -> Fallible {
        let served = served_playing().await?;
        let refused = served
            .server
            .probe_audio_device(Request::new(probe_request("some-other-device")))
            .await;
        let held = served
            .server
            .probe_audio_device(Request::new(probe_request("mock-device")))
            .await;
        // Stopped before the assertions: an hour-long song keeps the
        // runtime from shutting down.
        served.player.stop().await;
        let refused = refused.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        assert_eq!(
            refused.message(),
            "Cannot test another audio device during playback"
        );
        assert_eq!(held?.into_inner().outcome(), pb::ProbeOutcome::InUse);
        Ok(())
    }

    fn start_request(device: &str, channel: u32) -> pb::StartCalibrationRequest {
        pb::StartCalibrationRequest {
            device: device.to_string(),
            channel,
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn calibration_of_a_device_that_is_not_there_is_refused() -> Fallible {
        let served = served().await?;
        let refused = served
            .server
            .start_calibration(Request::new(start_request("nonexistent", 1)))
            .await
            .unwrap_err();
        // NOT_FOUND where the machine has no such input; a machine whose
        // audio host cannot list any may answer otherwise.
        assert!(
            matches!(
                refused.code(),
                Code::NotFound | Code::InvalidArgument | Code::Unavailable
            ),
            "{refused:?}"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn calibration_needs_a_channel_from_one() -> Fallible {
        let served = served().await?;
        // Channel 0 is invalid, and so is the channel an unset field reads as.
        for channel in [0, 70_000] {
            let refused = served
                .server
                .start_calibration(Request::new(start_request("nonexistent", channel)))
                .await
                .unwrap_err();
            assert_eq!(refused.code(), Code::InvalidArgument, "{channel}");
        }
        let refused = served
            .server
            .start_calibration(Request::new(start_request("nonexistent", 0)))
            .await
            .unwrap_err();
        assert_eq!(refused.message(), "channel must be >= 1");
        Ok(())
    }

    /// The calibration session is the player's one, whichever service
    /// instance a call lands on, so these walk a session that is not there.
    #[tokio::test(flavor = "multi_thread")]
    async fn calibration_calls_without_a_session_are_refused_or_nothing_to_cancel() -> Fallible {
        let served = served().await?;
        let capture = served
            .server
            .capture_calibration(Request::new(pb::CaptureCalibrationRequest {}))
            .await
            .unwrap_err();
        assert_eq!(capture.code(), Code::FailedPrecondition);
        assert!(
            capture.message().contains("No calibration session"),
            "{capture:?}"
        );
        let stop = served
            .server
            .stop_calibration(Request::new(pb::StopCalibrationRequest {}))
            .await
            .unwrap_err();
        assert_eq!(stop.code(), Code::FailedPrecondition);
        assert!(
            stop.message().contains("No calibration session"),
            "{stop:?}"
        );
        // Nothing to cancel is not an error.
        served
            .server
            .cancel_calibration(Request::new(pb::CancelCalibrationRequest {}))
            .await?;
        Ok(())
    }

    /// The restart answers the controllers it started, and leaves them
    /// running.
    #[tokio::test(flavor = "multi_thread")]
    async fn restarting_the_controllers_answers_their_status() -> Fallible {
        let served = served_with_audio(None).await?;
        let grpc = config::Controller::Grpc(config::GrpcController::new(0));
        let path = served.dir.path().join("mtrack.yaml");
        let store = Arc::new(config::ConfigStore::new(
            config::Player::new(
                vec![grpc.clone()],
                None,
                None,
                None,
                std::collections::HashMap::new(),
                "songs",
            ),
            path,
        ));
        served.player.set_config_store(store);
        served.player.start_controllers(vec![grpc]);
        let restarted = served
            .server
            .restart_controllers(Request::new(pb::RestartControllersRequest {}))
            .await?
            .into_inner();
        served.player.await_hardware_ready().await;
        assert_eq!(restarted.controllers.len(), 1, "{restarted:?}");
        assert_eq!(restarted.controllers[0].kind, "grpc");
        served.player.shutdown_controllers();
        Ok(())
    }

    /// Restarting the controllers would disrupt the control surfaces in use,
    /// so it is refused during playback.
    #[tokio::test(flavor = "multi_thread")]
    async fn restarting_the_controllers_is_refused_during_playback() -> Fallible {
        let served = served_playing().await?;
        let refused = served
            .server
            .restart_controllers(Request::new(pb::RestartControllersRequest {}))
            .await;
        // Stopped before the assertions: an hour-long song keeps the
        // runtime from shutting down.
        served.player.stop().await;
        let refused = refused.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        assert_eq!(
            refused.message(),
            "Cannot restart controllers during playback"
        );
        Ok(())
    }

    /// While locked the device lists and the restart are served, and the
    /// probe and the calibration are refused (the table in lock_policy; this
    /// is the service behind it, as the web UI reaches it).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_serves_the_lists_and_the_restart_but_not_the_probe() -> Fallible {
        use tower::ServiceExt;
        let served = served().await?;
        served.player.set_locked(true);
        let code_of = |method: &'static str| {
            let player = served.player.clone();
            async move {
                let request = http::Request::post(format!("/player.v1.ConfigService/{method}"))
                    .header("content-type", "application/grpc")
                    .body(tonic::body::Body::empty())
                    .unwrap();
                let response = crate::controller::grpc::config_service(player)
                    .oneshot(request)
                    .await
                    .unwrap();
                tonic::Status::from_header_map(response.headers())
                    .map(|s| (s.code(), s.message().to_string()))
            }
        };
        let locked = crate::player::LockedError.to_string();
        for method in [
            "ProbeAudioDevice",
            "StartCalibration",
            "CaptureCalibration",
            "StopCalibration",
            "CancelCalibration",
        ] {
            let (code, message) = code_of(method).await.expect(method);
            assert_eq!(code, Code::FailedPrecondition, "{method}");
            assert_eq!(message, locked, "{method}");
        }
        for method in ["ListAudioDevices", "ListMidiDevices", "RestartControllers"] {
            // Past the guard: whatever it answers, it is not the lock.
            if let Some((_, message)) = code_of(method).await {
                assert_ne!(message, locked, "{method}");
            }
        }
        Ok(())
    }

    // ---- Bluetooth pairing ----

    /// A controller that does nothing but hold a Bluetooth handle.
    struct HandleOnly(crate::controller::ble::BleHandle);

    impl crate::controller::Driver for HandleOnly {
        fn monitor_events(&self) -> tokio::task::JoinHandle<Result<(), std::io::Error>> {
            tokio::spawn(std::future::pending())
        }
        fn ble_handle(&self) -> Option<crate::controller::ble::BleHandle> {
            Some(self.0.clone())
        }
    }

    /// A served project whose player runs a Bluetooth controller; when
    /// `attached`, over a fake radio holding a phone, a connected phone and
    /// a pedal the owner trusts.
    async fn with_bluetooth(
        attached: bool,
    ) -> Fallible<(Served, Arc<crate::controller::ble::fake::Fake>)> {
        use crate::controller::ble::fake::{device, started, trusted_device};
        let served = served().await?;
        let handle = crate::controller::ble::BleHandle::default();
        let (fake, pairing) = started(vec![
            device(1, true, true),
            device(2, true, false),
            trusted_device(3, true),
        ])
        .await;
        pairing.close().await.unwrap();
        if attached {
            handle.attach(pairing);
        }
        served
            .player
            .install_controller(crate::controller::Controller::new_from_drivers(vec![
                Arc::new(HandleOnly(handle)),
            ]));
        Ok((served, fake))
    }

    async fn bluetooth(served: &Served) -> Result<pb::BluetoothStatus, tonic::Status> {
        Ok(served
            .server
            .get_bluetooth(Request::new(pb::GetBluetoothRequest {}))
            .await?
            .into_inner())
    }

    async fn pairing(
        served: &Served,
        open: bool,
        seconds: Option<u32>,
    ) -> Result<pb::BluetoothStatus, tonic::Status> {
        Ok(served
            .server
            .set_bluetooth_pairing(Request::new(pb::SetBluetoothPairingRequest {
                open,
                seconds,
            }))
            .await?
            .into_inner())
    }

    async fn forget(served: &Served, address: &str) -> Result<pb::BluetoothStatus, tonic::Status> {
        Ok(served
            .server
            .forget_bluetooth_device(Request::new(pb::ForgetBluetoothDeviceRequest {
                address: address.to_string(),
            }))
            .await?
            .into_inner())
    }

    fn phones(status: &pb::BluetoothStatus) -> Vec<(&str, Option<&str>, bool)> {
        status
            .devices
            .iter()
            .map(|p| (p.address.as_str(), p.name.as_deref(), p.connected))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_a_bluetooth_controller_nothing_is_configured() -> Fallible {
        let served = served().await?;
        assert_eq!(
            bluetooth(&served).await?,
            pb::BluetoothStatus {
                configured: false,
                running: false,
                error: None,
                pairing: Some(pb::BluetoothPairingWindow {
                    open: false,
                    seconds_left: 0
                }),
                devices: vec![],
            }
        );
        // Pairing and forgetting need a running controller.
        let refused = pairing(&served, true, None).await.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        let refused = forget(&served, "00:00:00:00:00:01").await.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        Ok(())
    }

    /// A controller configured on a build without Bluetooth fails to start,
    /// and says why.
    #[cfg(not(all(target_os = "linux", feature = "ble")))]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_that_failed_says_why() -> Fallible {
        let served = served().await?;
        served
            .player
            .start_controllers(vec![crate::config::Controller::Ble(
                crate::config::BleController::new(None),
            )]);
        let status = bluetooth(&served).await?;
        assert!(status.configured, "{status:?}");
        assert!(!status.running, "{status:?}");
        assert!(status.error.unwrap().contains("Bluetooth"));
        Ok(())
    }

    /// A controller that cannot reach the radio keeps retrying, and says
    /// why until it is running.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_that_is_retrying_says_why() -> Fallible {
        let (served, _fake) = with_bluetooth(false).await?;
        let handle = served.player.ble_handle().expect("handle");
        handle.failed("the radio is blocked");
        let status = bluetooth(&served).await?;
        assert!(!status.running, "{status:?}");
        assert_eq!(status.error.as_deref(), Some("the radio is blocked"));

        let (_fake, attached) = crate::controller::ble::fake::started(vec![]).await;
        handle.attach(attached);
        let status = bluetooth(&served).await?;
        assert!(status.running, "{status:?}");
        assert_eq!(status.error, None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_configured_controller_that_is_not_running_is_closed_and_empty() -> Fallible {
        let (served, _fake) = with_bluetooth(false).await?;
        let status = bluetooth(&served).await?;
        assert!(status.configured);
        assert!(!status.running);
        assert!(!status.pairing.unwrap().open);
        assert!(status.devices.is_empty());
        let refused = pairing(&served, true, None).await.unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn it_lists_the_phones_but_not_the_owners_own_devices() -> Fallible {
        let (served, _fake) = with_bluetooth(true).await?;
        let status = bluetooth(&served).await?;
        assert!(status.running);
        assert_eq!(status.error, None);
        assert_eq!(
            phones(&status),
            vec![
                ("00:00:00:00:00:01", Some("phone 1"), true),
                ("00:00:00:00:00:02", Some("phone 2"), false),
            ]
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_opens_for_the_time_asked_clamped_and_closes() -> Fallible {
        let (served, fake) = with_bluetooth(true).await?;
        let left = |status: &pb::BluetoothStatus| status.pairing.as_ref().unwrap().seconds_left;

        let status = pairing(&served, true, None).await?;
        assert!(status.pairing.as_ref().unwrap().open);
        assert!((119..=120).contains(&left(&status)), "{status:?}");

        // The read sees it too.
        assert!(bluetooth(&served).await?.pairing.unwrap().open);

        let status = pairing(&served, true, Some(1)).await?;
        assert!((9..=10).contains(&left(&status)), "{status:?}");
        let status = pairing(&served, true, Some(86400)).await?;
        assert!((599..=600).contains(&left(&status)), "{status:?}");
        let status = pairing(&served, true, Some(45)).await?;
        assert!((44..=45).contains(&left(&status)), "{status:?}");

        let status = pairing(&served, false, None).await?;
        let window = status.pairing.unwrap();
        assert!(!window.open);
        assert_eq!(window.seconds_left, 0);
        assert_eq!(fake.pairable.lock().last(), Some(&false));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refusing_adapter_is_unavailable() -> Fallible {
        let (served, fake) = with_bluetooth(true).await?;
        *fake.failing.lock() = true;
        let refused = pairing(&served, true, None).await.unwrap_err();
        assert_eq!(refused.code(), Code::Unavailable);
        assert_eq!(refused.message(), "adapter gone");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forgetting_a_phone_removes_it_and_answers_the_list() -> Fallible {
        let (served, fake) = with_bluetooth(true).await?;
        let status = forget(&served, "00:00:00:00:00:01").await?;
        assert_eq!(*fake.removed.lock(), vec![[0, 0, 0, 0, 0, 1]]);
        assert_eq!(
            phones(&status),
            vec![("00:00:00:00:00:02", Some("phone 2"), false)]
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forgetting_what_is_not_a_phone_is_refused() -> Fallible {
        let (served, fake) = with_bluetooth(true).await?;
        let refused = forget(&served, "nonsense").await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert_eq!(refused.message(), "Not a Bluetooth address");
        // Unknown, and the owner's own.
        for address in ["00:00:00:00:00:09", "00:00:00:00:00:03"] {
            let refused = forget(&served, address).await.unwrap_err();
            assert_eq!(refused.code(), Code::NotFound, "{address}");
            assert_eq!(
                refused.message(),
                "Not a phone paired with this player",
                "{address}"
            );
        }
        assert!(fake.removed.lock().is_empty());
        Ok(())
    }

    /// While locked the status is served, and opening or closing the window
    /// and forgetting a phone are refused before the handler runs (the table
    /// in lock_policy; this is the service behind it, as the web UI reaches
    /// it).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_can_be_read_but_not_paired_with() -> Fallible {
        use tower::ServiceExt;
        let (served, fake) = with_bluetooth(true).await?;
        served.player.set_locked(true);
        let call = |method: &'static str| {
            let player = served.player.clone();
            async move {
                let request = http::Request::post(format!("/player.v1.ConfigService/{method}"))
                    .header("content-type", "application/grpc")
                    .body(tonic::body::Body::empty())
                    .unwrap();
                let response = crate::controller::grpc::config_service(player)
                    .oneshot(request)
                    .await
                    .unwrap();
                tonic::Status::from_header_map(response.headers())
                    .map(|s| (s.code(), s.message().to_string()))
            }
        };
        let locked = crate::player::LockedError.to_string();
        for method in ["SetBluetoothPairing", "ForgetBluetoothDevice"] {
            let (code, text) = call(method).await.expect(method);
            assert_eq!(code, Code::FailedPrecondition, "{method}");
            assert_eq!(text, locked, "{method}");
        }
        // The read is past the guard: whatever it answers, it is not the lock.
        if let Some((_, text)) = call("GetBluetooth").await {
            assert_ne!(text, locked);
        }
        assert!(bluetooth(&served).await?.running);
        assert!(fake.removed.lock().is_empty());
        assert_eq!(fake.pairable.lock().as_slice(), &[false, false]);
        Ok(())
    }
}
