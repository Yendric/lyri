use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

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

pub fn active_line_index(lines: &[TimedLine], position: Duration) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }

    let mut low = 0;
    let mut high = lines.len();

    while low < high {
        let mid = (low + high) / 2;
        if lines[mid].timestamp <= position {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    match low {
        0 => Some(0),
        index => Some(index - 1),
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

        assert_eq!(active_line_index(&lines, Duration::from_millis(0)), Some(0));
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
