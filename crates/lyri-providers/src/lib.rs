use anyhow::Result;
use lyri_core::lyrics::{LyricsProvider, LyricsQuery, LyricsResult};

pub mod lrclib;
pub mod worker;

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

    pub async fn fetch(&self, query: &LyricsQuery) -> Result<Option<LyricsResult>> {
        for provider in &self.providers {
            if let Some(result) = provider.fetch(query).await? {
                return Ok(Some(result));
            }
        }

        Ok(None)
    }
}
