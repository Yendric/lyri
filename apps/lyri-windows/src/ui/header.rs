use windows_reactor::*;

use super::common::format_duration;
use crate::now_playing::NowPlaying;

pub(super) fn render_header(playback: Option<&NowPlaying>, status: &str) -> Element {
    let title_text = playback
        .map(|playback| playback.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Lyri");

    let subtitle_text = playback
        .map(format_artist_album)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| status.to_string());
    let time_text = playback
        .map(format_playback_time)
        .unwrap_or_else(|| status.to_string());

    vstack((
        text_block(title_text)
            .font_size(22.0)
            .semibold()
            .wrap()
            .text_alignment(TextAlignment::Center)
            .horizontal_alignment(HorizontalAlignment::Center),
        text_block(subtitle_text)
            .font_size(13.0)
            .opacity(0.68)
            .wrap()
            .text_alignment(TextAlignment::Center)
            .horizontal_alignment(HorizontalAlignment::Center),
        text_block(time_text)
            .font_size(12.0)
            .opacity(0.56)
            .horizontal_alignment(HorizontalAlignment::Center),
    ))
    .spacing(4.0)
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .into()
}

fn format_artist_album(playback: &NowPlaying) -> String {
    let artist = playback.artist.trim();
    let album = playback.album.trim();

    match (artist.is_empty(), album.is_empty()) {
        (false, false) => format!("{artist} - {album}"),
        (false, true) => artist.to_string(),
        (true, false) => album.to_string(),
        (true, true) => "Unknown artist".to_string(),
    }
}

fn format_playback_time(playback: &NowPlaying) -> String {
    let position = format_duration(playback.current_position());
    let play_state = if playback.is_playing { "" } else { " paused" };

    match playback.duration {
        Some(duration) => format!("{position} / {}{play_state}", format_duration(duration)),
        None => format!("{position}{play_state}"),
    }
}
