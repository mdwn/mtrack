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
import 'dart:async';
import 'dart:typed_data';

import 'package:protobuf/protobuf.dart';
import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import '../../gen/player/v1/player.pb.dart' as player;
import 'asset_download.dart';
import 'player_channel.dart';

export 'asset_download.dart' show AssetDownload, AssetProgress;

/// Calls the player's `PlayerService` over a [PlayerChannel] and follows its
/// status.
///
/// The client is the typed face of the seam: it turns each method into a
/// request message and decodes the answer. Which carrier the channel runs on
/// is not its concern.
class PlayerClient {
  /// A client over [_channel].
  PlayerClient(this._channel);

  final PlayerChannel _channel;

  /// Which radio the client talks over.
  Carrier get carrier => _channel.carrier;

  /// How many calls are waiting for their answer now.
  int get callsInFlight => _channel.callsInFlight;

  /// The player's status as it is pushed.
  Stream<player.RemoteStatus> get statusStream => _channel.statusStream;

  /// The player's status now.
  Future<player.RemoteStatus> readStatus() => _channel.readStatus();

  /// Calls [method] (a bare `PlayerService` method name such as `Play`) with
  /// [request] and returns what [decode] makes of the response payload.
  ///
  /// Throws `PlayerException` when the player answers with a non-zero code,
  /// when no answer comes within [timeout], or when the link is down or goes
  /// down first.
  Future<R> call<R>(
    String method,
    GeneratedMessage request,
    R Function(List<int>) decode, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async => decode(
    await _channel.call(method, request.writeToBuffer(), timeout: timeout),
  );

  /// Opens the server stream [method] with the serialized request [payload].
  /// See [PlayerChannel.openStream].
  Stream<Uint8List> openStream(String method, Uint8List payload) =>
      _channel.openStream(method, payload);

  /// Stops listening, fails every call still waiting and ends every stream.
  Future<void> dispose() => _channel.dispose();

  // Typed calls, one per method the app uses.

  /// Plays the current song.
  Future<player.PlayResponse> play() =>
      call('Play', player.PlayRequest(), player.PlayResponse.fromBuffer);

  /// Plays the current song from [start].
  Future<player.PlayResponse> playFrom(Duration start) => call(
    'PlayFrom',
    player.PlayFromRequest(startTime: pb.Duration.fromDart(start)),
    player.PlayResponse.fromBuffer,
  );

  /// Plays the named song from [start].
  Future<player.PlayResponse> playSongFrom(String song, Duration start) => call(
    'PlaySongFrom',
    player.PlaySongFromRequest(
      songName: song,
      startTime: pb.Duration.fromDart(start),
    ),
    player.PlayResponse.fromBuffer,
  );

  /// Stops the song that is playing.
  Future<player.StopResponse> stop() =>
      call('Stop', player.StopRequest(), player.StopResponse.fromBuffer);

  /// Moves the playlist to the next song.
  Future<player.NextResponse> next() =>
      call('Next', player.NextRequest(), player.NextResponse.fromBuffer);

  /// Moves the playlist to the previous song.
  Future<player.PreviousResponse> previous() => call(
    'Previous',
    player.PreviousRequest(),
    player.PreviousResponse.fromBuffer,
  );

  /// Makes the playlist entry at [position] current without playing it.
  /// [songName] is the song the caller believes is there: the player refuses
  /// with FAILED_PRECONDITION when the entry holds another.
  Future<player.NavigateToSongResponse> navigateToSong(
    int position,
    String songName,
  ) => call(
    'NavigateToSong',
    player.NavigateToSongRequest(position: position, songName: songName),
    player.NavigateToSongResponse.fromBuffer,
  );

  /// Jumps to [position] in the current song.
  Future<player.SeekResponse> seek(Duration position) => call(
    'Seek',
    player.SeekRequest(position: pb.Duration.fromDart(position)),
    player.SeekResponse.fromBuffer,
  );

  /// Jumps to the start of the named section of the current song.
  Future<player.SeekResponse> seekToSection(String section) => call(
    'SeekToSection',
    player.SeekToSectionRequest(sectionName: section),
    player.SeekResponse.fromBuffer,
  );

  /// Switches the player to the named playlist.
  Future<player.SwitchToPlaylistResponse> switchToPlaylist(String name) => call(
    'SwitchToPlaylist',
    player.SwitchToPlaylistRequest(playlistName: name),
    player.SwitchToPlaylistResponse.fromBuffer,
  );

  /// The player's full status, with the song's beat grid and sections.
  Future<player.StatusResponse> status() =>
      call('Status', player.StatusRequest(), player.StatusResponse.fromBuffer);

  /// Loops the named section of the current song.
  Future<player.LoopSectionResponse> loopSection(String section) => call(
    'LoopSection',
    player.LoopSectionRequest(sectionName: section),
    player.LoopSectionResponse.fromBuffer,
  );

  /// Ends the section loop at the end of the current pass.
  Future<player.StopSectionLoopResponse> stopSectionLoop() => call(
    'StopSectionLoop',
    player.StopSectionLoopRequest(),
    player.StopSectionLoopResponse.fromBuffer,
  );

  /// Acknowledges a section that is waiting for it.
  Future<player.SectionAckResponse> sectionAck() => call(
    'SectionAck',
    player.SectionAckRequest(),
    player.SectionAckResponse.fromBuffer,
  );

  /// Stops all triggered samples that are playing.
  Future<player.StopSamplesResponse> stopSamples() => call(
    'StopSamples',
    player.StopSamplesRequest(),
    player.StopSamplesResponse.fromBuffer,
  );

  /// Cancels the song that is waiting to start by itself.
  Future<player.CancelAutoPlayResponse> cancelAutoPlay() => call(
    'CancelAutoPlay',
    player.CancelAutoPlayRequest(),
    player.CancelAutoPlayResponse.fromBuffer,
  );

  /// The names of the playlists and which is active.
  Future<player.ListPlaylistsResponse> listPlaylists() => call(
    'ListPlaylists',
    player.ListPlaylistsRequest(),
    player.ListPlaylistsResponse.fromBuffer,
  );

  /// The entries of the named playlist, or of the active one when [name] is
  /// null.
  Future<player.GetPlaylistResponse> getPlaylist([String? name]) => call(
    'GetPlaylist',
    player.GetPlaylistRequest(name: name),
    player.GetPlaylistResponse.fromBuffer,
  );

  /// The playlist [name] as [getPlaylist] has it, with the lighting palette
  /// of each of its songs (`songPalettes`). Not the whole library, which is
  /// too large for Bluetooth.
  Future<player.GetPlaylistResponse> getPlaylistWithPalettes(String name) =>
      call(
        'GetPlaylist',
        player.GetPlaylistRequest(name: name, withPalettes: true),
        player.GetPlaylistResponse.fromBuffer,
      );

  /// Sets the songs of the playlist [name] to [songs], in order. With
  /// [expected], the save is refused when the playlist on the player is no
  /// longer those songs, and with [expectedRevision] as well (the revision
  /// `GetPlaylist` answered) when its settings changed. With [create], the
  /// playlist must not exist yet.
  ///
  /// With [settings], one per song in order, each entry's settings are set
  /// from them; without, each entry keeps its own, with its song. With
  /// [setAmbient], the playlist's ambient light is set to [ambient] (null:
  /// the profile's); without, it keeps its own. Answers with the playlist
  /// as saved.
  Future<player.GetPlaylistResponse> savePlaylist(
    String name,
    List<String> songs, {
    List<String>? expected,
    String? expectedRevision,
    bool create = false,
    List<player.EntrySettings>? settings,
    bool setAmbient = false,
    player.AmbientSetting? ambient,
  }) => call(
    'SavePlaylist',
    player.SavePlaylistRequest(
      name: name,
      songs: songs,
      expected: expected,
      checkExpected: expected != null,
      expectedRevision: expected == null ? null : expectedRevision,
      // The generator suffixes the field's name; it is `create` on the wire.
      create_5: create,
      settings: settings,
      setSettings: settings != null,
      ambient: setAmbient ? ambient : null,
      setAmbient: setAmbient,
    ),
    player.GetPlaylistResponse.fromBuffer,
  );

  /// A page of the song library from [start] (0 for the first): every song,
  /// loaded or not, sorted by name. A page with `next` set has more after
  /// it; see [listAllSongs].
  Future<player.ListSongsResponse> listSongs({int start = 0}) => call(
    'ListSongs',
    player.ListSongsRequest(start: start),
    player.ListSongsResponse.fromBuffer,
  );

  /// The whole song library, page after page.
  Future<List<player.SongListing>> listAllSongs() async {
    final songs = <player.SongListing>[];
    var start = 0;
    while (true) {
      final page = await listSongs(start: start);
      songs.addAll(page.songs);
      // A page that names no further start, or one that does not move on,
      // is the last.
      if (!page.hasNext() || page.next <= start || page.songs.isEmpty) {
        return songs;
      }
      start = page.next;
    }
  }

  /// The song [name]'s settings, as its file has them.
  Future<player.GetSongResponse> getSong(String name) => call(
    'GetSong',
    player.GetSongRequest(name: name),
    player.GetSongResponse.fromBuffer,
  );

  /// Changes the song [name]'s looping, tracks or samples: each given is
  /// set, and each left null is left as the song has it. With
  /// [expectedRevision] (the revision `GetSong` answered), the change is
  /// refused when the song's file has changed since. Answers with the song
  /// as saved.
  Future<player.GetSongResponse> updateSongSettings(
    String name, {
    String? expectedRevision,
    player.SongLooping? looping,
    player.SongTracks? tracks,
    player.SongSamples? samples,
  }) => call(
    'UpdateSongSettings',
    player.UpdateSongSettingsRequest(
      name: name,
      expectedRevision: expectedRevision,
      looping: looping,
      tracks: tracks,
      samples: samples,
    ),
    player.GetSongResponse.fromBuffer,
  );

  /// The `.light` files an ambient light can play, as paths relative to the
  /// player's project.
  Future<player.ListLightShowsResponse> listLightShows() => call(
    'ListLightShows',
    player.ListLightShowsRequest(),
    player.ListLightShowsResponse.fromBuffer,
  );

  /// Deletes the playlist [name].
  Future<player.DeletePlaylistResponse> deletePlaylist(String name) => call(
    'DeletePlaylist',
    player.DeletePlaylistRequest(name: name),
    player.DeletePlaylistResponse.fromBuffer,
  );

  /// Locks the player, or unlocks it. Allowed while locked.
  Future<player.SetLockResponse> setLock(bool locked) => call(
    'SetLock',
    player.SetLockRequest(locked: locked),
    player.SetLockResponse.fromBuffer,
  );

  /// The gain and mute of every output track, in the player's order.
  Future<player.GetTrackGainsResponse> getTrackGains() => call(
    'GetTrackGains',
    player.GetTrackGainsRequest(),
    player.GetTrackGainsResponse.fromBuffer,
  );

  /// Sets the gain of output track [track] to [gainDb]. The player clamps it
  /// and answers with the gain it applied.
  Future<player.SetTrackGainResponse> setTrackGain(
    String track,
    double gainDb,
  ) => call(
    'SetTrackGain',
    player.SetTrackGainRequest(track: track, gainDb: gainDb),
    player.SetTrackGainResponse.fromBuffer,
  );

  /// Mutes or unmutes output track [track].
  Future<player.SetTrackMuteResponse> setTrackMute(String track, bool muted) =>
      call(
        'SetTrackMute',
        player.SetTrackMuteRequest(track: track, muted: muted),
        player.SetTrackMuteResponse.fromBuffer,
      );

  /// Sets the ambient light's mode: `auto`, `on` or `off`.
  Future<player.SetAmbientModeResponse> setAmbientMode(String mode) => call(
    'SetAmbientMode',
    player.SetAmbientModeRequest(mode: mode),
    player.SetAmbientModeResponse.fromBuffer,
  );

  /// Whether the player can carry a stream at all: false for one from before
  /// streams. Valid while the link is up.
  bool get hasStreams => _channel.hasStreams;

  /// Opens the stage stream: serialized `StageUpdate` messages, the layout
  /// first and then at most [maxRate] frames a second (0 for the player's own
  /// rate). See [openStream] for how it ends and what cancelling does.
  Stream<Uint8List> watchStage({int maxRate = 10}) => openStream(
    'WatchStage',
    player.WatchStageRequest(maxRate: maxRate).writeToBuffer(),
  );

  /// Fetches the lighting asset at [path] (a path under the player's
  /// `/api/lighting/assets/`), gzipped on the link and unpacked here. The
  /// download starts at once; [onProgress] is told as its chunks arrive.
  /// See [AssetDownload] for how it fails and how it is stopped.
  ///
  /// It is a stream, and takes one of the two a phone may have open until
  /// [AssetDownload.freed]. Disposing of the client or losing the link ends
  /// it with `UNAVAILABLE`.
  AssetDownload getLightingAsset(String path, {AssetProgress? onProgress}) =>
      AssetDownload(
        openStream(
          'GetLightingAsset',
          player.GetLightingAssetRequest(
            path: path,
            gzip: true,
          ).writeToBuffer(),
        ),
        onProgress: onProgress,
      );

  /// Where the player is on the network: its host name, addresses and the
  /// port its gRPC controller listens on.
  Future<player.GetNetworkInfoResponse> getNetworkInfo({
    Duration timeout = PlayerChannel.defaultTimeout,
  }) => call(
    'GetNetworkInfo',
    player.GetNetworkInfoRequest(),
    player.GetNetworkInfoResponse.fromBuffer,
    timeout: timeout,
  );
}
