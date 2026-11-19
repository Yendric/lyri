use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use windows_reactor::*;

use lyri_core::lyrics::{LyricsStatus, TrackKey};
use lyri_providers::LyricsProviderChain;
use lyri_server::host::{Snapshot, ViewHost};
use lyri_server::protocol::{Lyrics, Playback, ViewState};

mod body;
mod common;
mod header;
mod settings;
mod synced_lyrics;

use body::render_body;
use header::render_header;
use settings::{
    SettingsChange, SettingsState, SourceConfig, WebServerConfig, render_settings_pane,
};

use crate::external_source::{ExternalEvent, PlaybackControls, spawn_external_source};
use crate::now_playing::{NowPlaying, NowPlayingEvent, spawn_now_playing_events};

/// The active source's controls, shared with the web-view server's seek
/// handler (which runs off the UI thread).
type SharedControls = Arc<Mutex<Option<PlaybackControls>>>;

const TICK_INTERVAL: Duration = Duration::from_millis(250);
const SETTINGS_FADE: Duration = Duration::from_millis(180);

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
    WindowWidth(f64),
    ColorScheme(ColorScheme),
    SetTheme(WindowTheme),
    ToggleTheme,
    ToggleSettings,
    CloseSettings,
    HideSettings,
    Settings(SettingsChange),
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

pub struct Lyri {
    playback: Option<NowPlaying>,
    playback_error: Option<String>,
    lyrics: LyricsStatus,
    web_server_error: Option<String>,
    settings: SettingsState,
    theme: WindowTheme,
    system_scheme: ColorScheme,
    window_width: f64,
    settings_open: bool,
    /// Lags `settings_open` on close until the fade-out has finished.
    settings_visible: bool,
    hide_settings: Option<ComponentTimer>,
    tick: Option<ComponentTimer>,

    completion: ComponentCompletion<Message>,
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

impl Component for Lyri {
    type Input = ();
    type Message = Message;

    fn create(_: &(), context: &ComponentContext<Self>) -> Self {
        let settings = SettingsState::default();
        let providers = Arc::new(LyricsProviderChain::lrclib(&settings.lrclib_url()));
        let mut lyri = Self {
            playback: None,
            playback_error: None,
            lyrics: LyricsStatus::default(),
            web_server_error: None,
            settings,
            theme: WindowTheme::System,
            system_scheme: ColorScheme::Light,
            window_width: 520.0,
            settings_open: false,
            settings_visible: false,
            hide_settings: None,
            tick: None,
            completion: context.completion(),
            runtime: background_runtime(),
            providers,
            controls: Arc::new(Mutex::new(None)),
            view_state: watch::channel(Snapshot::default()).0,
            source: None,
            source_generation: 0,
            lyrics_request: None,
            lyrics_task: None,
            lyrics_generation: 0,
            web_server: None,
            web_server_generation: 0,
        };
        lyri.start_source();
        lyri.refresh_lyrics();
        lyri.restart_web_server();
        lyri
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Source(generation, event) => {
                if generation == self.source_generation {
                    self.apply_source_event(event, context);
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
            Message::Tick => {
                self.tick = None;
                self.sync_tick(context);
            }
            Message::WindowWidth(width) => self.window_width = width,
            Message::ColorScheme(scheme) => self.system_scheme = scheme,
            Message::SetTheme(theme) => self.theme = theme,
            Message::ToggleTheme => {
                self.theme = if self.is_dark() {
                    WindowTheme::Light
                } else {
                    WindowTheme::Dark
                };
            }
            Message::ToggleSettings if self.settings_open => self.close_settings(context),
            Message::ToggleSettings => {
                self.hide_settings = None;
                self.settings_visible = true;
                self.settings_open = true;
            }
            Message::CloseSettings => self.close_settings(context),
            Message::HideSettings => {
                self.hide_settings = None;
                self.settings_visible = false;
            }
            Message::Settings(change) => self.apply_settings_change(change, context),
        }
    }

    fn view(&self, _: &(), context: &mut ViewContext<Self>) -> View {
        context
            .on_window_size(context.callback(|size: WindowSize| Message::WindowWidth(size.width)));
        context.on_color_scheme(context.callback(Message::ColorScheme));
        context.window_title("Lyri");
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .theme(self.theme),
        );

        let width = self.window_width;
        let is_dark = self.is_dark();

        let settings_button = title_bar_button("\u{E713}")
            .on_click(context.message(Message::ToggleSettings))
            .tooltip("Settings");
        let theme_toggle = title_bar_button(if is_dark { "\u{E706}" } else { "\u{E708}" })
            .on_click(context.message(Message::ToggleTheme))
            .tooltip(if is_dark { "Light mode" } else { "Dark mode" });

        let title_bar = TitleBar::new().title("Lyri").right_header(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(4.0)
                .margin(Thickness::new(0.0, 0.0, 8.0, 0.0))
                .horizontal_alignment(HorizontalAlignment::Right)
                .children((settings_button, theme_toggle)),
        );

        let active_lyrics = self
            .lyrics
            .active(self.playback.as_ref().map(|playback| &playback.key));
        let status = self
            .playback_error
            .clone()
            .unwrap_or_else(|| active_lyrics.status_message());
        let content_padding = responsive_content_padding(width);
        let body = render_body(
            self.playback.as_ref(),
            active_lyrics,
            &status,
            self.source.clone(),
            width - content_padding * 2.0,
        );
        // Scrolling lyrics span both rows and pass under the header, which
        // is declared last so it draws on top.
        let body_rows = if body.behind_header { 2 } else { 1 };
        let body = Border::new()
            .grid_row(2 - body_rows)
            .grid_row_span(body_rows)
            .content(body.view);
        let header = render_header(self.playback.as_ref(), &status).grid_row(0);

        let content = Grid::new()
            .rows([GridLength::Auto, GridLength::STAR])
            .row_spacing(28.0)
            .margin(Thickness::new(content_padding, 12.0, content_padding, 0.0))
            .children((body, header));

        let pane_width = if width < 460.0 {
            (width - 48.0).max(300.0)
        } else {
            340.0
        };
        // The pane stays mounted so its opacity can fade both ways; it is
        // collapsed to zero width once a close has finished fading.
        let settings_pane = render_settings_pane(
            &self.settings,
            is_dark,
            self.web_server_error.as_deref(),
            context,
        )
        .width(if self.settings_visible {
            pane_width
        } else {
            0.0
        })
        .margin(if self.settings_visible { 12.0 } else { 0.0 })
        .opacity(if self.settings_open { 1.0 } else { 0.0 })
        .opacity_transition(SETTINGS_FADE)
        .horizontal_alignment(HorizontalAlignment::Right);

        Grid::new()
            .rows([GridLength::Auto, GridLength::STAR])
            .children((
                title_bar.grid_row(0),
                Grid::new().grid_row(1).children((content, settings_pane)),
            ))
            .into()
    }
}

impl Drop for Lyri {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.shutdown();
        }
        if let Some(task) = self.lyrics_task.take() {
            task.abort();
        }
        if let Some(server) = self.web_server.take() {
            server.shutdown();
        }
    }
}

impl Lyri {
    fn is_dark(&self) -> bool {
        match self.theme {
            WindowTheme::Dark => true,
            WindowTheme::Light => false,
            _ => matches!(self.system_scheme, ColorScheme::Dark),
        }
    }

    fn close_settings(&mut self, context: &ComponentContext<Self>) {
        self.settings_open = false;
        self.hide_settings = Some(context.set_timeout(SETTINGS_FADE, Message::HideSettings));
    }

    /// Re-renders every tick while playing so the position stays current.
    fn sync_tick(&mut self, context: &ComponentContext<Self>) {
        let playing = self
            .playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing);
        if !playing {
            self.tick = None;
        } else if self.tick.is_none() {
            self.tick = Some(context.set_timeout(TICK_INTERVAL, Message::Tick));
        }
    }

    fn apply_source_event(&mut self, event: SourceEvent, context: &ComponentContext<Self>) {
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
        self.sync_tick(context);
        self.publish();
    }

    fn apply_settings_change(&mut self, change: SettingsChange, context: &ComponentContext<Self>) {
        let previous = self.settings.clone();
        self.settings.apply(change);

        if self.settings.source() != previous.source() {
            self.start_source();
            self.sync_tick(context);
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

    /// Follows the configured playback source: the local system's media
    /// sessions, or another Lyri instance on the network.
    fn start_source(&mut self) {
        if let Some(previous) = self.source.take() {
            previous.shutdown();
        }
        self.playback = None;
        self.playback_error = None;
        self.lyrics = LyricsStatus::Idle;

        self.source_generation += 1;
        let generation = self.source_generation;
        let completion = self.completion.clone();
        let emit = move |event| {
            let _ = completion
                .clone()
                .complete(Message::Source(generation, event));
        };

        let next = match self.settings.source() {
            SourceConfig::System => {
                PlaybackControls::System(spawn_now_playing_events(move |event| {
                    emit(SourceEvent::System(event))
                }))
            }
            SourceConfig::External { host, port } => {
                PlaybackControls::External(spawn_external_source(host, port, move |event| {
                    emit(SourceEvent::External(event))
                }))
            }
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
        let completion = self.completion.clone();
        self.lyrics_task = Some(self.runtime.spawn(async move {
            let lyrics = providers.fetch(key, &query).await;
            let _ = completion.complete(Message::Lyrics(generation, lyrics));
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
            Ok(_) => self.web_server = Some(self.spawn_web_server(config)),
            Err(error) => self.web_server_error = Some(error),
        }
    }

    fn spawn_web_server(&self, config: WebServerConfig) -> WebServer {
        let bind = config.socket_addr().expect("validated bind address");
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
        let completion = self.completion.clone();
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
                let _ =
                    completion.complete(Message::WebServerFailed(generation, format!("{error:#}")));
            }
        });

        WebServer { stop, task }
    }
}

fn title_bar_button(glyph: &str) -> Button {
    Button::new()
        .content(FontIcon::new().glyph(glyph).font_size(14.0))
        .width(40.0)
        .height(36.0)
}

/// Shared runtime for lyrics lookups and the web-view server.
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

fn responsive_content_padding(width: f64) -> f64 {
    if width >= 1400.0 { 48.0 } else { 36.0 }
}
