use windows_reactor::*;

use super::common::{centered_stack, empty_state};
use super::synced_lyrics::{LyricsSeekTarget, SyncedLyrics, SyncedLyricsInput};
use crate::external_source::PlaybackControls;
use crate::now_playing::NowPlaying;
use lyri_core::lyrics::{ActiveLyrics, LyricsResult};

pub(super) struct Body {
    pub(super) view: View,
    /// Whether the view scrolls under the header instead of sitting below it.
    pub(super) behind_header: bool,
}

pub(super) fn render_body(
    playback: Option<&NowPlaying>,
    lyrics: ActiveLyrics<'_>,
    status: &str,
    playback_controls: Option<PlaybackControls>,
    content_width: f64,
) -> Body {
    let view = match (playback, lyrics) {
        (None, _) => empty_state("Play a song and Lyri will follow along."),
        (Some(_), ActiveLyrics::Idle | ActiveLyrics::Loading) => {
            centered_stack(StackPanel::new().spacing(12.0).children((
                ProgressRing::new().is_active(true),
                TextBlock::new().text(status),
            )))
        }
        (Some(_), ActiveLyrics::Missing) => empty_state("No lyrics found for this track."),
        (Some(_), ActiveLyrics::Error(error)) => centered_stack(
            StackPanel::new().spacing(8.0).children((
                TextBlock::new()
                    .text("Could not load lyrics")
                    .font_size(20.0)
                    .font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new()
                    .text(error)
                    .font_size(13.0)
                    .opacity(0.65)
                    .text_wrapping(TextWrapping::Wrap),
            )),
        ),
        (Some(_), ActiveLyrics::Ready(LyricsResult::Instrumental { .. })) => {
            empty_state("Instrumental track")
        }
        (Some(_), ActiveLyrics::Ready(LyricsResult::Plain { text, .. })) => render_plain(text),
        (Some(playback), ActiveLyrics::Ready(LyricsResult::Synced { lines, .. })) => {
            let view = component::<SyncedLyrics>(
                "synced-lyrics",
                SyncedLyricsInput {
                    track: playback.key.clone(),
                    lines: lines.clone(),
                    position: playback.current_position(),
                    viewport_width: content_width.max(220.0),
                    seek: playback_controls.and_then(|controller| {
                        playback.can_seek.then(|| LyricsSeekTarget {
                            key: playback.key.clone(),
                            controller,
                        })
                    }),
                },
            );
            return Body {
                view: view.into(),
                behind_header: true,
            };
        }
    };
    Body {
        view,
        behind_header: false,
    }
}

fn render_plain(text: &str) -> View {
    ScrollView::new()
        .content(
            TextBlock::new()
                .text(text)
                .font_size(20.0)
                .text_wrapping(TextWrapping::Wrap)
                .text_alignment(TextAlignment::Center)
                .opacity(0.86)
                .horizontal_alignment(HorizontalAlignment::Center),
        )
        .into()
}
