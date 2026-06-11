use std::sync::Arc;

use lyri_core::lyrics::{LyricsQuery, LyricsResult};
use lyri_providers::LyricsProviderChain;

use crate::now_playing::{NowPlaying, TrackKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsStatus {
    Idle,
    Loading(TrackKey),
    Ready(TrackKey, LyricsResult),
    Missing(TrackKey),
    Error(TrackKey, String),
}

impl Default for LyricsStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl LyricsStatus {
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

pub fn lyrics_query(playback: &NowPlaying) -> LyricsQuery {
    LyricsQuery {
        title: playback.title.clone(),
        artist: playback.artist.clone(),
        album: playback.album.clone(),
        duration: playback.duration,
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
