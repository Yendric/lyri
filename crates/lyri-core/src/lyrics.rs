use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

/// Identifies a track for matching lyrics results to the current playback,
/// independent of the platform media session that reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackKey {
    title: String,
    artist: String,
}

impl TrackKey {
    pub fn new(title: &str, artist: &str) -> Self {
        Self {
            title: title.trim().to_string(),
            artist: artist.trim().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedLine {
    pub timestamp: Duration,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsResult {
    Synced {
        lines: Vec<TimedLine>,
        source: String,
    },
    Plain {
        text: String,
        source: String,
    },
    Instrumental {
        source: String,
    },
}

pub type LyricsFetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<LyricsResult>>> + Send + 'a>>;

pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn fetch<'a>(&'a self, query: &'a LyricsQuery) -> LyricsFetchFuture<'a>;
}

/// Lifecycle of a lyrics lookup for a particular track.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LyricsStatus {
    #[default]
    Idle,
    Loading(TrackKey),
    Ready(TrackKey, LyricsResult),
    Missing(TrackKey),
    Error(TrackKey, String),
}

impl LyricsStatus {
    pub fn key(&self) -> Option<&TrackKey> {
        match self {
            Self::Idle => None,
            Self::Loading(key)
            | Self::Ready(key, _)
            | Self::Missing(key)
            | Self::Error(key, _) => Some(key),
        }
    }

    pub fn active(&self, key: Option<&TrackKey>) -> ActiveLyrics<'_> {
        match (key, self) {
            (Some(current), Self::Loading(key)) if key == current => ActiveLyrics::Loading,
            (Some(current), Self::Ready(key, result)) if key == current => {
                ActiveLyrics::Ready(result)
            }
            (Some(current), Self::Missing(key)) if key == current => ActiveLyrics::Missing,
            (Some(current), Self::Error(key, error)) if key == current => {
                ActiveLyrics::Error(error)
            }
            _ => ActiveLyrics::Idle,
        }
    }
}

/// A [`LyricsStatus`] narrowed to the currently playing track.
#[derive(Clone, Copy)]
pub enum ActiveLyrics<'a> {
    Idle,
    Loading,
    Ready(&'a LyricsResult),
    Missing,
    Error(&'a str),
}

impl ActiveLyrics<'_> {
    pub fn status_message(&self) -> String {
        match self {
            Self::Idle => "Listening for media...".to_string(),
            Self::Loading => "Finding lyrics...".to_string(),
            Self::Ready(
                LyricsResult::Synced { source, .. }
                | LyricsResult::Plain { source, .. }
                | LyricsResult::Instrumental { source },
            ) => format!("Lyrics from {source}"),
            Self::Missing => "No lyrics found.".to_string(),
            Self::Error(_) => "Lyrics lookup failed.".to_string(),
        }
    }
}

/// Index of the line currently being sung; `None` before the first line starts.
pub fn active_line_index(lines: &[TimedLine], position: Duration) -> Option<usize> {
    lines
        .partition_point(|line| line.timestamp <= position)
        .checked_sub(1)
}

pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn artist_album_label(artist: &str, album: &str) -> String {
    match (artist.trim(), album.trim()) {
        ("", "") => "Unknown artist".to_string(),
        (artist, "") => artist.to_string(),
        ("", album) => album.to_string(),
        (artist, album) => format!("{artist} - {album}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ms: u64) -> TimedLine {
        TimedLine {
            timestamp: Duration::from_millis(ms),
            text: ms.to_string(),
        }
    }

    #[test]
    fn active_line_tracks_position() {
        let lines = vec![line(1000), line(2000), line(4000)];

        assert_eq!(active_line_index(&lines, Duration::from_millis(0)), None);
        assert_eq!(
            active_line_index(&lines, Duration::from_millis(1000)),
            Some(0)
        );
        assert_eq!(
            active_line_index(&lines, Duration::from_millis(2500)),
            Some(1)
        );
        assert_eq!(
            active_line_index(&lines, Duration::from_millis(9000)),
            Some(2)
        );
    }

    #[test]
    fn active_line_handles_empty_lyrics() {
        assert_eq!(active_line_index(&[], Duration::ZERO), None);
    }

    #[test]
    fn active_line_uses_last_duplicate_timestamp() {
        let lines = vec![line(1000), line(1000), line(2000)];

        assert_eq!(
            active_line_index(&lines, Duration::from_millis(1000)),
            Some(1)
        );
    }
}
