//! JSON protocol spoken between a hosting Lyri instance, browser clients,
//! and Lyri apps mirroring a remote host.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use lyri_core::lyrics::{ActiveLyrics, LyricsResult, LyricsStatus, TimedLine, TrackKey};

/// Everything a viewer needs to render the lyrics screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    pub playback: Option<Playback>,
    pub lyrics: Option<Lyrics>,
}

impl ViewState {
    /// Masks seeking when the host does not allow viewers to seek.
    pub fn with_seek_allowed(mut self, allowed: bool) -> Self {
        if let Some(playback) = &mut self.playback {
            playback.can_seek &= allowed;
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playback {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    /// Position at the moment the state was sent; viewers extrapolate from
    /// receipt time while `is_playing`.
    pub position_ms: u64,
    pub is_playing: bool,
    pub can_seek: bool,
}

impl Playback {
    pub fn track_key(&self) -> TrackKey {
        TrackKey::new(&self.title, &self.artist)
    }

    pub fn position(&self) -> Duration {
        Duration::from_millis(self.position_ms)
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration_ms.map(Duration::from_millis)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Lyrics {
    Loading,
    Missing,
    Error { message: String },
    Instrumental { source: String },
    Plain { text: String, source: String },
    Synced { lines: Vec<Line>, source: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub timestamp_ms: u64,
    pub text: String,
}

impl Lyrics {
    /// Protocol form of the current track's lyrics; `None` when idle. Taking
    /// [`ActiveLyrics`] keeps lyrics for another track from being published.
    pub fn from_active(active: ActiveLyrics<'_>) -> Option<Self> {
        match active {
            ActiveLyrics::Idle => None,
            ActiveLyrics::Loading => Some(Self::Loading),
            ActiveLyrics::Missing => Some(Self::Missing),
            ActiveLyrics::Error(message) => Some(Self::Error {
                message: message.to_string(),
            }),
            ActiveLyrics::Ready(LyricsResult::Instrumental { source }) => {
                Some(Self::Instrumental {
                    source: source.clone(),
                })
            }
            ActiveLyrics::Ready(LyricsResult::Plain { text, source }) => Some(Self::Plain {
                text: text.clone(),
                source: source.clone(),
            }),
            ActiveLyrics::Ready(LyricsResult::Synced { lines, source }) => Some(Self::Synced {
                lines: lines
                    .iter()
                    .map(|line| Line {
                        timestamp_ms: line.timestamp.as_millis() as u64,
                        text: line.text.clone(),
                    })
                    .collect(),
                source: source.clone(),
            }),
        }
    }

    pub fn into_status(self, key: TrackKey) -> LyricsStatus {
        match self {
            Self::Loading => LyricsStatus::Loading(key),
            Self::Missing => LyricsStatus::Missing(key),
            Self::Error { message } => LyricsStatus::Error(key, message),
            Self::Instrumental { source } => {
                LyricsStatus::Ready(key, LyricsResult::Instrumental { source })
            }
            Self::Plain { text, source } => {
                LyricsStatus::Ready(key, LyricsResult::Plain { text, source })
            }
            Self::Synced { lines, source } => LyricsStatus::Ready(
                key,
                LyricsResult::Synced {
                    lines: lines
                        .into_iter()
                        .map(|line| TimedLine {
                            timestamp: Duration::from_millis(line.timestamp_ms),
                            text: line.text,
                        })
                        .collect(),
                    source,
                },
            ),
        }
    }
}

/// Messages pushed from the host to viewers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    State(ViewState),
}

/// Messages viewers may send to the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientMessage {
    Seek { position_ms: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ViewState {
        ViewState {
            playback: Some(Playback {
                title: "Song".to_string(),
                artist: "Artist".to_string(),
                album: "Album".to_string(),
                duration_ms: Some(180_000),
                position_ms: 42_000,
                is_playing: true,
                can_seek: true,
            }),
            lyrics: Some(Lyrics::Synced {
                lines: vec![Line {
                    timestamp_ms: 1_000,
                    text: "hello".to_string(),
                }],
                source: "LRCLIB".to_string(),
            }),
        }
    }

    #[test]
    fn server_message_round_trips_as_tagged_json() {
        let message = ServerMessage::State(state());
        let json = serde_json::to_string(&message).unwrap();

        assert!(json.contains(r#""type":"state""#));
        assert!(json.contains(r#""positionMs":42000"#));
        assert!(json.contains(r#""kind":"synced""#));
        assert_eq!(serde_json::from_str::<ServerMessage>(&json).unwrap(), message);
    }

    #[test]
    fn client_seek_message_shape_is_stable() {
        let json = serde_json::to_string(&ClientMessage::Seek { position_ms: 1234 }).unwrap();
        assert_eq!(json, r#"{"type":"seek","positionMs":1234}"#);
    }

    #[test]
    fn seek_mask_disables_seeking() {
        let masked = state().with_seek_allowed(false);
        assert!(!masked.playback.unwrap().can_seek);
    }

    #[test]
    fn lyrics_status_round_trips_through_protocol() {
        let key = TrackKey::new("Song", "Artist");
        let status = LyricsStatus::Ready(
            key.clone(),
            LyricsResult::Synced {
                lines: vec![TimedLine {
                    timestamp: Duration::from_millis(1_000),
                    text: "hello".to_string(),
                }],
                source: "LRCLIB".to_string(),
            },
        );

        let lyrics = Lyrics::from_active(status.active(Some(&key))).unwrap();
        assert_eq!(lyrics.into_status(key.clone()), status);
        assert_eq!(Lyrics::from_active(LyricsStatus::Idle.active(Some(&key))), None);
    }

    #[test]
    fn lyrics_for_another_track_are_not_published() {
        let playing = TrackKey::new("Song", "Artist");
        let stale = LyricsStatus::Missing(TrackKey::new("Other", "Artist"));

        assert_eq!(Lyrics::from_active(stale.active(Some(&playing))), None);
    }
}
