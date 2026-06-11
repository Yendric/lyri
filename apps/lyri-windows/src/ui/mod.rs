use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use windows_reactor::*;

use lyri_core::lyrics::LyricsResult;
use lyri_providers::LyricsProviderChain;

mod body;
mod common;
mod header;
mod settings;
mod synced_lyrics;

use body::render_body;
use header::render_header;
use settings::{DEFAULT_LRCLIB_URL, SettingsState, render_settings_pane};

use crate::lyrics_worker::{ActiveLyrics, LyricsStatus, fetch_lyrics, lyrics_query};
use crate::now_playing::{
    NowPlaying, NowPlayingController, NowPlayingEvent, spawn_now_playing_events,
};

pub fn app(cx: &mut RenderCx) -> Element {
    let (playback, set_playback) = cx.use_async_state(None::<NowPlaying>);
    let (playback_error, set_playback_error) = cx.use_async_state(None::<String>);
    let (lyrics, set_lyrics) = cx.use_async_state(LyricsStatus::default());
    let (_, tick) = cx.use_reducer(0_u64);
    let playback_controller = cx.use_ref(None::<NowPlayingController>);
    let lyrics_runtime = cx.use_memo((), lyrics_runtime);
    let lyrics_providers = cx.use_memo((), || {
        Arc::new(LyricsProviderChain::lrclib(DEFAULT_LRCLIB_URL))
    });
    let playback_controls = playback_controller.borrow().clone();

    let controller_for_effect = playback_controller.clone();
    cx.use_effect((), move || {
        let missing_controller = controller_for_effect.borrow().is_none();
        if missing_controller {
            controller_for_effect.set(Some(spawn_now_playing_events(move |event| match event {
                NowPlayingEvent::Changed(playback) => {
                    set_playback_error.call(None);
                    set_playback.call(playback);
                }
                NowPlayingEvent::Error(error) => {
                    set_playback_error.call(Some(error));
                }
            })));
        }
    });

    let lyrics_playback = playback.clone();
    let lyrics_runtime = lyrics_runtime.clone();
    let lyrics_providers = lyrics_providers.clone();
    cx.use_effect_with_cleanup(
        playback.as_ref().map(|value| value.key.clone()),
        move || {
            let Some(playback) = lyrics_playback else {
                set_lyrics.call(LyricsStatus::Idle);
                return None;
            };

            let key = playback.key.clone();
            let query = lyrics_query(&playback);
            set_lyrics.call(LyricsStatus::Loading(key.clone()));

            let providers = lyrics_providers.clone();
            let setter = set_lyrics.clone();
            let task = lyrics_runtime.spawn(async move {
                setter.call(fetch_lyrics(providers, key, query).await);
            });

            Some(move || task.abort())
        },
    );

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

    render_app(
        playback.as_ref(),
        playback_error.as_deref(),
        &lyrics,
        playback_controls,
        cx,
    )
}

fn lyrics_runtime() -> Arc<Runtime> {
    Arc::new(
        Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("valid lyrics lookup runtime"),
    )
}

fn status_for(lyrics: ActiveLyrics<'_>) -> String {
    match lyrics {
        ActiveLyrics::Idle => "Listening for media...".to_string(),
        ActiveLyrics::Loading => "Finding lyrics...".to_string(),
        ActiveLyrics::Ready(
            LyricsResult::Synced { source, .. }
            | LyricsResult::Plain { source, .. }
            | LyricsResult::Instrumental { source },
        ) => format!("Lyrics from {source}"),
        ActiveLyrics::Missing => "No lyrics found.".to_string(),
        ActiveLyrics::Error(_) => "Lyrics lookup failed.".to_string(),
    }
}

fn render_app(
    playback: Option<&NowPlaying>,
    playback_error: Option<&str>,
    lyrics: &LyricsStatus,
    playback_controls: Option<NowPlayingController>,
    cx: &mut RenderCx,
) -> Element {
    let inner_size = cx.use_inner_size();
    let is_dark = matches!(cx.use_color_scheme(), ColorScheme::Dark);
    let (settings_open, set_settings_open) = cx.use_state(false);
    let (settings_visible, set_settings_visible) = cx.use_state(false);
    let (settings, set_settings) = cx.use_state(SettingsState::default());

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
        .unwrap_or_else(|| status_for(active_lyrics));
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
        render_settings_pane(settings, set_settings, is_dark, move || {
            close_settings.call(false)
        })
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
