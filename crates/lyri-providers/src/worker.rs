//! Shared lyrics-fetch state machine used by the platform apps.

use std::sync::Arc;

use lyri_core::lyrics::{LyricsQuery, LyricsResult, TrackKey};

use crate::LyricsProviderChain;

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

pub async fn fetch_lyrics(
    providers: Arc<LyricsProviderChain>,
    key: TrackKey,
    query: LyricsQuery,
) -> LyricsStatus {
    match providers.fetch(&query).await {
        Ok(Some(result)) => LyricsStatus::Ready(key, result),
        Ok(None) => LyricsStatus::Missing(key),
        Err(error) => LyricsStatus::Error(key, format!("{error:#}")),
    }
}
