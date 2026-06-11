use anyhow::{Context, Result};
use reqwest::Client;
use reqwest::StatusCode;
use serde::Deserialize;

use lyri_core::lrc::parse_synced_lrc;
use lyri_core::lyrics::{LyricsProvider, LyricsQuery, LyricsResult};

const SOURCE: &str = "LRCLIB";

pub struct LrclibProvider {
    client: Client,
    base_url: String,
}

impl LrclibProvider {
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .user_agent("Lyri/0.1")
            .build()
            .expect("valid LRCLIB HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn get_exact(&self, query: &LyricsQuery) -> Result<Option<LrclibLyrics>> {
        let Some(duration) = query.duration else {
            return Ok(None);
        };

        let url = format!(
            "{}/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
            self.base_url,
            encode(&query.title),
            encode(&query.artist),
            encode(&query.album),
            duration.as_secs()
        );

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("LRCLIB exact lookup")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = response.error_for_status().context("LRCLIB exact status")?;
        response
            .json::<LrclibLyrics>()
            .await
            .context("LRCLIB exact JSON")
            .map(Some)
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Option<LrclibLyrics>> {
        let url = format!(
            "{}/api/search?track_name={}&artist_name={}",
            self.base_url,
            encode(&query.title),
            encode(&query.artist)
        );

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("LRCLIB search lookup")?
            .error_for_status()
            .context("LRCLIB search status")?;

        let candidates = response
            .json::<Vec<LrclibLyrics>>()
            .await
            .context("LRCLIB search JSON")?;

        Ok(best_candidate(query, candidates))
    }
}

impl LyricsProvider for LrclibProvider {
    fn name(&self) -> &'static str {
        SOURCE
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery) -> lyri_core::lyrics::LyricsFetchFuture<'a> {
        Box::pin(async move {
            let lyrics = match self.get_exact(query).await? {
                Some(lyrics) => Some(lyrics),
                None => self.search(query).await?,
            };

            Ok(lyrics.and_then(map_response))
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibLyrics {
    track_name: String,
    artist_name: String,
    album_name: String,
    duration: f64,
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

fn map_response(response: LrclibLyrics) -> Option<LyricsResult> {
    if response.instrumental {
        return Some(LyricsResult::Instrumental {
            source: SOURCE.to_string(),
        });
    }

    if let Some(synced) = response.synced_lyrics.as_deref() {
        let lines = parse_synced_lrc(synced)
            .into_iter()
            .map(|mut line| {
                line.text = line.text.trim().to_string();
                line
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            return Some(LyricsResult::Synced {
                lines,
                source: SOURCE.to_string(),
            });
        }
    }

    response
        .plain_lyrics
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .map(|text| LyricsResult::Plain {
            text,
            source: SOURCE.to_string(),
        })
}

fn best_candidate(query: &LyricsQuery, candidates: Vec<LrclibLyrics>) -> Option<LrclibLyrics> {
    candidates
        .into_iter()
        .filter(|candidate| normalized(&candidate.track_name) == normalized(&query.title))
        .filter(|candidate| {
            query.artist.trim().is_empty()
                || normalized(&candidate.artist_name).contains(&normalized(&query.artist))
                || normalized(&query.artist).contains(&normalized(&candidate.artist_name))
        })
        .min_by_key(|candidate| score_candidate(query, candidate))
}

fn score_candidate(query: &LyricsQuery, candidate: &LrclibLyrics) -> u64 {
    let duration_penalty = query
        .duration
        .map(|duration| (duration.as_secs_f64() - candidate.duration).abs().round() as u64)
        .unwrap_or(0);

    let album_penalty = if query.album.trim().is_empty()
        || normalized(&query.album) == normalized(&candidate.album_name)
    {
        0
    } else {
        5
    };

    duration_penalty + album_penalty
}

fn normalized(value: &str) -> String {
    value.trim().to_lowercase()
}

fn encode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn response() -> LrclibLyrics {
        LrclibLyrics {
            track_name: "Song".to_string(),
            artist_name: "Artist".to_string(),
            album_name: "Album".to_string(),
            duration: 120.0,
            instrumental: false,
            plain_lyrics: Some("plain".to_string()),
            synced_lyrics: None,
        }
    }

    #[test]
    fn maps_synced_before_plain() {
        let mut response = response();
        response.synced_lyrics = Some("[00:01.00] synced ".to_string());

        match map_response(response) {
            Some(LyricsResult::Synced { lines, source }) => {
                assert_eq!(source, SOURCE);
                assert_eq!(lines[0].text, "synced");
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn maps_plain_when_synced_missing() {
        assert!(matches!(
            map_response(response()),
            Some(LyricsResult::Plain { text, .. }) if text == "plain"
        ));
    }

    #[test]
    fn trims_plain_lyrics_when_mapping() {
        let mut response = response();
        response.plain_lyrics = Some(" plain ".to_string());

        assert!(matches!(
            map_response(response),
            Some(LyricsResult::Plain { text, .. }) if text == "plain"
        ));
    }

    #[test]
    fn maps_instrumental() {
        let mut response = response();
        response.instrumental = true;

        assert!(matches!(
            map_response(response),
            Some(LyricsResult::Instrumental { .. })
        ));
    }

    #[test]
    fn search_candidate_prefers_closest_duration() {
        let query = LyricsQuery {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration: Some(Duration::from_secs(121)),
        };

        let mut far = response();
        far.duration = 200.0;
        let mut close = response();
        close.duration = 122.0;

        let best = best_candidate(&query, vec![far, close]).unwrap();
        assert_eq!(best.duration, 122.0);
    }

    #[test]
    fn accepts_float_duration_from_lrclib() {
        let payload = r#"{
            "trackName": "Song",
            "artistName": "Artist",
            "albumName": "Album",
            "duration": 181.0,
            "instrumental": false,
            "plainLyrics": "plain",
            "syncedLyrics": null
        }"#;

        let parsed: LrclibLyrics = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed.duration, 181.0);
    }
}
