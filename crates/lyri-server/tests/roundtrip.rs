use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use lyri_server::client::{ClientCommand, ClientEvent, run_view_client};
use lyri_server::host::{Snapshot, ViewHost};
use lyri_server::protocol::{Playback, ViewState};

const NO_SEEK: u64 = u64::MAX;

struct Host {
    state: watch::Sender<Snapshot>,
    shutdown: watch::Sender<bool>,
    seeked: Arc<AtomicU64>,
    addr: std::net::SocketAddr,
}

struct Client {
    events: mpsc::UnboundedReceiver<ClientEvent>,
    commands: mpsc::UnboundedSender<ClientCommand>,
}

async fn start_host(allow_seek: bool) -> Host {
    let (state, state_rx) = watch::channel(Snapshot::default());
    let (shutdown, shutdown_rx) = watch::channel(false);
    let seeked = Arc::new(AtomicU64::new(NO_SEEK));
    let on_seek = {
        let seeked = seeked.clone();
        Arc::new(move |position: Duration| {
            seeked.store(position.as_millis() as u64, Ordering::SeqCst);
        })
    };

    let host = ViewHost::bind(
        "127.0.0.1:0".parse().unwrap(),
        allow_seek,
        state_rx,
        on_seek,
        shutdown_rx,
    )
    .await
    .unwrap();
    let addr = host.local_addr();
    tokio::spawn(host.serve());

    Host {
        state,
        shutdown,
        seeked,
        addr,
    }
}

fn connect(addr: std::net::SocketAddr) -> Client {
    let (events_tx, events) = mpsc::unbounded_channel();
    let (commands, commands_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        run_view_client(&addr.ip().to_string(), addr.port(), commands_rx, move |event| {
            let _ = events_tx.send(event);
        })
        .await;
    });

    Client { events, commands }
}

impl Client {
    async fn next_event(&mut self) -> ClientEvent {
        timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("event within 5s")
            .expect("client still running")
    }

    async fn next_playback(&mut self) -> Playback {
        loop {
            if let ClientEvent::State(state) = self.next_event().await
                && let Some(playback) = state.playback
            {
                return playback;
            }
        }
    }
}

fn playing_at(position_ms: u64) -> ViewState {
    ViewState {
        playback: Some(Playback {
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_ms: Some(180_000),
            position_ms,
            is_playing: true,
            can_seek: true,
        }),
        lyrics: None,
    }
}

#[tokio::test]
async fn streams_state_and_forwards_allowed_seeks() {
    let host = start_host(true).await;
    let mut client = connect(host.addr);

    assert_eq!(client.next_event().await, ClientEvent::Connected);
    assert_eq!(
        client.next_event().await,
        ClientEvent::State(ViewState::default())
    );

    host.state.send(Snapshot::new(playing_at(42_000))).unwrap();
    let playback = client.next_playback().await;
    assert_eq!(playback.title, "Song");
    assert!(playback.can_seek);
    assert!((42_000..43_000).contains(&playback.position_ms));

    client
        .commands
        .send(ClientCommand::Seek(Duration::from_millis(1234)))
        .unwrap();
    timeout(Duration::from_secs(5), async {
        while host.seeked.load(Ordering::SeqCst) == NO_SEEK {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("seek forwarded to the host");
    assert_eq!(host.seeked.load(Ordering::SeqCst), 1234);

    client.commands.send(ClientCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn masks_and_ignores_seeks_when_not_allowed() {
    let host = start_host(false).await;
    let mut client = connect(host.addr);

    assert_eq!(client.next_event().await, ClientEvent::Connected);
    client.next_event().await; // initial empty state

    host.state.send(Snapshot::new(playing_at(42_000))).unwrap();
    let playback = client.next_playback().await;
    assert!(!playback.can_seek, "host must mask can_seek");

    client
        .commands
        .send(ClientCommand::Seek(Duration::from_millis(1234)))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        host.seeked.load(Ordering::SeqCst),
        NO_SEEK,
        "seek must not reach the host when disallowed"
    );

    client.commands.send(ClientCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn late_joiners_receive_an_aged_position() {
    let host = start_host(false).await;

    host.state.send(Snapshot::new(playing_at(42_000))).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut client = connect(host.addr);
    assert_eq!(client.next_event().await, ClientEvent::Connected);
    let playback = client.next_playback().await;
    assert!(
        playback.position_ms >= 42_300,
        "expected the snapshot to be aged past 42300, got {}",
        playback.position_ms
    );

    client.commands.send(ClientCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn shutdown_disconnects_connected_viewers() {
    let host = start_host(false).await;
    let mut client = connect(host.addr);

    assert_eq!(client.next_event().await, ClientEvent::Connected);
    client.next_event().await; // initial state

    host.shutdown.send(true).unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let ClientEvent::Disconnected(_) = client.next_event().await {
                break;
            }
        }
    })
    .await
    .expect("viewer disconnected after host shutdown");

    client.commands.send(ClientCommand::Shutdown).unwrap();
}

#[tokio::test]
async fn serves_the_web_view_over_http() {
    let host = start_host(false).await;

    let response = reqwest::get(format!("http://{}/", host.addr)).await.unwrap();
    assert_eq!(
        response.headers()["cache-control"],
        "no-cache",
        "the entry page must revalidate so app updates reach browsers"
    );
    let body = response.error_for_status().unwrap().text().await.unwrap();
    assert!(body.contains("<!doctype html>"), "expected an HTML page");
}

#[tokio::test]
async fn stale_asset_requests_fail_instead_of_serving_html() {
    let host = start_host(false).await;

    let response = reqwest::get(format!("http://{}/assets/index-STALEHASH.js", host.addr))
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
