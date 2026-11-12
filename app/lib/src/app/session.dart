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

import 'package:flutter/foundation.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/envelope_channel.dart';
import '../protocol/playback_clock.dart';
import '../protocol/player_channel.dart';
import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';
import '../protocol/transport.dart';
import 'asset_cache.dart';
import 'failure.dart';
import 'levels.dart';
import 'lighting_assets.dart';
import 'player_link.dart';
import 'playlist_edit.dart';
import 'ports.dart';
import 'sections.dart';
import 'song_edit.dart';
import 'stage_watch.dart';
import 'wifi_address.dart';

/// Where the connection to a player stands.
enum ConnectionPhase {
  /// No player is remembered. The connect screen is shown.
  noPlayer,

  /// A link is being opened to the player.
  connecting,

  /// The phone is being paired with the player.
  pairing,

  /// The player refused to pair: its pairing window is closed.
  pairingRefused,

  /// Connected and showing the player's state.
  ready,

  /// The link dropped or could not be opened. The app is trying again.
  lost,
}

/// The things a person can ask the player to do. Each is in flight at most
/// once at a time.
enum ActionKind {
  play,
  stop,
  next,
  previous,
  navigate,
  switchPlaylist,
  seekSection,
  loopSection,
  stopLoop,
  cancelAutoPlay,
  setAmbient,
  setLock,
  deletePlaylist,
}

/// What the app knows of the current song beyond the status: its sections
/// placed on its beat grid.
class SongDetails {
  /// Details of the song [name].
  const SongDetails(this.name, this.song, this.sections);

  /// The song's name.
  final String name;

  /// The player's description of the song.
  final player.Song song;

  /// The song's sections, placed on its beat grid where the grid allows.
  final List<SectionSpan> sections;
}

/// The player's playlists and which one is active.
class PlaylistChoices {
  /// The playlists, [names] sorted as the player sorts them.
  const PlaylistChoices(this.names, this.active);

  /// Every playlist's name.
  final List<String> names;

  /// The active playlist's name.
  final String active;
}

/// How long to wait before each reconnect attempt after a failure: 1 s, 2 s,
/// 5 s, then 5 s again for as long as it takes.
const List<Duration> reconnectBackoff = [
  Duration(seconds: 1),
  Duration(seconds: 2),
  Duration(seconds: 5),
];

/// The most Wi-Fi addresses tried for a player before going to Bluetooth.
const int maxWifiHosts = 3;

/// How long to wait before asking again for song or playlist details that
/// could not be fetched.
const Duration detailsRetryDelay = Duration(seconds: 3);

/// How long to wait for a player to answer over Wi-Fi before trying
/// Bluetooth. A player that is on the network answers in a few hundred
/// milliseconds; this is how long a connect is held up when it is not.
const Duration wifiConnectTimeout = Duration(seconds: 2);

/// How often, while connected by Bluetooth to a player that has Wi-Fi
/// addresses, to see whether it answers there now.
const Duration wifiRetryInterval = Duration(seconds: 20);

/// How long a switch from Bluetooth to Wi-Fi waits for the calls in flight
/// to finish before it gives the attempt up until the next interval.
const Duration switchPatience = Duration(seconds: 10);

/// A reading of a monotonic clock, as time since some fixed start.
typedef Monotonic = Duration Function();

/// Everything the screens show, and every action they can take.
///
/// A session owns the link to one player: it finds it, pairs, follows its
/// status, fetches what the status leaves out, reconnects when the link
/// drops, and sends the person's commands. It knows nothing of widgets, and
/// reaches the radio only through a [PlayerLink], so every decision in it is
/// tested without one.
class Session extends ChangeNotifier {
  /// A session over [link], remembering its player in [store], and keeping
  /// the lighting assets it fetches in [assetCache] (none kept when null).
  /// With [wifi], a player is also reached over the network when it answers
  /// there; without, over Bluetooth alone.
  Session({
    required this._link,
    required this._store,
    this._wake,
    this._wifi,
    Monotonic? monotonic,
    this._backoff = reconnectBackoff,
    this._wifiTimeout = wifiConnectTimeout,
    this._wifiInterval = wifiRetryInterval,
    LightingAssetCache? assetCache,
  }) : _monotonic = monotonic ?? _stopwatchClock() {
    clock = PlaybackClock(_monotonic);
    levels = Levels(
      client: () => _client,
      isReady: () => isReady,
      notice: _notice,
      lostIfUnanswered: _lostIfUnanswered,
    );
    _stage = StageWatch(
      client: () => _client,
      isReady: () => isReady,
      lostIfUnanswered: _lostIfUnanswered,
      changed: _notify,
    );
    lightingAssets = LightingAssets(
      client: () => _client,
      isReady: () => isReady,
      cache: assetCache,
    )..addListener(_assetsChanged);
    songLibrary = SongLibrary(
      client: () => _client,
      isReady: () => isReady,
      lostIfUnanswered: _lostIfUnanswered,
    )..addListener(_libraryChanged);
  }

  final PlayerLink _link;
  final PlayerStore _store;
  final ScreenWake? _wake;
  final Monotonic _monotonic;
  final List<Duration> _backoff;
  final WifiLink? _wifi;
  final Duration _wifiTimeout;
  final Duration _wifiInterval;

  /// The song position to show, run between status messages.
  late final PlaybackClock clock;

  /// The output tracks' gains and mutes, for the Levels tab.
  late final Levels levels;

  /// The player's lighting assets, for the 3D view of the Lights tab.
  late final LightingAssets lightingAssets;
  bool _assetsTooOld = false;

  /// The player's songs, for the Library tab's Songs side.
  late final SongLibrary songLibrary;
  bool _libraryTooOld = false;

  late final StageWatch _stage;

  final StreamController<String> _notices = StreamController<String>.broadcast(
    sync: true,
  );

  ConnectionPhase _phase = ConnectionPhase.noPlayer;
  RememberedPlayer? _target;
  String? _connectError;
  AdapterState _adapter = AdapterState.ready;

  PlayerTransport? _transport;
  PlayerChannel? _channel;
  PlayerClient? _client;
  final List<StreamSubscription<Object?>> _subscriptions = [];

  player.RemoteStatus? _status;
  String? _ambientMode;
  String? _ambientState;
  Duration _statusAt = Duration.zero;
  SongDetails? _details;
  String? _detailsName;
  List<player.PlaylistEntry> _entries = const [];
  player.AmbientSetting? _entriesAmbient;
  String _entriesRevision = '';
  String? _entriesPlaylist;
  String? _entriesAskedFor;
  PlaylistChoices? _choices;
  bool _loadingChoices = false;
  bool? _lockedAnswer;
  PlaylistEdit? _edit;

  final Set<ActionKind> _busy = {};

  int _epoch = 0;
  int _retries = 0;
  Timer? _retryTimer;
  Timer? _detailsTimer;
  Timer? _wifiProbe;
  bool _probing = false;
  String? _wifiOldFor;
  PlayerException? _wifiError;
  bool _syncing = false;
  bool _syncAgain = false;

  List<DiscoveredPlayer> _found = const [];
  bool _scanning = false;
  StreamSubscription<DiscoveredPlayer>? _scan;

  bool _foreground = true;
  bool? _wakeAsked;
  bool _disposed = false;

  /// Where the connection stands.
  ConnectionPhase get phase => _phase;

  /// The player being connected to or connected, or null.
  RememberedPlayer? get target => _target;

  /// The name of that player.
  String? get playerName => _target?.name;

  /// Why the last attempt to connect to a new player failed, in words for a
  /// person; null otherwise.
  String? get connectError => _connectError;

  /// Which radio the connection runs on, or null when not connected.
  Carrier? get carrier => isReady ? _channel?.carrier : null;

  /// Whether Bluetooth can be used, as last checked.
  AdapterState get adapter => _adapter;

  /// The latest status from the player. Kept, and shown dimmed, while the
  /// link is lost.
  player.RemoteStatus? get status => _status;

  /// The ambient light's mode (`auto`, `on` or `off`), or null when the
  /// player has no ambient light to set.
  String? get ambientMode => _ambientMode;

  /// What the ambient light is doing (`playing`, `pending`, `off` or
  /// `unavailable`), or null with [ambientMode].
  String? get ambientState => _ambientState;

  /// Whether the player is locked, or null when it does not say (a player
  /// from before a phone could lock it, which cannot be edited from one).
  bool? get locked {
    final answered = _lockedAnswer;
    if (answered != null) {
      return answered;
    }
    final s = _status;
    return s != null && s.hasLocked() ? s.locked : null;
  }

  /// Whether the player has a lock a phone can use: what shows the lock, and
  /// the playlist editing it guards.
  bool get lockAvailable => locked != null;

  /// What the app can do that the connected player is too old for, in words
  /// for a sentence; empty when it has everything, or when nothing has been
  /// heard from a player yet. Each feature came with a field in the status,
  /// so a status without the field is from before the feature; playlist
  /// settings came with a revision on the playlist; the stage
  /// view came with streams instead (see [stageMissingFromPlayer]), and its
  /// 3D with `GetLightingAsset` (see [LightingAssets.tooOld]).
  List<String> get missingFromPlayer {
    final status = _status;
    if (status == null) {
      return const [];
    }
    return [
      if (!status.hasGainsRevision()) 'track levels',
      if (!status.hasLocked()) 'playlist editing',
      if (!status.hasLocked()) 'the lock',
      // Known once a playlist has come: one without a revision is from a
      // player that edits songs and their order only.
      if (status.hasLocked() &&
          _entriesPlaylist != null &&
          !playlistSettingsAvailable)
        'playlist settings',
      if (_stage.missing) 'the light view',
      if (!_stage.missing && lightingAssets.tooOld) 'the 3D light view',
      // Known once the Songs side has asked for them.
      if (songLibrary.tooOld) 'the song library',
    ];
  }

  /// The settings sheet names a player too old for the song library, so it
  /// hears when that is learned or forgotten.
  void _libraryChanged() {
    if (songLibrary.tooOld != _libraryTooOld) {
      _libraryTooOld = songLibrary.tooOld;
      _notify();
    }
  }

  /// An edit of the song [name], for its page, which disposes of it. A
  /// save tells the library what the song now has.
  SongEdit editSong(String name) => SongEdit(
    name: name,
    client: () => _client,
    isReady: () => isReady,
    locked: () => locked,
    notice: _notice,
    lostIfUnanswered: _lostIfUnanswered,
    saved: songLibrary.noteSaved,
  );

  /// The settings sheet names a player too old for the 3D view, so it
  /// hears when that is learned or forgotten.
  void _assetsChanged() {
    if (lightingAssets.tooOld != _assetsTooOld) {
      _assetsTooOld = lightingAssets.tooOld;
      _notify();
    }
  }

  /// True when the connected player is too old to show its stage: it has no
  /// `stream` characteristic, which is known on connecting, or it refused
  /// `WatchStage` as a method it does not allow, which is known once
  /// [watchStage] has been listened to. [missingFromPlayer] names it too.
  bool get stageMissingFromPlayer => _stage.missing;

  /// The player's stage, as serialized `player.v1.StageUpdate` messages for
  /// the page that draws it: nothing here decodes them.
  ///
  /// Each message is a [StageBytes]. A [StageReset] comes before the first
  /// message of every stream the session opens at the player, the very first
  /// included: the page forgets its layout on it. The session opens the
  /// stream when this is first listened to, cancels it when the last
  /// listener cancels, and opens it again by itself after a reconnect, so a
  /// listener listens when its tab shows and cancels when it hides, as often
  /// as it likes. While the link is down, and against a player that is too
  /// old ([stageMissingFromPlayer]), the stream is silent; it does not end
  /// or fail.
  ///
  /// Several listeners share the one stream the player has open. When one
  /// joins, the stream is started over so that it gets the layout, and every
  /// listener sees the [StageReset]. [maxRate] is the most frames a second
  /// this listener wants (0 for the player's own); the stream runs at the
  /// highest asked for. The default asks for what the link carries: the
  /// player's own rate over Wi-Fi, 10 over Bluetooth.
  ///
  /// Events are handed over as they arrive and never queued: a listener that
  /// cannot keep up keeps the newest frame itself, and never drops a layout.
  /// One that pauses its subscription is passed over, and gets a
  /// [StageReset] and a new stream when it resumes.
  Stream<StageEvent> watchStage({int maxRate = stageRateAuto}) =>
      _stage.watch(maxRate: maxRate);

  /// Whether the active playlist may be edited now: the player is unlocked
  /// and the playlist is not its own list of every song.
  bool get canEditPlaylist {
    final name = _status?.playlistName ?? '';
    return locked == false && name.isNotEmpty && name != allSongsName;
  }

  /// The playlist being edited, or null.
  PlaylistEdit? get editing => _edit;

  /// The current song's sections, once fetched.
  SongDetails? get details => _details;

  /// The active playlist's entries, once fetched.
  List<player.PlaylistEntry> get entries => _entries;

  /// The playlists, once the picker has asked for them.
  PlaylistChoices? get choices => _choices;

  /// True while the list of playlists is being fetched.
  bool get loadingChoices => _loadingChoices;

  /// The players found by the scan, strongest first.
  List<DiscoveredPlayer> get found => _found;

  /// True while a scan is running.
  bool get scanning => _scanning;

  /// True when the link is up and the player's state is current.
  bool get isReady => _phase == ConnectionPhase.ready;

  /// True when the screens should not accept commands: not connected.
  bool get controlsDisabled => !isReady;

  /// Whether a song is playing, as of the latest status.
  bool get playing => _status?.playing ?? false;

  /// The index of the playlist entry the player is on, or null.
  int? get position =>
      (_status?.hasPosition() ?? false) ? _status!.position : null;

  /// The song after the current one in the playlist, or null at the end of
  /// the list or when the position is not known.
  player.PlaylistEntry? get nextEntry {
    final at = position;
    if (at == null || at + 1 >= _entries.length) {
      return null;
    }
    return _entries[at + 1];
  }

  /// The section a section loop is set on, if any.
  String? get loopingSection {
    final s = _status;
    return s != null && s.hasLoopSection() ? s.loopSection : null;
  }

  /// How long until the waiting song starts by itself, counted down on the
  /// phone between status messages; null when none is waiting.
  Duration? get autoPlayRemaining {
    final s = _status;
    if (s == null || !s.hasAutoPlay()) {
      return null;
    }
    final left =
        s.autoPlay.startsIn.toDart() -
        (_isLive ? _monotonic() - _statusAt : Duration.zero);
    return left.isNegative ? Duration.zero : left;
  }

  /// The name of the song waiting to start by itself, or null.
  String? get autoPlaySong {
    final s = _status;
    return s != null && s.hasAutoPlay() ? s.autoPlay.song : null;
  }

  bool get _isLive => _phase == ConnectionPhase.ready;

  /// True while [kind] has a request out, so its control stays disabled.
  bool isBusy(ActionKind kind) => _busy.contains(kind);

  /// Messages for the person to be shown briefly: why an action failed.
  Stream<String> get notices => _notices.stream;

  /// Loads the remembered player and, when there is one, connects to it.
  Future<void> start() async {
    final remembered = await _store.load();
    if (_disposed || remembered == null) {
      return;
    }
    _target = remembered;
    unawaited(_connect(remembered));
  }

  // The connection.

  /// Connects to [chosen], which the person picked from the scan, and
  /// remembers it once the connection is up. A player remembered already
  /// keeps the Wi-Fi addresses it was told.
  Future<void> connectTo(DiscoveredPlayer chosen) async {
    stopScan();
    _connectError = null;
    final prior = await _store.load();
    if (_disposed) {
      return;
    }
    final target = prior != null && prior.id == chosen.id
        ? prior.copyWith(name: chosen.name)
        : RememberedPlayer(id: chosen.id, name: chosen.name);
    _target = target;
    return _connect(target);
  }

  /// Connects to the player at [address], which the person typed, and
  /// remembers it once the connection is up. Such a player is reached over
  /// Wi-Fi alone.
  Future<void> connectToAddress(WifiAddress address) {
    stopScan();
    _connectError = null;
    final target = RememberedPlayer.byAddress(address);
    _target = target;
    return _connect(target);
  }

  /// Tries again to connect to the player, after a refusal to pair or a lost
  /// link, without waiting for the next scheduled attempt.
  Future<void> retryNow() {
    final target = _target;
    if (target == null) {
      return Future<void>.value();
    }
    _retries = 0;
    _wifiOldFor = null;
    return _connect(target);
  }

  /// Gives up on the player being connected to and goes back to finding one.
  /// What is remembered stays, unless the connection never completed.
  Future<void> cancelConnecting() async {
    _epoch++;
    _cancelTimers();
    await _dropConnection();
    final remembered = await _store.load();
    if (remembered == null) {
      _target = null;
      _setPhase(ConnectionPhase.noPlayer);
    } else {
      _target = remembered;
      _setPhase(ConnectionPhase.lost);
      _scheduleRetry();
    }
  }

  /// Disconnects and forgets the player, so the app goes back to finding one.
  Future<void> forget() async {
    _epoch++;
    _cancelTimers();
    await _dropConnection();
    await _store.clear();
    _wifiOldFor = null;
    _target = null;
    _connectError = null;
    _status = null;
    _ambientMode = null;
    _ambientState = null;
    levels.clear();
    _details = null;
    _detailsName = null;
    _entries = const [];
    _entriesAmbient = null;
    _entriesRevision = '';
    _entriesPlaylist = null;
    _entriesAskedFor = null;
    _choices = null;
    _lockedAnswer = null;
    _endEdit();
    clock.reset();
    _stage.clear();
    lightingAssets.clear();
    songLibrary.clear();
    _setPhase(ConnectionPhase.noPlayer);
  }

  Future<void> _connect(RememberedPlayer target) async {
    final epoch = ++_epoch;
    _cancelTimers();
    await _dropConnection(keepState: true);
    if (_stale(epoch)) {
      return;
    }
    final wasLost = _phase == ConnectionPhase.lost;
    if (!wasLost) {
      _setPhase(ConnectionPhase.connecting);
    }
    PlayerTransport? transport;
    try {
      // Wi-Fi first, when there is anywhere to look: it is quicker, carries
      // more, and needs no pairing. Bluetooth when nothing answers.
      final wifi = await _tryWifi(target, epoch);
      if (_stale(epoch)) {
        await wifi?.channel.dispose();
        return;
      }
      if (wifi != null) {
        await _adopt(wifi.target, epoch, wifi.channel, null);
        return;
      }
      if (!target.bluetooth) {
        throw _wifiError ??
            const PlayerException.unavailable('nothing answered over Wi-Fi');
      }
      final adapter = await _link.adapterState();
      if (_stale(epoch)) {
        return;
      }
      _adapter = adapter;
      if (adapter != AdapterState.ready) {
        throw const PlayerException.unavailable('Bluetooth is not available');
      }
      transport = await _link.connect(target.id);
      if (_stale(epoch)) {
        await transport.close();
        return;
      }
      if (!await _link.isPaired(target.id)) {
        if (_stale(epoch)) {
          await transport.close();
          return;
        }
        // The player drops a phone that stays connected unpaired for two
        // seconds, so pairing starts at once.
        _setPhase(ConnectionPhase.pairing);
        await _link.pair(target.id);
      }
      if (_stale(epoch)) {
        await transport.close();
        return;
      }
      await _link.subscribe(target.id);
      if (_stale(epoch)) {
        await transport.close();
        return;
      }
      await _adopt(target, epoch, EnvelopeChannel(transport), transport);
    } on PairingRefusedException {
      if (_stale(epoch)) {
        return;
      }
      await transport?.close();
      await _dropConnection(keepState: true);
      _setPhase(ConnectionPhase.pairingRefused);
    } on Object catch (e) {
      if (_stale(epoch)) {
        return;
      }
      await transport?.close();
      await _dropConnection(keepState: true);
      if (await _store.load() == null) {
        // A first connection that did not work: back to the choice of
        // players, with the reason.
        _connectError = target.bluetooth
            ? describeConnectFailure(e, _adapter)
            : describeWifiFailure(e, target);
        _target = null;
        _setPhase(ConnectionPhase.noPlayer);
      } else {
        _setPhase(ConnectionPhase.lost);
        _scheduleRetry();
      }
    }
  }

  /// Makes [channel] the connection to [target]: follows its status, fetches
  /// what the status leaves out, remembers the player and goes ready.
  /// [transport] is the Bluetooth link under [channel], if there is one.
  Future<void> _adopt(
    RememberedPlayer target,
    int epoch,
    PlayerChannel channel,
    PlayerTransport? transport,
  ) async {
    final client = PlayerClient(channel);
    _target = target;
    _transport = transport;
    _channel = channel;
    _client = client;
    _subscriptions
      ..add(client.statusStream.listen(_onStatus))
      ..add(channel.state.listen(_onTransportState));
    final first = await client.readStatus();
    if (_stale(epoch)) {
      return;
    }
    _onStatus(first);
    await _sync();
    if (_stale(epoch)) {
      return;
    }
    if (channel.currentState != TransportState.ready) {
      throw const PlayerException.unavailable('connection lost');
    }
    await _store.save(target);
    _retries = 0;
    _connectError = null;
    _setPhase(ConnectionPhase.ready);
    if (channel.carrier == Carrier.bluetooth) {
      // Ask where the player is on the network, for the next time, and keep
      // an eye out for it answering there.
      unawaited(_learnAddresses(client, epoch));
      _startWifiProbe();
    }
  }

  // Wi-Fi.

  /// Tries the Wi-Fi addresses [target] has, in order, for a player that
  /// answers on one and is the player remembered. Returns the open channel,
  /// with the target as it should now be remembered, or null: nothing
  /// answered, the player is too old, or what answered was another device.
  /// Why is left in [_wifiError].
  Future<({PlayerChannel channel, RememberedPlayer target})?> _tryWifi(
    RememberedPlayer target,
    int epoch,
  ) async {
    final link = _wifi;
    final port = target.port;
    _wifiError = null;
    if (link == null ||
        port == null ||
        !target.hasWifi ||
        (target.bluetooth && _wifiOldFor == target.id)) {
      return null;
    }
    for (final host in target.hosts.take(maxWifiHosts)) {
      PlayerChannel? channel;
      try {
        channel = await link.connect(host, port, timeout: _wifiTimeout);
        if (_stale(epoch)) {
          await channel.dispose();
          return null;
        }
        player.GetNetworkInfoResponse? info;
        try {
          info = await PlayerClient(channel)
              .getNetworkInfo(timeout: _wifiTimeout);
        } on PlayerException catch (e) {
          if (e.isTimeout || e.isUnavailable) {
            rethrow;
          }
          // A player that will not say who it is cannot be told from another
          // at the same address, which only matters once it has been told.
        }
        final known = target.hostname;
        if (known != null && (info == null || info.hostname != known)) {
          // Another device has this address now (the lease moved), or the
          // player was renamed: not the player this was remembered as.
          await channel.dispose();
          continue;
        }
        return (
          channel: channel,
          target: info == null ? target : target.learned(info, host),
        );
      } on Object catch (e) {
        await channel?.dispose();
        if (_stale(epoch)) {
          return null;
        }
        final error = e is PlayerException
            ? e
            : PlayerException.unavailable('$e');
        _wifiError = error;
        if (error.code == GrpcCode.unimplemented) {
          // A player from before Wi-Fi: Bluetooth only, and nothing to tell
          // the person about.
          _wifiOldFor = target.id;
          return null;
        }
      }
    }
    return null;
  }

  /// Asks the player, over Bluetooth, where it is on the network, and
  /// remembers the answer with it. A player too old to know, or one that
  /// does not answer, leaves what was remembered alone.
  Future<void> _learnAddresses(PlayerClient client, int epoch) async {
    final player.GetNetworkInfoResponse info;
    try {
      info = await client.getNetworkInfo(timeout: _wifiTimeout * 2);
    } on Object {
      return;
    }
    final target = _target;
    if (_stale(epoch) || target == null || _client != client) {
      return;
    }
    final learned = target.learned(info, null);
    if (learned != target) {
      _target = learned;
      await _store.save(learned);
    }
    if (!_stale(epoch)) {
      _startWifiProbe();
    }
  }

  void _startWifiProbe() {
    _wifiProbe?.cancel();
    _wifiProbe = null;
    final target = _target;
    if (_wifi == null ||
        _disposed ||
        target == null ||
        !target.hasWifi ||
        _wifiOldFor == target.id ||
        _channel?.carrier != Carrier.bluetooth) {
      return;
    }
    _wifiProbe = Timer.periodic(_wifiInterval, (_) => unawaited(_probeWifi()));
  }

  /// Sees whether the player answers over Wi-Fi now, and moves over to it
  /// when it does and nothing is waiting on the Bluetooth link.
  Future<void> _probeWifi() async {
    final target = _target;
    if (_probing ||
        target == null ||
        _phase != ConnectionPhase.ready ||
        _channel?.carrier != Carrier.bluetooth) {
      return;
    }
    _probing = true;
    final epoch = _epoch;
    try {
      final found = await _tryWifi(target, epoch);
      if (found == null) {
        return;
      }
      if (!await _idle(epoch)) {
        await found.channel.dispose();
        return;
      }
      await _switchTo(found.target, found.channel);
    } on Object {
      // Nothing was changed until the switch; one that failed part way is
      // a lost link, which is handled.
    } finally {
      _probing = false;
    }
  }

  /// Waits, up to [switchPatience], until no call is in flight. False when
  /// calls kept coming, or the connection changed meanwhile.
  Future<bool> _idle(int epoch) async {
    final until = _monotonic() + switchPatience;
    while (true) {
      if (_stale(epoch) || _phase != ConnectionPhase.ready) {
        return false;
      }
      if (_busy.isEmpty && !_syncing && (_client?.callsInFlight ?? 0) == 0) {
        return true;
      }
      if (_monotonic() >= until) {
        return false;
      }
      await Future<void>.delayed(const Duration(milliseconds: 250));
    }
  }

  /// Moves the connection from Bluetooth to the open Wi-Fi [channel]: a
  /// reconnect like any other, without the looking.
  Future<void> _switchTo(RememberedPlayer target, PlayerChannel channel) async {
    final epoch = ++_epoch;
    _cancelTimers();
    clock.hold();
    _setPhase(ConnectionPhase.lost);
    await _dropConnection(keepState: true);
    if (_stale(epoch)) {
      await channel.dispose();
      return;
    }
    try {
      await _adopt(target, epoch, channel, null);
    } on Object {
      if (_stale(epoch)) {
        return;
      }
      await _dropConnection(keepState: true);
      _retries = 0;
      _setPhase(ConnectionPhase.lost);
      _scheduleRetry();
    }
  }

  bool _stale(int epoch) => _disposed || epoch != _epoch;

  void _onTransportState(TransportState state) {
    if (state == TransportState.ready || _phase != ConnectionPhase.ready) {
      return;
    }
    // The link dropped under a working connection.
    _lose();
  }

  /// Gives up a working connection and starts over from the first retry.
  void _lose() {
    _epoch++;
    clock.hold();
    unawaited(_dropConnection(keepState: true));
    _retries = 0;
    _setPhase(ConnectionPhase.lost);
    _scheduleRetry();
  }

  /// A call to [client] failed with [error]. No answer in time, or a write
  /// that would not go, on a link the phone still calls connected means the
  /// player behind it is not the one subscribed to (it restarted, and the
  /// radio kept the link): the connection is made again. Returns whether it
  /// was.
  bool _lostIfUnanswered(PlayerClient client, Object error) {
    if (error is! PlayerException || !(error.isTimeout || error.unsent)) {
      return false;
    }
    if (_disposed || _client != client || _phase != ConnectionPhase.ready) {
      return false;
    }
    _lose();
    return true;
  }

  void _scheduleRetry() {
    final target = _target;
    if (target == null || _disposed) {
      return;
    }
    final delay =
        _backoff[_retries < _backoff.length ? _retries : _backoff.length - 1];
    _retries++;
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, () {
      _retryTimer = null;
      unawaited(_connect(target));
    });
  }

  Future<void> _dropConnection({bool keepState = false}) async {
    final subscriptions = List.of(_subscriptions);
    _subscriptions.clear();
    // Cancelling stops delivery at once; the future only reports the stream's
    // own clean-up.
    for (final subscription in subscriptions) {
      unawaited(subscription.cancel());
    }
    final client = _client;
    final transport = _transport;
    _client = null;
    _channel = null;
    _transport = null;
    _busy.clear();
    _syncing = false;
    levels.reset();
    await client?.dispose();
    await transport?.close();
    if (!keepState) {
      _status = null;
    }
  }

  void _cancelTimers() {
    _wifiProbe?.cancel();
    _wifiProbe = null;
    _retryTimer?.cancel();
    _retryTimer = null;
    _detailsTimer?.cancel();
    _detailsTimer = null;
  }

  // The status, and what is fetched because of it.

  void _onStatus(player.RemoteStatus status) {
    if (_disposed) {
      return;
    }
    _status = status;
    _lockedAnswer = null;
    _statusAt = _monotonic();
    _stage.heard();
    _ambientMode = status.hasAmbientMode() ? status.ambientMode : null;
    _ambientState = status.hasAmbientState() ? status.ambientState : null;
    clock.update(status);
    _edit?.noteStatus();
    _notify();
    levels.onStatus(status.hasGainsRevision() ? status.gainsRevision : null);
    if (_needsFetch(status)) {
      unawaited(_sync());
    }
  }

  /// Whether the song details or the playlist are not for what the status
  /// describes.
  bool _needsFetch(player.RemoteStatus status) =>
      status.songName != _detailsName || _entriesStale(status);

  /// Whether the playlist entries are not the playlist the status is on: a
  /// different playlist, or the same name with other songs at the position
  /// (it was edited). An edit that leaves the position's song alone goes
  /// unseen until the next change.
  bool _entriesStale(player.RemoteStatus status) {
    if (status.playlistName != _entriesPlaylist) {
      return true;
    }
    final at = status.hasPosition() ? status.position : null;
    final mismatch = at == null
        ? status.songName.isEmpty && _entries.isNotEmpty
        : at >= _entries.length || _entries[at].songName != status.songName;
    if (!mismatch) {
      return false;
    }
    // Asked once for each place the status can be at, so an answer that
    // never agrees with the status is not asked for four times a second.
    return _entriesKey(status) != _entriesAskedFor;
  }

  String _entriesKey(player.RemoteStatus status) =>
      '${status.playlistName}|${status.hasPosition() ? status.position : null}'
      '|${status.songName}';

  /// Fetches the song details and the playlist when the status has moved on
  /// to a song or playlist they are not for. A failure is tried again after
  /// [detailsRetryDelay].
  Future<void> _sync() async {
    if (_syncing) {
      _syncAgain = true;
      return;
    }
    _syncing = true;
    PlayerClient? asked;
    try {
      do {
        _syncAgain = false;
        final client = _client;
        asked = client;
        final status = _status;
        if (client == null || status == null) {
          break;
        }
        if (status.songName != _detailsName) {
          await _loadDetails(client, status.songName);
        }
        if (_entriesStale(status)) {
          _entriesAskedFor = _entriesKey(status);
          await _loadEntries(client, status.playlistName);
        }
      } while (_syncAgain && !_disposed && _client != null);
    } on Object catch (e) {
      if (asked != null && _lostIfUnanswered(asked, e)) {
        // Connecting again fetches everything.
      } else if (!_disposed && _client != null) {
        _detailsTimer?.cancel();
        _detailsTimer = Timer(detailsRetryDelay, () {
          _detailsTimer = null;
          unawaited(_sync());
        });
      }
    } finally {
      _syncing = false;
    }
  }

  Future<void> _loadDetails(PlayerClient client, String name) async {
    if (name.isEmpty) {
      _details = null;
      _detailsName = name;
      _notify();
      return;
    }
    final response = await client.status();
    if (_disposed || _client != client) {
      return;
    }
    if (response.hasCurrentSong()) {
      final song = response.currentSong;
      _details = SongDetails(
        song.name,
        song,
        resolveSections(
          song.sections,
          song.hasBeatGrid() ? song.beatGrid : null,
          song.hasDuration() ? song.duration.toDart() : Duration.zero,
        ),
      );
    } else {
      _details = null;
    }
    _detailsName = name;
    _notify();
  }

  Future<void> _loadEntries(PlayerClient client, String playlist) async {
    final response = await client.getPlaylist();
    if (_disposed || _client != client) {
      return;
    }
    _keepEntries(response);
    _entriesPlaylist = playlist;
    _notify();
  }

  /// Keeps the active playlist as [response] has it: its entries, and for
  /// the editor its ambient light and revision.
  void _keepEntries(player.GetPlaylistResponse response) {
    _entries = response.entries;
    _entriesAmbient = response.hasAmbient() ? response.ambient : null;
    _entriesRevision = response.revision;
  }

  /// Whether the player reads and sets playlist settings (each song's
  /// fades, auto play and ambient light, and the playlist's): a playlist it
  /// sent had a revision. A player from before sends none, and its editor
  /// edits songs and their order only.
  bool get playlistSettingsAvailable => _entriesRevision.isNotEmpty;

  /// Fetches the list of playlists, for the picker. Returns the failure in
  /// words, or null.
  Future<String?> loadPlaylists() async {
    final client = _client;
    if (client == null || !isReady) {
      return _notice(notConnectedMessage);
    }
    _loadingChoices = true;
    _notify();
    try {
      final response = await client.listPlaylists();
      if (_disposed) {
        return null;
      }
      _choices = PlaylistChoices(response.names, response.active);
      return null;
    } on Object catch (e) {
      return _notice(describeFailure(e));
    } finally {
      _loadingChoices = false;
      _notify();
    }
  }

  // Scanning.

  /// Starts a scan for players. Checks the adapter first, so Bluetooth being
  /// off or refused shows as such instead of as an empty scan. A scan the
  /// person asked for clears the reason the last connection failed; one the
  /// screen started by itself, on coming back to the choice of players
  /// because of it, [keepError] does not.
  Future<void> startScan({bool keepError = false}) async {
    stopScan();
    _found = const [];
    if (!keepError) {
      _connectError = null;
    }
    final adapter = await _link.adapterState();
    if (_disposed) {
      return;
    }
    _adapter = adapter;
    if (adapter != AdapterState.ready) {
      _notify();
      return;
    }
    _scanning = true;
    _notify();
    final seen = <String, DiscoveredPlayer>{};
    _scan = _link.scan().listen(
      (found) {
        seen[found.id] = found;
        _found = seen.values.toList()..sort((a, b) => b.rssi.compareTo(a.rssi));
        _notify();
      },
      onError: (Object error) {
        _scanning = false;
        _scan = null;
        _connectError = describeConnectFailure(error, _adapter);
        _notify();
      },
      onDone: () {
        _scanning = false;
        _scan = null;
        _notify();
      },
      cancelOnError: true,
    );
  }

  /// Ends a scan.
  void stopScan() {
    final scan = _scan;
    _scan = null;
    if (scan != null) {
      unawaited(scan.cancel());
    }
    if (_scanning) {
      _scanning = false;
      _notify();
    }
  }

  /// Asks for the Bluetooth permissions again, and scans if they are given.
  Future<void> requestAccess() async {
    final adapter = await _link.requestAccess();
    if (_disposed) {
      return;
    }
    _adapter = adapter;
    _notify();
    if (adapter == AdapterState.ready) {
      await startScan();
    }
  }

  // Actions. Each returns the failure in words, or null when it worked; the
  // same words go to [notices].

  /// Plays the current song.
  Future<String?> play() => _act(ActionKind.play, (c) => c.play());

  /// Stops the song that is playing.
  Future<String?> stop() => _act(ActionKind.stop, (c) => c.stop());

  /// Moves to the next song.
  Future<String?> next() => _act(ActionKind.next, (c) => c.next());

  /// Moves to the previous song.
  Future<String?> previous() => _act(ActionKind.previous, (c) => c.previous());

  /// Makes the playlist entry at [position], which the caller sees holding
  /// the song [name], the current one without playing it. Does nothing when
  /// the player is already on it. When the player says the playlist has
  /// changed, the playlist is fetched again.
  Future<String?> navigateTo(int position, String name) {
    if (position == this.position && isReady) {
      return Future<String?>.value();
    }
    return _act(ActionKind.navigate, (c) async {
      try {
        return await c.navigateToSong(position, name);
      } on PlayerException catch (e) {
        if (e.code == GrpcCode.failedPrecondition &&
            e.message.contains('playlist has changed')) {
          _entriesPlaylist = null;
          unawaited(_sync());
        }
        rethrow;
      }
    });
  }

  /// Switches the player to the playlist named [name].
  Future<String?> switchPlaylist(String name) =>
      _act(ActionKind.switchPlaylist, (c) => c.switchToPlaylist(name));

  /// Jumps to the start of the section named [name].
  Future<String?> seekToSection(String name) =>
      _act(ActionKind.seekSection, (c) => c.seekToSection(name));

  /// Loops the section named [name].
  Future<String?> loopSection(String name) =>
      _act(ActionKind.loopSection, (c) => c.loopSection(name));

  /// Ends the section loop.
  Future<String?> stopLoop() =>
      _act(ActionKind.stopLoop, (c) => c.stopSectionLoop());

  /// Calls off the song waiting to start by itself.
  Future<String?> cancelAutoPlay() =>
      _act(ActionKind.cancelAutoPlay, (c) => c.cancelAutoPlay());

  /// Sets the ambient light's mode to `auto`, `on` or `off`. What the player
  /// answers is shown at once; the status then keeps it current.
  Future<String?> setAmbientMode(String mode) =>
      _act(ActionKind.setAmbient, (c) async {
        final response = await c.setAmbientMode(mode);
        if (response.hasAmbient() && !_disposed && _client == c) {
          _ambientMode = response.ambient.mode;
          _ambientState = response.ambient.state;
        }
        return response;
      });

  /// Locks the player, or unlocks it. What the player answers is shown at
  /// once; the status then keeps it current.
  Future<String?> setLock(bool locked) => _act(ActionKind.setLock, (c) async {
    final response = await c.setLock(locked);
    if (!_disposed && _client == c) {
      _lockedAnswer = response.locked;
      _edit?.noteStatus();
    }
    return response;
  });

  /// Begins editing the active playlist, as loaded. Returns why not, or null.
  String? startEditing() {
    final status = _status;
    if (_edit != null) {
      return null;
    }
    if (!canEditPlaylist || status == null) {
      return _notice('Unlock the player to edit a playlist.');
    }
    if (_entriesPlaylist != status.playlistName) {
      return _notice('The playlist has not loaded yet.');
    }
    _beginEdit(
      status.playlistName,
      _entries,
      ambient: _entriesAmbient,
      revision: _entriesRevision,
      isNew: false,
    );
    return null;
  }

  /// Begins a new, empty playlist called [name]. It is made on the player
  /// when the edit is saved. Returns what is wrong with the name, or null.
  String? startNewPlaylist(String name) {
    if (_edit != null) {
      return null;
    }
    final trimmed = name.trim();
    final bad = playlistNameProblem(trimmed);
    if (bad != null) {
      return bad;
    }
    if (_choices?.names.contains(trimmed) ?? false) {
      return nameTakenMessage;
    }
    _beginEdit(trimmed, const [], isNew: true);
    return null;
  }

  void _beginEdit(
    String name,
    List<player.PlaylistEntry> entries, {
    required bool isNew,
    player.AmbientSetting? ambient,
    String revision = '',
  }) {
    _edit = PlaylistEdit(
      name: name,
      entries: entries,
      ambient: ambient,
      revision: revision,
      // Known from the playlist the player last sent, which a new playlist
      // has not got.
      supportsSettings: playlistSettingsAvailable,
      isNew: isNew,
      client: () => _client,
      isReady: () => isReady,
      locked: () => locked,
      notice: _notice,
      lostIfUnanswered: _lostIfUnanswered,
      saved: _playlistSaved,
    );
    _notify();
  }

  /// Leaves the editor, dropping what was changed.
  void endEditing() {
    _endEdit();
    _notify();
  }

  void _endEdit() {
    final edit = _edit;
    _edit = null;
    if (edit != null) {
      // Disposed after the screen has let go of it.
      scheduleMicrotask(edit.dispose);
    }
  }

  void _playlistSaved(PlaylistEdit edit, player.GetPlaylistResponse response) {
    if (!edit.isNew && response.name == _status?.playlistName) {
      _keepEntries(response);
      _entriesPlaylist = response.name;
    }
    _choices = null;
    if (edit.isNew) {
      _notice(
        'Saved ${response.name}. Choose it in the playlist list to use it.',
      );
    }
    _endEdit();
    _notify();
  }

  /// Deletes the playlist [name] once [confirm] says to. Refused here, with
  /// the reason, for the playlist in use and for the player's own list of
  /// every song. Returns the failure in words, or null.
  Future<String?> deletePlaylist(
    String name, {
    required Future<bool> Function() confirm,
  }) async {
    if (name == allSongsName) {
      return _notice('The $allSongsName playlist cannot be deleted.');
    }
    if (name == (_status?.playlistName ?? _choices?.active)) {
      return _notice('Switch to another playlist before deleting this one.');
    }
    if (!await confirm() || _disposed) {
      return null;
    }
    final failure = await _act(
      ActionKind.deletePlaylist,
      (c) => c.deletePlaylist(name),
    );
    if (failure == null) {
      await loadPlaylists();
    }
    return failure;
  }

  Future<String?> _act(
    ActionKind kind,
    Future<Object?> Function(PlayerClient) call,
  ) async {
    final client = _client;
    if (client == null || !isReady) {
      return _notice(notConnectedMessage);
    }
    if (!_busy.add(kind)) {
      // A request for this is already out: a second tap is not a second
      // request.
      return null;
    }
    _notify();
    try {
      await call(client);
      return null;
    } on Object catch (e) {
      _lostIfUnanswered(client, e);
      return _notice(describeFailure(e));
    } finally {
      _busy.remove(kind);
      _notify();
    }
  }

  String _notice(String message) {
    if (!_disposed) {
      _notices.add(message);
    }
    return message;
  }

  // Housekeeping.

  /// Tells the session whether the app is in the foreground. The screen is
  /// kept on only while it is, and a player is connected.
  void setForeground(bool foreground) {
    _foreground = foreground;
    _updateWake();
  }

  void _setPhase(ConnectionPhase phase) {
    _phase = phase;
    _updateWake();
    _stage.linkChanged(ready: phase == ConnectionPhase.ready);
    lightingAssets.linkChanged(ready: phase == ConnectionPhase.ready);
    _notify();
    levels.check();
  }

  void _updateWake() {
    final wanted = _foreground && _phase == ConnectionPhase.ready;
    if (_wakeAsked == wanted) {
      return;
    }
    _wakeAsked = wanted;
    final wake = _wake;
    if (wake != null) {
      unawaited(wake.setEnabled(wanted).catchError((Object _) {}));
    }
  }

  void _notify() {
    if (!_disposed) {
      notifyListeners();
    }
  }

  @override
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    _epoch++;
    _cancelTimers();
    unawaited(_scan?.cancel());
    _scan = null;
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    _subscriptions.clear();
    final client = _client;
    final transport = _transport;
    _client = null;
    _channel = null;
    _transport = null;
    if (client != null) {
      unawaited(client.dispose());
    }
    if (transport != null) {
      unawaited(transport.close());
    }
    final wake = _wake;
    if (wake != null && _wakeAsked == true) {
      unawaited(wake.setEnabled(false).catchError((Object _) {}));
    }
    levels.dispose();
    _stage.dispose();
    lightingAssets
      ..removeListener(_assetsChanged)
      ..dispose();
    songLibrary
      ..removeListener(_libraryChanged)
      ..dispose();
    _edit?.dispose();
    _edit = null;
    unawaited(_notices.close());
    super.dispose();
  }
}

Monotonic _stopwatchClock() {
  final stopwatch = Stopwatch()..start();
  return () => stopwatch.elapsed;
}
