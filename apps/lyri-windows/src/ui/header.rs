use windows_reactor::*;

use crate::now_playing::NowPlaying;
use lyri_core::lyrics::{artist_album_label, format_duration};

pub(super) fn render_header(playback: Option<&NowPlaying>, status: &str) -> StackPanel {
    let title_text = playback
        .map(|playback| playback.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Lyri");

    let subtitle_text = playback
        .map(|playback| artist_album_label(&playback.artist, &playback.album))
        .unwrap_or_else(|| status.to_string());
    let time_text = playback.map(format_playback_time).unwrap_or_default();

    StackPanel::new()
        .spacing(4.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .children((
            centered_text(title_text)
                .font_size(22.0)
                .font_weight(FontWeight::SEMI_BOLD),
            centered_text(subtitle_text).font_size(13.0).opacity(0.68),
            TextBlock::new()
                .text(time_text)
                .font_size(12.0)
                .opacity(0.56)
                .horizontal_alignment(HorizontalAlignment::Center),
        ))
}

fn centered_text(text: impl AsRef<str>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .text_wrapping(TextWrapping::Wrap)
        .text_alignment(TextAlignment::Center)
        .horizontal_alignment(HorizontalAlignment::Center)
}

fn format_playback_time(playback: &NowPlaying) -> String {
    let position = format_duration(playback.current_position());
    let play_state = if playback.is_playing { "" } else { " paused" };

    match playback.duration {
        Some(duration) => format!("{position} / {}{play_state}", format_duration(duration)),
        None => format!("{position}{play_state}"),
    }
}
