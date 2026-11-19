use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;
use gtk::{Align, Orientation, gdk};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use lyri_core::lyrics::{
    ActiveLyrics, LyricsResult, LyricsStatus, TrackKey, artist_album_label, format_duration,
};
use lyri_providers::LyricsProviderChain;
use lyri_server::host::{Snapshot, ViewHost};
use lyri_server::protocol::{Lyrics, Playback, ViewState};

mod settings;
mod synced_lyrics;

use settings::{SettingsChange, SettingsPane, SettingsState, SourceConfig, WebServerConfig};
use synced_lyrics::SyncedLyrics;

use crate::external_source::{ExternalEvent, PlaybackControls, spawn_external_source};
use crate::now_playing::{NowPlaying, NowPlayingEvent, spawn_now_playing_events};

const APP_ID: &str = "dev.lyri.Lyri";
const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// The active source's controls, shared with the web-view server's seek
/// handler (which runs off the UI thread).
type SharedControls = Arc<Mutex<Option<PlaybackControls>>>;

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        // Relaunching activates the running instance; don't build a second UI.
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        start(app);
    });
    app.run()
}

#[derive(Clone, Debug)]
pub enum SourceEvent {
    System(NowPlayingEvent),
    External(ExternalEvent),
}

/// Background results carry the generation they were started for, so a
/// replaced source or task can't clobber the current state.
#[derive(Clone, Debug)]
pub enum Message {
    Source(u64, SourceEvent),
    Lyrics(u64, LyricsStatus),
    WebServerFailed(u64, String),
    Tick,
    Seek(Duration),
    ToggleSettings,
    CloseSettings,
    Settings(SettingsChange),
}

/// What the body currently shows; it is only rebuilt when this changes.
#[derive(Clone, Debug, PartialEq)]
enum BodyKey {
    Empty,
    Loading(String),
    Missing,
    Error(String),
    Instrumental,
    Plain(TrackKey),
    Synced { track: TrackKey, seekable: bool },
}

struct WebServer {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl WebServer {
    fn shutdown(self) {
        // Stops accepting and disconnects every connected viewer.
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

struct Ui {
    title: gtk::Label,
    subtitle: gtk::Label,
    time: gtk::Label,
    body: gtk::Box,
    settings: gtk::Revealer,
    settings_pane: SettingsPane,
}

struct Lyri {
    ui: Ui,
    tx: UnboundedSender<Message>,
    playback: Option<NowPlaying>,
    playback_error: Option<String>,
    lyrics: LyricsStatus,
    web_server_error: Option<String>,
    settings: SettingsState,
    body_key: Option<BodyKey>,
    synced: Option<SyncedLyrics>,
    tick: Option<glib::SourceId>,

    runtime: Arc<Runtime>,
    providers: Arc<LyricsProviderChain>,
    controls: SharedControls,
    view_state: watch::Sender<Snapshot>,
    source: Option<PlaybackControls>,
    source_generation: u64,
    lyrics_request: Option<(Option<TrackKey>, bool, String)>,
    lyrics_task: Option<JoinHandle<()>>,
    lyrics_generation: u64,
    web_server: Option<WebServer>,
    web_server_generation: u64,
}

fn start(app: &gtk::Application) {
    install_css();

    let (tx, mut rx) = unbounded_channel();
    let settings = SettingsState::default();
    let ui = build_window(app, &settings, &tx);
    let lyri = Rc::new(RefCell::new(Lyri {
        ui,
        tx,
        playback: None,
        playback_error: None,
        lyrics: LyricsStatus::Idle,
        web_server_error: None,
        providers: Arc::new(LyricsProviderChain::lrclib(&settings.lrclib_url())),
        settings,
        body_key: None,
        synced: None,
        tick: None,
        runtime: background_runtime(),
        controls: Arc::new(Mutex::new(None)),
        view_state: watch::channel(Snapshot::default()).0,
        source: None,
        source_generation: 0,
        lyrics_request: None,
        lyrics_task: None,
        lyrics_generation: 0,
        web_server: None,
        web_server_generation: 0,
    }));

    {
        let mut lyri = lyri.borrow_mut();
        lyri.start_source();
        lyri.refresh_lyrics();
        lyri.restart_web_server();
        lyri.render();
    }

    // Widget handlers and background tasks only send messages, so updates
    // never re-enter while GTK is emitting signals.
    glib::spawn_future_local(async move {
        while let Some(message) = rx.recv().await {
            lyri.borrow_mut().update(message);
        }
    });
}

impl Lyri {
    fn update(&mut self, message: Message) {
        match message {
            Message::Source(generation, event) => {
                if generation == self.source_generation {
                    self.apply_source_event(event);
                }
            }
            Message::Lyrics(generation, lyrics) => {
                if generation == self.lyrics_generation {
                    self.lyrics = lyrics;
                    self.publish();
                }
            }
            Message::WebServerFailed(generation, error) => {
                if generation == self.web_server_generation {
                    self.web_server_error = Some(error);
                }
            }
            Message::Tick => {}
            Message::Seek(position) => self.seek(position),
            Message::ToggleSettings => {
                let open = self.ui.settings.reveals_child();
                self.set_settings_open(!open);
            }
            Message::CloseSettings => self.set_settings_open(false),
            Message::Settings(change) => self.apply_settings_change(change),
        }
        self.render();
    }

    fn apply_source_event(&mut self, event: SourceEvent) {
        match event {
            SourceEvent::System(NowPlayingEvent::Changed(playback)) => {
                self.playback_error = None;
                self.playback = playback;
            }
            SourceEvent::System(NowPlayingEvent::Error(error)) => {
                self.playback_error = Some(error);
            }
            SourceEvent::External(ExternalEvent::State(state)) => {
                self.playback_error = None;
                self.playback = state.playback;
                self.lyrics = state.lyrics;
            }
            SourceEvent::External(ExternalEvent::ConnectionChanged(None)) => {
                self.playback_error = None;
            }
            SourceEvent::External(ExternalEvent::ConnectionChanged(Some(error))) => {
                self.playback = None;
                self.playback_error = Some(format!("External Lyri: {error}"));
            }
        }
        self.refresh_lyrics();
        self.sync_tick();
        self.publish();
    }

    fn apply_settings_change(&mut self, change: SettingsChange) {
        let previous = self.settings.clone();
        self.settings.apply(change);

        if self.settings.source() != previous.source() {
            self.start_source();
            self.sync_tick();
        }
        if self.settings.lrclib_url() != previous.lrclib_url() {
            self.providers = Arc::new(LyricsProviderChain::lrclib(&self.settings.lrclib_url()));
        }
        self.refresh_lyrics();
        if self.settings.web_server() != previous.web_server() {
            self.restart_web_server();
        }
        self.publish();
    }

    /// A hidden crossfade revealer keeps its size, so it must stop taking
    /// input or it swallows clicks and scrolls meant for the lyrics.
    fn set_settings_open(&self, open: bool) {
        self.ui.settings.set_reveal_child(open);
        self.ui.settings.set_can_target(open);
    }

    fn seek(&self, position: Duration) {
        if let (Some(source), Some(playback)) = (&self.source, &self.playback) {
            source.seek_to(playback.key.clone(), position);
        }
        if let Some(synced) = &self.synced {
            synced.follow();
        }
    }

    /// Re-renders on a timer while playing so the position stays current.
    fn sync_tick(&mut self) {
        let playing = self
            .playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing);
        if !playing {
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
        } else if self.tick.is_none() {
            let tx = self.tx.clone();
            self.tick = Some(glib::timeout_add_local(TICK_INTERVAL, move || {
                let _ = tx.send(Message::Tick);
                ControlFlow::Continue
            }));
        }
    }

    /// Follows the configured playback source: the local system's media
    /// players, or another Lyri instance on the network.
    fn start_source(&mut self) {
        if let Some(previous) = self.source.take() {
            previous.shutdown();
        }
        self.playback = None;
        self.playback_error = None;
        self.lyrics = LyricsStatus::Idle;

        self.source_generation += 1;
        let generation = self.source_generation;
        let tx = self.tx.clone();
        let emit = move |event| {
            let _ = tx.send(Message::Source(generation, event));
        };

        let runtime = self.runtime.handle();
        let next = match self.settings.source() {
            SourceConfig::System => {
                PlaybackControls::System(spawn_now_playing_events(runtime, move |event| {
                    emit(SourceEvent::System(event))
                }))
            }
            SourceConfig::External { host, port } => PlaybackControls::External(
                spawn_external_source(runtime, host, port, move |event| {
                    emit(SourceEvent::External(event))
                }),
            ),
        };

        *self.controls.lock().expect("playback controls lock") = Some(next.clone());
        self.source = Some(next);
    }

    /// Looks up lyrics locally when the track or lookup settings change; a
    /// mirrored host streams its own instead.
    fn refresh_lyrics(&mut self) {
        let fetch_locally = self.settings.source() == SourceConfig::System;
        let request = (
            self.playback.as_ref().map(|playback| playback.key.clone()),
            fetch_locally,
            self.settings.lrclib_url(),
        );
        if self.lyrics_request.as_ref() == Some(&request) {
            return;
        }
        self.lyrics_request = Some(request);
        self.lyrics_generation += 1;
        if let Some(task) = self.lyrics_task.take() {
            task.abort();
        }

        if !fetch_locally {
            return;
        }
        let Some(playback) = &self.playback else {
            self.lyrics = LyricsStatus::Idle;
            return;
        };

        let key = playback.key.clone();
        let query = playback.lyrics_query();
        self.lyrics = LyricsStatus::Loading(key.clone());

        let generation = self.lyrics_generation;
        let providers = self.providers.clone();
        let tx = self.tx.clone();
        self.lyrics_task = Some(self.runtime.spawn(async move {
            let lyrics = providers.fetch(key, &query).await;
            let _ = tx.send(Message::Lyrics(generation, lyrics));
        }));
    }

    /// Publishes the current view for the web-view server to stream.
    fn publish(&self) {
        let state = view_state(self.playback.as_ref(), &self.lyrics);
        // send_replace keeps the value even while no server is subscribed.
        self.view_state.send_replace(Snapshot::new(state));
    }

    /// Hosts the web view while enabled in settings.
    fn restart_web_server(&mut self) {
        if let Some(server) = self.web_server.take() {
            server.shutdown();
        }
        self.web_server_error = None;
        self.web_server_generation += 1;

        let Some(config) = self.settings.web_server() else {
            return;
        };
        match config.socket_addr() {
            Ok(bind) => self.web_server = Some(self.spawn_web_server(bind, config)),
            Err(error) => self.web_server_error = Some(error),
        }
    }

    fn spawn_web_server(&self, bind: std::net::SocketAddr, config: WebServerConfig) -> WebServer {
        let state = self.view_state.subscribe();

        let seek_state = state.clone();
        let seek_controls = self.controls.clone();
        let on_seek = Arc::new(move |position: Duration| {
            let snapshot = seek_state.borrow();
            let controls = seek_controls.lock().expect("playback controls lock");
            if let (Some(controls), Some(playback)) =
                (controls.as_ref(), snapshot.state().playback.as_ref())
            {
                controls.seek_to(playback.track_key(), position);
            }
        });

        let generation = self.web_server_generation;
        let tx = self.tx.clone();
        let allow_seek = config.allow_seek;
        let (stop, stopped) = watch::channel(false);
        let task = self.runtime.spawn(async move {
            let served = async {
                // A restart may race the previous server releasing the port;
                // retry the bind briefly before reporting.
                let mut attempts = 0;
                let host = loop {
                    match ViewHost::bind(
                        bind,
                        allow_seek,
                        state.clone(),
                        on_seek.clone(),
                        stopped.clone(),
                    )
                    .await
                    {
                        Ok(host) => break host,
                        Err(_) if attempts < 8 => {
                            attempts += 1;
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        Err(error) => return Err(error),
                    }
                };

                host.serve().await
            };
            if let Err(error) = served.await {
                let _ = tx.send(Message::WebServerFailed(generation, format!("{error:#}")));
            }
        });

        WebServer { stop, task }
    }

    fn render(&mut self) {
        let active = self
            .lyrics
            .active(self.playback.as_ref().map(|playback| &playback.key));
        let status = self
            .playback_error
            .clone()
            .unwrap_or_else(|| active.status_message());

        match &self.playback {
            Some(playback) => {
                let title = playback.title.trim();
                self.ui
                    .title
                    .set_text(if title.is_empty() { "Lyri" } else { title });
                self.ui
                    .subtitle
                    .set_text(&artist_album_label(&playback.artist, &playback.album));
                self.ui.time.set_text(&playback_time(playback));
            }
            None => {
                self.ui.title.set_text("Lyri");
                self.ui.subtitle.set_text(&status);
                self.ui.time.set_text("");
            }
        }
        self.ui
            .settings_pane
            .sync(&self.settings, self.web_server_error.as_deref());

        let key = body_key(self.playback.as_ref(), active, &status);
        if self.body_key.as_ref() == Some(&key) {
            if let (
                Some(synced),
                Some(playback),
                ActiveLyrics::Ready(LyricsResult::Synced { lines, .. }),
            ) = (&self.synced, &self.playback, active)
            {
                synced.update(lines, playback.current_position());
            }
            return;
        }

        self.synced = None;
        while let Some(child) = self.ui.body.first_child() {
            self.ui.body.remove(&child);
        }
        let body: gtk::Widget = match (&self.playback, active) {
            (None, _) => empty_state("Play a song and Lyri will follow along.").upcast(),
            (Some(_), ActiveLyrics::Idle | ActiveLyrics::Loading) => {
                let spinner = gtk::Spinner::new();
                spinner.start();
                centered([spinner.upcast(), label(&status, &[]).upcast()]).upcast()
            }
            (Some(_), ActiveLyrics::Missing) => {
                empty_state("No lyrics found for this track.").upcast()
            }
            (Some(_), ActiveLyrics::Error(error)) => centered([
                label("Could not load lyrics", &["error-title"]).upcast(),
                label(error, &["error-detail"]).upcast(),
            ])
            .upcast(),
            (Some(_), ActiveLyrics::Ready(LyricsResult::Instrumental { .. })) => {
                empty_state("Instrumental track").upcast()
            }
            (Some(_), ActiveLyrics::Ready(LyricsResult::Plain { text, .. })) => {
                plain_lyrics(text).upcast()
            }
            (Some(_), ActiveLyrics::Ready(LyricsResult::Synced { lines, .. }))
                if lines.is_empty() =>
            {
                empty_state("No timed lyric lines found.").upcast()
            }
            (Some(playback), ActiveLyrics::Ready(LyricsResult::Synced { lines, .. })) => {
                let seekable = playback.can_seek.then_some(&self.tx);
                let synced = SyncedLyrics::new(lines, playback.current_position(), seekable);
                let widget = synced.widget().clone().upcast();
                self.synced = Some(synced);
                widget
            }
        };
        self.ui.body.append(&body);
        self.body_key = Some(key);
    }
}

fn body_key(playback: Option<&NowPlaying>, active: ActiveLyrics<'_>, status: &str) -> BodyKey {
    let Some(playback) = playback else {
        return BodyKey::Empty;
    };
    match active {
        ActiveLyrics::Idle | ActiveLyrics::Loading => BodyKey::Loading(status.to_string()),
        ActiveLyrics::Missing => BodyKey::Missing,
        ActiveLyrics::Error(error) => BodyKey::Error(error.to_string()),
        ActiveLyrics::Ready(LyricsResult::Instrumental { .. }) => BodyKey::Instrumental,
        ActiveLyrics::Ready(LyricsResult::Plain { .. }) => BodyKey::Plain(playback.key.clone()),
        ActiveLyrics::Ready(LyricsResult::Synced { .. }) => BodyKey::Synced {
            track: playback.key.clone(),
            seekable: playback.can_seek,
        },
    }
}

fn build_window(
    app: &gtk::Application,
    settings: &SettingsState,
    tx: &UnboundedSender<Message>,
) -> Ui {
    let title = label("Lyri", &["track-title"]);
    let subtitle = label("", &["muted"]);
    let time = label("", &["dim"]);
    let header = gtk::Box::new(Orientation::Vertical, 4);
    header.add_css_class("track-header");
    header.set_valign(Align::Start);
    header.append(&title);
    header.append(&subtitle);
    header.append(&time);

    let body = gtk::Box::new(Orientation::Vertical, 0);
    body.set_hexpand(true);
    body.set_vexpand(true);

    // Synced lyrics scroll under the track header, which draws on top.
    let content = gtk::Overlay::new();
    content.add_css_class("content");
    content.set_child(Some(&body));
    content.add_overlay(&header);

    let settings_pane = SettingsPane::new(settings, tx);
    let settings_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&settings_pane.root)
        .propagate_natural_height(true)
        .build();
    let settings = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(180)
        .halign(Align::End)
        .valign(Align::Fill)
        .can_target(false)
        .child(&settings_scroll)
        .build();

    let root = gtk::Overlay::new();
    root.set_child(Some(&content));
    root.add_overlay(&settings);

    let settings_button = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some("Settings"));
    let tx_settings = tx.clone();
    settings_button.connect_clicked(move |_| {
        let _ = tx_settings.send(Message::ToggleSettings);
    });

    let theme_button = gtk::Button::new();
    if let Some(gtk_settings) = gtk::Settings::default() {
        let sync_icon = {
            let theme_button = theme_button.clone();
            move |gtk_settings: &gtk::Settings| {
                let dark = gtk_settings.is_gtk_application_prefer_dark_theme();
                theme_button.set_icon_name(if dark {
                    "weather-clear-symbolic"
                } else {
                    "weather-clear-night-symbolic"
                });
                theme_button.set_tooltip_text(Some(if dark { "Light mode" } else { "Dark mode" }));
            }
        };
        sync_icon(&gtk_settings);
        gtk_settings.connect_gtk_application_prefer_dark_theme_notify(sync_icon);
    }
    theme_button.connect_clicked(|_| {
        if let Some(gtk_settings) = gtk::Settings::default() {
            let dark = gtk_settings.is_gtk_application_prefer_dark_theme();
            gtk_settings.set_gtk_application_prefer_dark_theme(!dark);
        }
    });

    let header_bar = gtk::HeaderBar::new();
    header_bar.pack_end(&theme_button);
    header_bar.pack_end(&settings_button);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Lyri")
        .icon_name(APP_ID)
        .default_width(520)
        .default_height(750)
        .titlebar(&header_bar)
        .child(&root)
        .build();
    window.present();

    Ui {
        title,
        subtitle,
        time,
        body,
        settings,
        settings_pane,
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

fn empty_state(text: &str) -> gtk::Box {
    centered([label(text, &["empty"]).upcast()])
}

fn centered(children: impl IntoIterator<Item = gtk::Widget>) -> gtk::Box {
    let column = gtk::Box::new(Orientation::Vertical, 12);
    column.set_valign(Align::Center);
    column.set_vexpand(true);
    for child in children {
        column.append(&child);
    }
    column
}

fn plain_lyrics(text: &str) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&label(text, &["plain-lyrics"]))
        .vexpand(true)
        .build()
}

fn playback_time(playback: &NowPlaying) -> String {
    let position = format_duration(playback.current_position());
    let paused = if playback.is_playing { "" } else { " paused" };
    match playback.duration {
        Some(duration) => format!("{position} / {}{paused}", format_duration(duration)),
        None => format!("{position}{paused}"),
    }
}

/// Shared runtime for D-Bus, lyrics lookups and the web-view server.
fn background_runtime() -> Arc<Runtime> {
    Arc::new(
        Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("valid background runtime"),
    )
}

/// Protocol snapshot of what this instance is showing right now. Lyrics are
/// narrowed to the current track so stale statuses are never published.
fn view_state(playback: Option<&NowPlaying>, lyrics: &LyricsStatus) -> ViewState {
    ViewState {
        playback: playback.map(|playback| Playback {
            title: playback.title.clone(),
            artist: playback.artist.clone(),
            album: playback.album.clone(),
            duration_ms: playback
                .duration
                .map(|duration| duration.as_millis() as u64),
            position_ms: playback.current_position().as_millis() as u64,
            is_playing: playback.is_playing,
            can_seek: playback.can_seek,
        }),
        lyrics: Lyrics::from_active(lyrics.active(playback.map(|playback| &playback.key))),
    }
}

fn install_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../style.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
