use lyri_core::lyrics::{LyricsProvider, LyricsQuery, LyricsStatus, TrackKey};

pub mod lrclib;

pub struct LyricsProviderChain {
    providers: Vec<Box<dyn LyricsProvider>>,
}

impl LyricsProviderChain {
    pub fn new(providers: Vec<Box<dyn LyricsProvider>>) -> Self {
        Self { providers }
    }

    pub fn lrclib(base_url: &str) -> Self {
        Self::new(vec![Box::new(lrclib::LrclibProvider::new(base_url))])
    }

    pub async fn fetch(&self, key: TrackKey, query: &LyricsQuery) -> LyricsStatus {
        for provider in &self.providers {
            match provider.fetch(query).await {
                Ok(Some(result)) => return LyricsStatus::Ready(key, result),
                Ok(None) => {}
                Err(error) => return LyricsStatus::Error(key, format!("{error:#}")),
            }
        }

        LyricsStatus::Missing(key)
    }
}
