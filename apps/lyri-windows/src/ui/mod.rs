use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;
use windows_reactor::*;

use lyri_core::lyrics::LyricsStatus;
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
use settings::{SettingsState, SourceConfig, render_settings_pane};

use crate::external_source::{ExternalEvent, PlaybackControls, spawn_external_source};
use crate::now_playing::{NowPlaying, NowPlayingEvent, spawn_now_playing_events};

/// The active source's controls, shared with the web-view server's seek
/// handler (which runs off the UI thread).
type SharedControls = Arc<Mutex<Option<PlaybackControls>>>;

pub fn app(cx: &mut RenderCx) -> Element {
    let (playback, set_playback) = cx.use_async_state(None::<NowPlaying>);
    let (playback_error, set_playback_error) = cx.use_async_state(None::<String>);
    let (lyrics, set_lyrics) = cx.use_async_state(LyricsStatus::default());
    let (web_server_error, set_web_server_error) = cx.use_async_state(None::<String>);
    let (settings, set_settings) = cx.use_state(SettingsState::default());
    let (_, tick) = cx.use_reducer(0_u64);
    let runtime = cx.use_memo((), background_runtime);
    let lrclib_url = settings.lrclib_url();
    let lyrics_providers = cx.use_memo(lrclib_url.clone(), || {
        Arc::new(LyricsProviderChain::lrclib(&lrclib_url))
    });
    let controls: SharedControls = cx.use_memo((), || Arc::new(Mutex::new(None)));
    let (view_state_tx, view_state_rx) = cx.use_memo((), || {
        let (tx, rx) = watch::channel(Snapshot::default());
        (Arc::new(tx), rx)
    });

    // Follow the configured playback source: the local system's media
    // sessions, or another Lyri instance on the network.
    let source = settings.source();
    let fetch_lyrics_locally = source == SourceConfig::System;
    let source_controls = controls.clone();
    let source_playback = set_playback.clone();
    let source_error = set_playback_error.clone();
    let source_lyrics = set_lyrics.clone();
    cx.use_effect_with_cleanup(source.clone(), move || {
        source_playback.call(None);
        source_error.call(None);
        source_lyrics.call(LyricsStatus::Idle);

        // A dying source may emit one last event after shutdown; the gate
        // keeps it from clobbering the next source's state.
        let active = Arc::new(AtomicBool::new(true));

        let next = match source {
            SourceConfig::System => {
                let set_playback = source_playback.clone();
                let set_error = source_error.clone();
                let gate = active.clone();
                PlaybackControls::System(spawn_now_playing_events(move |event| {
                    if !gate.load(Ordering::Acquire) {
                        return;
                    }
                    match event {
                        NowPlayingEvent::Changed(playback) => {
                            set_error.call(None);
                            set_playback.call(playback);
                        }
                        NowPlayingEvent::Error(error) => set_error.call(Some(error)),
                    }
                }))
            }
            SourceConfig::External { host, port } => {
                let set_playback = source_playback.clone();
                let set_error = source_error.clone();
                let set_lyrics = source_lyrics.clone();
                let gate = active.clone();
                PlaybackControls::External(spawn_external_source(host, port, move |event| {
                    if !gate.load(Ordering::Acquire) {
                        return;
                    }
                    match event {
                        ExternalEvent::State(state) => {
                            set_error.call(None);
                            set_playback.call(state.playback);
                            set_lyrics.call(state.lyrics);
                        }
                        ExternalEvent::ConnectionChanged(None) => set_error.call(None),
                        ExternalEvent::ConnectionChanged(Some(error)) => {
                            set_playback.call(None);
                            set_error.call(Some(format!("External Lyri: {error}")));
                        }
                    }
                }))
            }
        };

        *source_controls.lock().expect("playback controls lock") = Some(next.clone());
        Some(move || {
            active.store(false, Ordering::Release);
            next.shutdown();
        })
    });

    // Look up lyrics locally; a mirrored host streams its own instead.
    let lyrics_playback = playback.clone();
    let lyrics_runtime = runtime.clone();
    cx.use_effect_with_cleanup(
        (
            playback.as_ref().map(|value| value.key.clone()),
            fetch_lyrics_locally,
            settings.lrclib_url(),
        ),
        move || {
            if !fetch_lyrics_locally {
                return None;
            }

            let Some(playback) = lyrics_playback else {
                set_lyrics.call(LyricsStatus::Idle);
                return None;
            };

            let key = playback.key.clone();
            let query = playback.lyrics_query();
            set_lyrics.call(LyricsStatus::Loading(key.clone()));

            let setter = set_lyrics.clone();
            let task = lyrics_runtime.spawn(async move {
                setter.call(lyrics_providers.fetch(key, &query).await);
            });

            Some(move || task.abort())
        },
    );

    // Publish the current view for the web-view server to stream.
    let published = view_state(playback.as_ref(), &lyrics);
    let publish = view_state_tx.clone();
    cx.use_effect((playback.clone(), lyrics.clone()), move || {
        let _ = publish.send(Snapshot::new(published));
    });

    // Host the web view while enabled in settings.
    let web_server = settings.web_server();
    let server_runtime = runtime.clone();
    let server_controls = controls.clone();
    let server_state = view_state_rx.clone();
    let server_error = set_web_server_error.clone();
    cx.use_effect_with_cleanup(web_server.clone(), move || {
        server_error.call(None);
        let config = web_server?;
        let bind = match config.socket_addr() {
            Ok(bind) => bind,
            Err(error) => {
                server_error.call(Some(error));
                return None;
            }
        };

        let seek_state = server_state.clone();
        let seek_controls = server_controls.clone();
        let on_seek = Arc::new(move |position: Duration| {
            let snapshot = seek_state.borrow();
            let controls = seek_controls.lock().expect("playback controls lock");
            if let (Some(controls), Some(playback)) =
                (controls.as_ref(), snapshot.state().playback.as_ref())
            {
                controls.seek_to(playback.track_key(), position);
            }
        });

        let report = server_error.clone();
        let state = server_state.clone();
        let allow_seek = config.allow_seek;
        let (stop, stopped) = watch::channel(false);
        let task = server_runtime.spawn(async move {
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
                report.call(Some(format!("{error:#}")));
            }
        });

        Some(move || {
            // Stops accepting and disconnects every connected viewer.
            let _ = stop.send(true);
            task.abort();
        })
    });

    let is_playing = playback
        .as_ref()
        .map(|value| value.is_playing)
        .unwrap_or(false);
    cx.use_effect_with_cleanup(is_playing, move || {
        if !is_playing {
            return None;
        }

        let timer = DispatcherTimer::new(Duration::from_millis(250), move || {
            tick.call(|value| value.wrapping_add(1));
        })
        .ok()?;

        Some(move || drop(timer))
    });

    let playback_controls = controls.lock().expect("playback controls lock").clone();
    render_app(
        playback.as_ref(),
        playback_error.as_deref(),
        &lyrics,
        playback_controls,
        SettingsContext {
            settings,
            set_settings,
            web_server_error: web_server_error.as_deref(),
        },
        cx,
    )
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
            duration_ms: playback.duration.map(|duration| duration.as_millis() as u64),
            position_ms: playback.current_position().as_millis() as u64,
            is_playing: playback.is_playing,
            can_seek: playback.can_seek,
        }),
        lyrics: Lyrics::from_active(lyrics.active(playback.map(|playback| &playback.key))),
    }
}

struct SettingsContext<'a> {
    settings: SettingsState,
    set_settings: SetState<SettingsState>,
    web_server_error: Option<&'a str>,
}

fn render_app(
    playback: Option<&NowPlaying>,
    playback_error: Option<&str>,
    lyrics: &LyricsStatus,
    playback_controls: Option<PlaybackControls>,
    settings_context: SettingsContext<'_>,
    cx: &mut RenderCx,
) -> Element {
    let inner_size = cx.use_inner_size();
    let is_dark = matches!(cx.use_color_scheme(), ColorScheme::Dark);
    let (settings_open, set_settings_open) = cx.use_state(false);
    let (settings_visible, set_settings_visible) = cx.use_state(false);

    let toggle_settings = set_settings_open.clone();
    let show_settings = set_settings_visible.clone();
    let settings_button = button("\u{E713}")
        .on_click(move || {
            if settings_open {
                toggle_settings.call(false);
            } else {
                show_settings.call(true);
                toggle_settings.call(true);
            }
        })
        .font_family("Segoe MDL2 Assets")
        .font_size(14.0)
        .width(40.0)
        .height(36.0)
        .padding(0.0)
        .tooltip("Settings");

    let theme_toggle = {
        let glyph = if is_dark { "\u{E706}" } else { "\u{E708}" };
        button(glyph)
            .on_click(move || {
                set_requested_theme(if is_dark {
                    RequestedTheme::Light
                } else {
                    RequestedTheme::Dark
                });
            })
            .font_family("Segoe MDL2 Assets")
            .font_size(14.0)
            .width(40.0)
            .height(36.0)
            .padding(0.0)
            .tooltip(if is_dark { "Light mode" } else { "Dark mode" })
    };

    let title_bar = TitleBar::new("Lyri").footer(
        hstack((settings_button, theme_toggle))
            .spacing(4.0)
            .margin(Thickness {
                left: 0.0,
                top: 0.0,
                right: 8.0,
                bottom: 0.0,
            })
            .horizontal_alignment(HorizontalAlignment::Right),
    );

    let hide_settings = set_settings_visible.clone();
    cx.use_effect_with_cleanup((settings_open, settings_visible), move || {
        if settings_open || !settings_visible {
            return None;
        }

        let timer = DispatcherTimer::new_one_shot(Duration::from_millis(180), move || {
            hide_settings.call(false);
        })
        .ok()?;

        Some(move || drop(timer))
    });

    let active_lyrics = lyrics.active(playback.map(|playback| &playback.key));
    let status = playback_error
        .map(str::to_string)
        .unwrap_or_else(|| active_lyrics.status_message());
    let header = render_header(playback, &status);
    let body = render_body(playback, active_lyrics, &status, playback_controls, cx);
    let content_padding = responsive_content_padding(inner_size.width);

    let content = border(
        grid((header.grid_row(0), body.grid_row(1)))
            .rows(vec![GridLength::Auto, GridLength::Star(1.0)])
            .row_spacing(28.0)
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .vertical_alignment(VerticalAlignment::Stretch),
    )
    .padding(Thickness {
        left: content_padding,
        top: 12.0,
        right: content_padding,
        bottom: 0.0,
    })
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .vertical_alignment(VerticalAlignment::Stretch);

    let pane_width = if inner_size.width < 460.0 {
        (inner_size.width - 48.0).max(300.0)
    } else {
        340.0
    };

    let close_settings = set_settings_open.clone();
    let app_content: Element = grid((
        content.grid_row(0).with_key("lyrics-content"),
        render_settings_pane(
            settings_context.settings,
            settings_context.set_settings,
            is_dark,
            settings_context.web_server_error,
            move || close_settings.call(false),
        )
        .grid_row(0)
        .width(if settings_visible { pane_width } else { 0.0 })
        .margin(if settings_visible {
            Thickness::uniform(12.0)
        } else {
            Thickness::uniform(0.0)
        })
        .opacity(if settings_open { 1.0 } else { 0.0 })
        .with_opacity_transition(Duration::from_millis(180))
        .with_key("settings-pane")
        .horizontal_alignment(HorizontalAlignment::Right)
        .vertical_alignment(VerticalAlignment::Stretch),
    ))
    .rows([GridLength::Star(1.0)])
    .into();

    grid((title_bar.grid_row(0), app_content.grid_row(1)))
        .rows([GridLength::Auto, GridLength::Star(1.0)])
        .into()
}

fn responsive_content_padding(width: f64) -> f64 {
    if width >= 1400.0 { 48.0 } else { 36.0 }
}
