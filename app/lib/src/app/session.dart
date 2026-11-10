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

import '../../gen/ble/v1/ble.pb.dart' as ble;
import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/playback_clock.dart';
import '../protocol/player_client.dart';
import '../protocol/player_exception.dart';
import '../protocol/transport.dart';
import 'failure.dart';
import 'player_link.dart';
import 'ports.dart';
import 'sections.dart';

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

/// How long to wait before asking again for song or playlist details that
/// could not be fetched.
const Duration detailsRetryDelay = Duration(seconds: 3);

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
  /// A session over [link], remembering its player in [store].
  Session({
    required this._link,
    required this._store,
    this._wake,
    Monotonic? monotonic,
    this._backoff = reconnectBackoff,
    PlayerClient Function(PlayerTransport)? clientFactory,
  }) : _monotonic = monotonic ?? _stopwatchClock(),
       _clientFactory = clientFactory ?? PlayerClient.new {
    clock = PlaybackClock(_monotonic);
  }

  final PlayerLink _link;
  final PlayerStore _store;
  final ScreenWake? _wake;
  final Monotonic _monotonic;
  final List<Duration> _backoff;
  final PlayerClient Function(PlayerTransport) _clientFactory;

  /// The song position to show, run between status messages.
  late final PlaybackClock clock;

  final StreamController<String> _notices = StreamController<String>.broadcast(
    sync: true,
  );

  ConnectionPhase _phase = ConnectionPhase.noPlayer;
  RememberedPlayer? _target;
  String? _connectError;
  AdapterState _adapter = AdapterState.ready;

  PlayerTransport? _transport;
  PlayerClient? _client;
  final List<StreamSubscription<Object?>> _subscriptions = [];

  ble.Status? _status;
  Duration _statusAt = Duration.zero;
  SongDetails? _details;
  String? _detailsName;
  List<player.PlaylistEntry> _entries = const [];
  String? _entriesPlaylist;
  String? _entriesAskedFor;
  PlaylistChoices? _choices;
  bool _loadingChoices = false;

  final Set<ActionKind> _busy = {};

  int _epoch = 0;
  int _retries = 0;
  Timer? _retryTimer;
  Timer? _detailsTimer;
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

  /// Whether Bluetooth can be used, as last checked.
  AdapterState get adapter => _adapter;

  /// The latest status from the player. Kept, and shown dimmed, while the
  /// link is lost.
  ble.Status? get status => _status;

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
  /// remembers it once the connection is up.
  Future<void> connectTo(DiscoveredPlayer chosen) {
    stopScan();
    _connectError = null;
    final target = RememberedPlayer(id: chosen.id, name: chosen.name);
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
    _target = null;
    _connectError = null;
    _status = null;
    _details = null;
    _detailsName = null;
    _entries = const [];
    _entriesPlaylist = null;
    _entriesAskedFor = null;
    _choices = null;
    clock.reset();
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
      final client = _clientFactory(transport);
      _transport = transport;
      _client = client;
      _subscriptions
        ..add(client.statusStream.listen(_onStatus))
        ..add(transport.state.listen(_onTransportState));
      final first = await client.readStatus();
      if (_stale(epoch)) {
        return;
      }
      _onStatus(first);
      await _sync();
      if (_stale(epoch)) {
        return;
      }
      if (transport.currentState != TransportState.ready) {
        throw const PlayerException.unavailable('connection lost');
      }
      await _store.save(target);
      _retries = 0;
      _connectError = null;
      _setPhase(ConnectionPhase.ready);
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
        _connectError = describeConnectFailure(e, _adapter);
        _target = null;
        _setPhase(ConnectionPhase.noPlayer);
      } else {
        _setPhase(ConnectionPhase.lost);
        _scheduleRetry();
      }
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
    _transport = null;
    _busy.clear();
    _syncing = false;
    await client?.dispose();
    await transport?.close();
    if (!keepState) {
      _status = null;
    }
  }

  void _cancelTimers() {
    _retryTimer?.cancel();
    _retryTimer = null;
    _detailsTimer?.cancel();
    _detailsTimer = null;
  }

  // The status, and what is fetched because of it.

  void _onStatus(ble.Status status) {
    if (_disposed) {
      return;
    }
    _status = status;
    _statusAt = _monotonic();
    clock.update(status);
    _notify();
    if (_needsFetch(status)) {
      unawaited(_sync());
    }
  }

  /// Whether the song details or the playlist are not for what the status
  /// describes.
  bool _needsFetch(ble.Status status) =>
      status.songName != _detailsName || _entriesStale(status);

  /// Whether the playlist entries are not the playlist the status is on: a
  /// different playlist, or the same name with other songs at the position
  /// (it was edited). An edit that leaves the position's song alone goes
  /// unseen until the next change.
  bool _entriesStale(ble.Status status) {
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

  String _entriesKey(ble.Status status) =>
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
    _entries = response.entries;
    _entriesPlaylist = playlist;
    _notify();
  }

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
  /// off or refused shows as such instead of as an empty scan.
  Future<void> startScan() async {
    stopScan();
    _found = const [];
    _connectError = null;
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
    _notify();
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
    unawaited(_notices.close());
    super.dispose();
  }
}

Monotonic _stopwatchClock() {
  final stopwatch = Stopwatch()..start();
  return () => stopwatch.elapsed;
}
