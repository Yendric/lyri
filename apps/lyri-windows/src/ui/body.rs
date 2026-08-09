use windows_reactor::*;

use super::common::{centered_stack, empty_state};
use super::responsive_content_padding;
use super::synced_lyrics::{LyricsSeekTarget, SyncedLyricsProps, synced_lyrics};
use crate::external_source::PlaybackControls;
use crate::now_playing::NowPlaying;
use lyri_core::lyrics::{ActiveLyrics, LyricsResult};

pub(super) fn render_body(
    playback: Option<&NowPlaying>,
    lyrics: ActiveLyrics<'_>,
    status: &str,
    playback_controls: Option<PlaybackControls>,
    cx: &mut RenderCx,
) -> Element {
    let inner_size = cx.use_inner_size();
    let content_padding = responsive_content_padding(inner_size.width);
    let viewport_height = (inner_size.height - HEADER_RESERVE).max(220.0);
    let viewport_width = (inner_size.width - content_padding * 2.0).max(220.0);

    match (playback, lyrics) {
        (None, _) => empty_state("Play a song and Lyri will follow along."),
        (Some(_), ActiveLyrics::Idle | ActiveLyrics::Loading) => centered_stack(
            vstack(vec![
                Element::from(ProgressRing::indeterminate()),
                Element::from(text_block(status)),
            ])
            .spacing(12.0),
        ),
        (Some(_), ActiveLyrics::Missing) => empty_state("No lyrics found for this track."),
        (Some(_), ActiveLyrics::Error(error)) => centered_stack(
            vstack(vec![
                Element::from(
                    text_block("Could not load lyrics")
                        .font_size(20.0)
                        .semibold(),
                ),
                Element::from(text_block(error).font_size(13.0).opacity(0.65).wrap()),
            ])
            .spacing(8.0),
        ),
        (Some(_), ActiveLyrics::Ready(LyricsResult::Instrumental { .. })) => {
            empty_state("Instrumental track")
        }
        (Some(_), ActiveLyrics::Ready(LyricsResult::Plain { text, .. })) => {
            render_plain(text, viewport_height)
        }
        (Some(playback), ActiveLyrics::Ready(LyricsResult::Synced { lines, .. })) => component(
            synced_lyrics,
            SyncedLyricsProps {
                lines: lines.clone(),
                position: playback.current_position(),
                viewport_height,
                viewport_width,
                seek: playback_controls.and_then(|controller| {
                    playback.can_seek.then(|| LyricsSeekTarget {
                        key: playback.key.clone(),
                        controller,
                    })
                }),
            },
        ),
    }
}

fn render_plain(text: &str, viewport_height: f64) -> Element {
    scroll_view(
        text_block(text)
            .font_size(20.0)
            .wrap()
            .text_alignment(TextAlignment::Center)
            .opacity(0.86)
            .horizontal_alignment(HorizontalAlignment::Center),
    )
    .height(viewport_height)
    .vertical_alignment(VerticalAlignment::Stretch)
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .into()
}

const CONTENT_TOP_PADDING: f64 = 12.0;
const HEADER_ROW_SPACING: f64 = 28.0;
const HEADER_TEXT_HEIGHT: f64 = 52.0;
const VIEWPORT_OVERFILL: f64 = 24.0;

const HEADER_RESERVE: f64 =
    CONTENT_TOP_PADDING + HEADER_ROW_SPACING + HEADER_TEXT_HEIGHT - VIEWPORT_OVERFILL;
