//! Follows MPRIS media players on the D-Bus session bus.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::fdo::DBusProxy;
use zbus::message::Type as MessageType;
use zbus::names::BusName;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

use lyri_core::lyrics::{LyricsQuery, TrackKey};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Players don't signal position during normal playback, so resync now and then.
const RESYNC_INTERVAL: Duration = Duration::from_secs(2);
/// Resyncs landing slightly behind the extrapolated position are jitter, not seeks.
const BACKWARD_JITTER: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub key: TrackKey,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<Duration>,
    pub position: Duration,
    pub is_playing: bool,
    pub can_seek: bool,
    playback_rate: f64,
    observed_at: Instant,
}

impl NowPlaying {
    pub fn current_position(&self) -> Duration {
        let position = if self.is_playing {
            self.position + self.observed_at.elapsed().mul_f64(self.playback_rate)
        } else {
            self.position
        };

        self.duration
            .map_or(position, |duration| position.min(duration))
    }

    pub fn lyrics_query(&self) -> LyricsQuery {
        LyricsQuery {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration: self.duration,
        }
    }

    /// Playback mirrored from a remote Lyri host; positions extrapolate from
    /// the moment the state was received.
    pub fn remote(
        title: String,
        artist: String,
        album: String,
        duration: Option<Duration>,
        position: Duration,
        is_playing: bool,
        can_seek: bool,
    ) -> Self {
        Self {
            key: TrackKey::new(&title, &artist),
            title,
            artist,
            album,
            duration,
            position,
            is_playing,
            can_seek,
            playback_rate: 1.0,
            observed_at: Instant::now(),
        }
    }

    fn smooth_with(&mut self, previous: &Self) {
        if self.key != previous.key || !self.is_playing || !previous.is_playing {
            return;
        }
        let expected = previous.current_position();
        if self.position < expected && expected - self.position < BACKWARD_JITTER {
            self.position = expected;
            self.observed_at = Instant::now();
        }
    }
}

#[derive(Clone, Debug)]
pub struct NowPlayingController {
    tx: UnboundedSender<WatchCommand>,
}

impl NowPlayingController {
    pub fn seek_to(&self, key: TrackKey, position: Duration) -> bool {
        self.tx.send(WatchCommand::SeekTo { key, position }).is_ok()
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(WatchCommand::Shutdown);
    }
}

#[derive(Clone, Debug)]
pub enum NowPlayingEvent {
    Changed(Option<NowPlaying>),
    Error(String),
}

#[derive(Clone, Debug)]
enum WatchCommand {
    SeekTo { key: TrackKey, position: Duration },
    Shutdown,
}

type Emit = Arc<dyn Fn(NowPlayingEvent) + Send + Sync>;

pub fn spawn_now_playing_events(
    runtime: &Handle,
    emit: impl Fn(NowPlayingEvent) + Send + Sync + 'static,
) -> NowPlayingController {
    let (tx, rx) = unbounded_channel();
    let emit: Emit = Arc::new(emit);

    runtime.spawn(async move {
        if let Err(error) = watch(rx, emit.clone()).await {
            emit(NowPlayingEvent::Error(format!("D-Bus: {error}")));
        }
    });

    NowPlayingController { tx }
}

async fn watch(mut commands: UnboundedReceiver<WatchCommand>, emit: Emit) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let player_rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .path(MPRIS_PATH)?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg0ns("org.mpris.MediaPlayer2")?
        .build();
    let mut player_signals = MessageStream::for_match_rule(player_rule, &connection, None).await?;
    let mut owner_signals = MessageStream::for_match_rule(owner_rule, &connection, None).await?;
    let mut resync = tokio::time::interval(RESYNC_INTERVAL);

    let mut watcher = Watcher {
        dbus: DBusProxy::new(&connection).await?,
        connection,
        emit,
        current: None,
        last: None,
    };

    loop {
        let seeked = tokio::select! {
            command = commands.recv() => match command {
                Some(WatchCommand::SeekTo { key, position }) => {
                    watcher.seek_to(&key, position).await;
                    continue;
                }
                Some(WatchCommand::Shutdown) | None => return Ok(()),
            },
            Some(message) = player_signals.next() => seeked(&message),
            Some(_) = owner_signals.next() => None,
            _ = resync.tick() => None,
        };
        watcher.refresh(seeked).await;
    }
}

/// A player's `Seeked` signal, which carries an exact position.
struct Seeked {
    sender: String,
    position: Duration,
}

fn seeked(message: &zbus::Result<Message>) -> Option<Seeked> {
    let message = message.as_ref().ok()?;
    let header = message.header();
    if header.member()?.as_str() != "Seeked" {
        return None;
    }
    let micros: i64 = message.body().deserialize().ok()?;
    Some(Seeked {
        sender: header.sender()?.to_string(),
        position: Duration::from_micros(micros.max(0) as u64),
    })
}

/// The player being followed, for routing seeks.
struct Followed {
    bus_name: String,
    track_id: Option<ObjectPath<'static>>,
    key: Option<TrackKey>,
}

struct Watcher {
    connection: Connection,
    dbus: DBusProxy<'static>,
    emit: Emit,
    current: Option<Followed>,
    last: Option<NowPlaying>,
}

impl Watcher {
    async fn refresh(&mut self, seeked: Option<Seeked>) {
        let players = match self.players().await {
            Ok(players) => players,
            Err(error) => return (self.emit)(NowPlayingEvent::Error(format!("D-Bus: {error}"))),
        };

        let current = self
            .current
            .as_ref()
            .map(|followed| followed.bus_name.as_str());
        let chosen = pick_player(players, current);
        let mut playback = chosen.as_ref().and_then(|player| player.playback.clone());
        // Some players (Firefox) floor the Position property to whole
        // seconds; the Seeked signal is exact, so prefer it.
        if let (Some(seeked), Some(player), Some(next)) = (&seeked, &chosen, &mut playback) {
            if self.is_owner(&player.bus_name, &seeked.sender).await {
                next.position = seeked.position;
                next.observed_at = Instant::now();
            }
        } else if let (Some(next), Some(previous)) = (&mut playback, &self.last) {
            next.smooth_with(previous);
        }

        self.current = chosen.map(|player| Followed {
            key: player.playback.map(|playback| playback.key),
            bus_name: player.bus_name,
            track_id: player.track_id,
        });
        self.last = playback.clone();
        (self.emit)(NowPlayingEvent::Changed(playback));
    }

    /// Signals name their sender by unique name, players by well-known name.
    async fn is_owner(&self, bus_name: &str, unique_name: &str) -> bool {
        let Ok(name) = BusName::try_from(bus_name) else {
            return false;
        };
        self.dbus
            .get_name_owner(name)
            .await
            .is_ok_and(|owner| owner.as_str() == unique_name)
    }

    async fn players(&self) -> zbus::Result<Vec<PlayerSnapshot>> {
        let mut players = Vec::new();
        for name in self.dbus.list_names().await? {
            if !name.starts_with(MPRIS_PREFIX) {
                continue;
            }
            // A player can vanish between listing and querying; skip it.
            if let Ok(player) = snapshot_player(&self.connection, name.as_str()).await {
                players.push(player);
            }
        }
        Ok(players)
    }

    async fn seek_to(&self, key: &TrackKey, position: Duration) {
        let Some(followed) = &self.current else {
            return;
        };
        if followed.key.as_ref() != Some(key) {
            return;
        }

        let bus_name = followed.bus_name.as_str();
        let target = to_micros(position);
        let result = match &followed.track_id {
            Some(track_id) => {
                self.connection
                    .call_method(
                        Some(bus_name),
                        MPRIS_PATH,
                        Some(PLAYER_INTERFACE),
                        "SetPosition",
                        &(track_id, target),
                    )
                    .await
            }
            // Without a track id only relative seeks are possible.
            None => {
                let current = self.last.as_ref().map(NowPlaying::current_position);
                let offset = target - to_micros(current.unwrap_or_default());
                self.connection
                    .call_method(
                        Some(bus_name),
                        MPRIS_PATH,
                        Some(PLAYER_INTERFACE),
                        "Seek",
                        &(offset,),
                    )
                    .await
            }
        };
        if let Err(error) = result {
            (self.emit)(NowPlayingEvent::Error(format!("Seek failed: {error}")));
        }
    }
}

fn to_micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

struct PlayerSnapshot {
    bus_name: String,
    track_id: Option<ObjectPath<'static>>,
    /// `None` while the player has nothing loaded.
    playback: Option<NowPlaying>,
}

impl PlayerSnapshot {
    fn is_playing(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing)
    }
}

/// Sticks with the current player while it plays, otherwise switches to
/// whichever player is playing, like the system media controls do.
fn pick_player(mut players: Vec<PlayerSnapshot>, current: Option<&str>) -> Option<PlayerSnapshot> {
    let is_current = |player: &PlayerSnapshot| Some(player.bus_name.as_str()) == current;
    let index = players
        .iter()
        .position(|player| is_current(player) && player.is_playing())
        .or_else(|| players.iter().position(PlayerSnapshot::is_playing))
        .or_else(|| players.iter().position(is_current))
        .or_else(|| players.iter().position(|player| player.playback.is_some()))?;
    Some(players.swap_remove(index))
}

async fn snapshot_player(connection: &Connection, bus_name: &str) -> zbus::Result<PlayerSnapshot> {
    let reply = connection
        .call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &(PLAYER_INTERFACE,),
        )
        .await?;
    let properties: HashMap<String, OwnedValue> = reply.body().deserialize()?;
    let property = |name: &str| properties.get(name).map(|value| unwrap_variant(value));

    let metadata: HashMap<String, OwnedValue> = properties
        .get("Metadata")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| HashMap::try_from(value).ok())
        .unwrap_or_default();
    let field = |name: &str| metadata.get(name).map(|value| unwrap_variant(value));

    let title = text(field("xesam:title"));
    let artist = artists(field("xesam:artist"));
    let track_id = match field("mpris:trackid") {
        Some(Value::ObjectPath(path)) => Some(path.to_owned()),
        Some(Value::Str(path)) => ObjectPath::try_from(path.to_string()).ok(),
        _ => None,
    };

    let playback = (!title.trim().is_empty() || !artist.trim().is_empty()).then(|| {
        let duration = micros(field("mpris:length"))
            .filter(|length| *length > 0)
            .map(|length| Duration::from_micros(length as u64));
        let position = micros(property("Position")).unwrap_or(0).max(0) as u64;
        let playback_rate = match property("Rate") {
            Some(Value::F64(rate)) if rate.is_finite() && *rate > 0.0 => *rate,
            _ => 1.0,
        };

        NowPlaying {
            key: TrackKey::new(&title, &artist),
            album: text(field("xesam:album")),
            title,
            artist,
            duration,
            position: Duration::from_micros(position),
            is_playing: matches!(property("PlaybackStatus"), Some(Value::Str(status)) if status.as_str() == "Playing"),
            can_seek: matches!(property("CanSeek"), Some(Value::Bool(true))),
            playback_rate,
            observed_at: Instant::now(),
        }
    });

    Ok(PlayerSnapshot {
        bus_name: bus_name.to_string(),
        track_id,
        playback,
    })
}

fn unwrap_variant<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => unwrap_variant(inner),
        value => value,
    }
}

fn text(value: Option<&Value<'_>>) -> String {
    match value {
        Some(Value::Str(text)) => text.to_string(),
        _ => String::new(),
    }
}

fn artists(value: Option<&Value<'_>>) -> String {
    match value {
        Some(Value::Array(artists)) => artists
            .iter()
            .filter_map(|artist| match unwrap_variant(artist) {
                Value::Str(artist) => Some(artist.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        value => text(value),
    }
}

/// MPRIS times are microseconds, but players disagree on the integer type.
fn micros(value: Option<&Value<'_>>) -> Option<i64> {
    match value? {
        Value::I64(value) => Some(*value),
        Value::U64(value) => i64::try_from(*value).ok(),
        Value::I32(value) => Some(i64::from(*value)),
        Value::U32(value) => Some(i64::from(*value)),
        Value::F64(value) if value.is_finite() => Some(*value as i64),
        _ => None,
    }
}
