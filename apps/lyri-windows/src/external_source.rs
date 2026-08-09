//! Mirrors another Lyri instance on the network as the playback source.

use std::time::Duration;

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use lyri_core::lyrics::{LyricsStatus, TrackKey};
use lyri_server::client::{ClientCommand, ClientEvent, run_view_client};
use lyri_server::protocol::{Playback, ViewState};

use crate::now_playing::{NowPlaying, NowPlayingController};

/// Events emitted while mirroring a remote Lyri host.
#[derive(Clone, Debug)]
pub enum ExternalEvent {
    /// A fresh snapshot of the host's playback and lyrics.
    State(Box<MirroredState>),
    /// `Some(error)` while disconnected, `None` once (re)connected.
    ConnectionChanged(Option<String>),
}

#[derive(Clone, Debug)]
pub struct MirroredState {
    pub playback: Option<NowPlaying>,
    pub lyrics: LyricsStatus,
}

#[derive(Clone, Debug)]
pub struct ExternalController {
    tx: UnboundedSender<ClientCommand>,
}

impl ExternalController {
    pub fn seek_to(&self, position: Duration) -> bool {
        self.tx.send(ClientCommand::Seek(position)).is_ok()
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(ClientCommand::Shutdown);
    }
}

/// Follows the Lyri instance at `host:port`, reconnecting until shut down.
pub fn spawn_external_source(
    host: String,
    port: u16,
    emit: impl Fn(ExternalEvent) + Send + Sync + 'static,
) -> ExternalController {
    let (tx, rx) = unbounded_channel();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("valid external source runtime");

        runtime.block_on(run_view_client(&host, port, rx, |event| match event {
            ClientEvent::Connected => emit(ExternalEvent::ConnectionChanged(None)),
            ClientEvent::Disconnected(error) => {
                emit(ExternalEvent::ConnectionChanged(Some(error)));
            }
            ClientEvent::State(state) => emit(mirrored_state(state)),
        }));
    });

    ExternalController { tx }
}

fn mirrored_state(state: ViewState) -> ExternalEvent {
    let playback = state.playback.as_ref().map(mirrored_playback);
    let lyrics = match (&playback, state.lyrics) {
        (Some(playback), Some(lyrics)) => lyrics.into_status(playback.key.clone()),
        _ => LyricsStatus::Idle,
    };

    ExternalEvent::State(Box::new(MirroredState { playback, lyrics }))
}

fn mirrored_playback(remote: &Playback) -> NowPlaying {
    NowPlaying::remote(
        remote.title.clone(),
        remote.artist.clone(),
        remote.album.clone(),
        remote.duration(),
        remote.position(),
        remote.is_playing,
        remote.can_seek,
    )
}

/// Seek routing for whichever playback source is active.
#[derive(Clone, Debug)]
pub enum PlaybackControls {
    System(NowPlayingController),
    External(ExternalController),
}

impl PlaybackControls {
    pub fn seek_to(&self, key: TrackKey, position: Duration) -> bool {
        match self {
            // The host validates the seek against its own current track.
            Self::System(controller) => controller.seek_to(key, position),
            Self::External(controller) => controller.seek_to(position),
        }
    }

    pub fn shutdown(&self) {
        match self {
            Self::System(controller) => controller.shutdown(),
            Self::External(controller) => controller.shutdown(),
        }
    }
}
