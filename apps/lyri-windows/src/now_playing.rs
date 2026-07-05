use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
    PlaybackInfoChangedEventArgs, SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::Result;

use lyri_core::lyrics::{LyricsQuery, TrackKey};

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
}

#[derive(Clone, Debug)]
pub struct NowPlayingController {
    tx: mpsc::Sender<WatchCommand>,
}

impl NowPlayingController {
    pub fn seek_to(&self, key: TrackKey, position: Duration) -> bool {
        self.tx.send(WatchCommand::SeekTo { key, position }).is_ok()
    }
}

#[derive(Clone, Debug)]
pub enum NowPlayingEvent {
    Changed(Option<NowPlaying>),
    Error(String),
}

#[derive(Clone, Debug)]
enum WatchCommand {
    Refresh,
    Rebind,
    SeekTo { key: TrackKey, position: Duration },
}

type Emit = Arc<dyn Fn(NowPlayingEvent) + Send + Sync>;
type ManagerHandler<TArgs> =
    TypedEventHandler<GlobalSystemMediaTransportControlsSessionManager, TArgs>;
type SessionHandler<TArgs> = TypedEventHandler<GlobalSystemMediaTransportControlsSession, TArgs>;

pub fn spawn_now_playing_events(
    emit: impl Fn(NowPlayingEvent) + Send + Sync + 'static,
) -> NowPlayingController {
    let (tx, rx) = mpsc::channel();
    let controller = NowPlayingController { tx: tx.clone() };
    let emit = Arc::new(emit);
    let thread_emit = emit.clone();

    std::thread::spawn(move || {
        if let Err(error) = NowPlayingWatcher::run(tx, rx, emit) {
            thread_emit(NowPlayingEvent::Error(error.to_string()));
        }
    });

    controller
}

struct NowPlayingWatcher {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    tx: mpsc::Sender<WatchCommand>,
    rx: mpsc::Receiver<WatchCommand>,
    emit: Emit,
    _manager_events: ManagerEvents,
    session: Option<SessionRegistration>,
    current_key: Option<TrackKey>,
}

impl NowPlayingWatcher {
    fn run(
        tx: mpsc::Sender<WatchCommand>,
        rx: mpsc::Receiver<WatchCommand>,
        emit: Emit,
    ) -> Result<()> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;

        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;

        let manager_events = ManagerEvents::new(&manager, &tx, emit.as_ref());
        let mut watcher = Self {
            manager,
            tx,
            rx,
            emit,
            _manager_events: manager_events,
            session: None,
            current_key: None,
        };

        watcher.rebind();

        while let Ok(command) = watcher.rx.recv() {
            match command {
                WatchCommand::Refresh => watcher.refresh(),
                WatchCommand::Rebind => watcher.rebind(),
                WatchCommand::SeekTo { key, position } => watcher.seek_to(&key, position),
            }
        }

        Ok(())
    }

    fn rebind(&mut self) {
        self.session = self
            .manager
            .GetCurrentSession()
            .ok()
            .map(|session| SessionRegistration::new(session, &self.tx, self.emit.as_ref()));
        self.refresh();
    }

    fn refresh(&mut self) {
        let Some(registration) = &self.session else {
            self.current_key = None;
            self.emit_changed(None);
            return;
        };

        match snapshot_session(&registration.session) {
            Ok(Some(playback)) => {
                self.current_key = Some(playback.key.clone());
                self.emit_changed(Some(playback));
            }
            Ok(None) => {
                if self.current_key.is_none() {
                    self.emit_changed(None);
                }
            }
            Err(error) => self.emit_error(error),
        }
    }

    fn seek_to(&self, key: &TrackKey, position: Duration) {
        if Some(key) != self.current_key.as_ref() {
            return;
        }

        if let Some(registration) = &self.session
            && let Err(error) = seek_session(&registration.session, position)
        {
            self.emit_error(error);
        }
    }

    fn emit_changed(&self, playback: Option<NowPlaying>) {
        (self.emit)(NowPlayingEvent::Changed(playback));
    }

    fn emit_error(&self, error: windows::core::Error) {
        (self.emit)(NowPlayingEvent::Error(error.to_string()));
    }
}

struct ManagerEvents {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    current_token: Option<i64>,
    sessions_token: Option<i64>,
    _current_handler: ManagerHandler<CurrentSessionChangedEventArgs>,
    _sessions_handler: ManagerHandler<SessionsChangedEventArgs>,
}

/// Builds a handler that forwards `command` to the watcher and registers it
/// through `register`, reporting registration failures via `emit`.
fn watch_event<TSource, TArgs>(
    tx: &mpsc::Sender<WatchCommand>,
    command: WatchCommand,
    emit: &dyn Fn(NowPlayingEvent),
    register: impl FnOnce(&TypedEventHandler<TSource, TArgs>) -> Result<i64>,
) -> (Option<i64>, TypedEventHandler<TSource, TArgs>)
where
    TSource: windows::core::RuntimeType,
    TArgs: windows::core::RuntimeType,
{
    let tx = tx.clone();
    let handler = TypedEventHandler::new(move |_, _| {
        let _ = tx.send(command.clone());
        Ok(())
    });
    let token = register(&handler)
        .map_err(|error| emit(NowPlayingEvent::Error(error.to_string())))
        .ok();

    (token, handler)
}

impl ManagerEvents {
    fn new(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        tx: &mpsc::Sender<WatchCommand>,
        emit: &dyn Fn(NowPlayingEvent),
    ) -> Self {
        let (current_token, current_handler) =
            watch_event(tx, WatchCommand::Rebind, emit, |handler| {
                manager.CurrentSessionChanged(handler)
            });
        let (sessions_token, sessions_handler) =
            watch_event(tx, WatchCommand::Rebind, emit, |handler| {
                manager.SessionsChanged(handler)
            });

        Self {
            manager: manager.clone(),
            current_token,
            sessions_token,
            _current_handler: current_handler,
            _sessions_handler: sessions_handler,
        }
    }
}

impl Drop for ManagerEvents {
    fn drop(&mut self) {
        if let Some(token) = self.current_token {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
        if let Some(token) = self.sessions_token {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
    }
}

struct SessionRegistration {
    session: GlobalSystemMediaTransportControlsSession,
    timeline_token: Option<i64>,
    playback_token: Option<i64>,
    media_token: Option<i64>,
    _timeline_handler: SessionHandler<TimelinePropertiesChangedEventArgs>,
    _playback_handler: SessionHandler<PlaybackInfoChangedEventArgs>,
    _media_handler: SessionHandler<MediaPropertiesChangedEventArgs>,
}

impl SessionRegistration {
    fn new(
        session: GlobalSystemMediaTransportControlsSession,
        tx: &mpsc::Sender<WatchCommand>,
        emit: &dyn Fn(NowPlayingEvent),
    ) -> Self {
        let (timeline_token, timeline_handler) =
            watch_event(tx, WatchCommand::Refresh, emit, |handler| {
                session.TimelinePropertiesChanged(handler)
            });
        let (playback_token, playback_handler) =
            watch_event(tx, WatchCommand::Refresh, emit, |handler| {
                session.PlaybackInfoChanged(handler)
            });
        let (media_token, media_handler) =
            watch_event(tx, WatchCommand::Refresh, emit, |handler| {
                session.MediaPropertiesChanged(handler)
            });

        Self {
            session,
            timeline_token,
            playback_token,
            media_token,
            _timeline_handler: timeline_handler,
            _playback_handler: playback_handler,
            _media_handler: media_handler,
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        if let Some(token) = self.timeline_token {
            let _ = self.session.RemoveTimelinePropertiesChanged(token);
        }
        if let Some(token) = self.playback_token {
            let _ = self.session.RemovePlaybackInfoChanged(token);
        }
        if let Some(token) = self.media_token {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
    }
}

fn snapshot_session(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<Option<NowPlaying>> {
    let properties = session.TryGetMediaPropertiesAsync()?.join()?;

    let raw_title = properties.Title()?;
    let raw_artist = properties.Artist()?;

    if raw_title.is_empty() && raw_artist.is_empty() {
        return Ok(None);
    }

    let playback_info = session.GetPlaybackInfo()?;
    let playback_rate = playback_info
        .PlaybackRate()
        .ok()
        .and_then(|rate| rate.Value().ok())
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .unwrap_or(1.0);
    let timeline = session.GetTimelineProperties()?;
    let duration =
        duration_from_timeline(timeline.StartTime()?.Duration, timeline.EndTime()?.Duration);
    let title = raw_title.to_string();
    let artist = raw_artist.to_string();
    let is_playing = playback_info.PlaybackStatus()?
        == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

    Ok(Some(NowPlaying {
        key: TrackKey::new(&title, &artist),
        title,
        artist,
        album: properties.AlbumTitle()?.to_string(),
        duration,
        position: adjusted_position(
            timeline.Position()?.Duration,
            timeline.LastUpdatedTime()?.UniversalTime,
            duration,
            is_playing,
            playback_rate,
        ),
        is_playing,
        can_seek: playback_info
            .Controls()
            .and_then(|controls| controls.IsPlaybackPositionEnabled())
            .unwrap_or(false),
        playback_rate,
        observed_at: Instant::now(),
    }))
}

fn seek_session(
    session: &GlobalSystemMediaTransportControlsSession,
    position: Duration,
) -> Result<bool> {
    session
        .TryChangePlaybackPositionAsync(duration_to_timespan(position))?
        .join()
}

fn duration_from_timeline(start: i64, end: i64) -> Option<Duration> {
    (end > start).then(|| timespan_to_duration(end - start))
}

fn timespan_to_duration(ticks: i64) -> Duration {
    if ticks <= 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(ticks as u64 * 100)
    }
}

fn duration_to_timespan(duration: Duration) -> i64 {
    (duration.as_nanos() / 100).min(i64::MAX as u128) as i64
}

fn adjusted_position(
    position_ticks: i64,
    last_updated_ticks: i64,
    duration: Option<Duration>,
    is_playing: bool,
    playback_rate: f64,
) -> Duration {
    let mut position = timespan_to_duration(position_ticks);

    // A zero LastUpdatedTime means the session never reported timeline data;
    // extrapolating from it would add the entire FILETIME epoch (~425 years).
    if is_playing && last_updated_ticks > 0 {
        let elapsed_ticks = current_filetime_ticks().saturating_sub(last_updated_ticks);
        if elapsed_ticks > 0 {
            position += timespan_to_duration(elapsed_ticks).mul_f64(playback_rate);
        }
    }

    duration.map_or(position, |duration| position.min(duration))
}

fn current_filetime_ticks() -> i64 {
    const WINDOWS_TICKS_TO_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

    let unix_elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    WINDOWS_TICKS_TO_UNIX_EPOCH + (unix_elapsed.as_nanos() / 100) as i64
}
