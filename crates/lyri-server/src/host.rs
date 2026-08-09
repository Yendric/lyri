//! Hosts the web view: serves the built `lyri-web` app and streams
//! [`ViewState`] over a WebSocket to any viewer on the network.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use include_dir::{Dir, include_dir};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::protocol::{ClientMessage, ServerMessage, ViewState};

/// Web-view assets built from `apps/lyri-web` (a placeholder page when the
/// web app has not been built; see `build.rs`).
static WEB_ASSETS: Dir<'_> = include_dir!("$OUT_DIR/webview");

/// Invoked with the requested position when a viewer seeks.
pub type SeekHandler = Arc<dyn Fn(Duration) + Send + Sync>;

/// A published [`ViewState`] plus the moment it was captured, so positions
/// can be aged for viewers that connect long after the last media event.
#[derive(Clone, Debug)]
pub struct Snapshot {
    state: ViewState,
    taken: Instant,
}

impl Snapshot {
    pub fn new(state: ViewState) -> Self {
        Self {
            state,
            taken: Instant::now(),
        }
    }

    pub fn state(&self) -> &ViewState {
        &self.state
    }

    /// The state with its position advanced by the snapshot's age, honoring
    /// the wire contract that `position_ms` is current at send time.
    fn aged(&self, seek_allowed: bool) -> ViewState {
        let mut state = self.state.clone();
        if let Some(playback) = &mut state.playback
            && playback.is_playing
        {
            let position = playback
                .position_ms
                .saturating_add(self.taken.elapsed().as_millis() as u64);
            playback.position_ms = playback
                .duration_ms
                .map_or(position, |duration| position.min(duration));
        }

        state.with_seek_allowed(seek_allowed)
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new(ViewState::default())
    }
}

#[derive(Clone)]
struct HostState {
    state: watch::Receiver<Snapshot>,
    shutdown: watch::Receiver<bool>,
    allow_seek: bool,
    on_seek: SeekHandler,
}

/// A bound but not yet running web-view host.
pub struct ViewHost {
    local_addr: SocketAddr,
    listener: TcpListener,
    router: Router,
    shutdown: watch::Receiver<bool>,
}

impl ViewHost {
    /// Binds the host. Flipping `shutdown` to `true` (or dropping its sender)
    /// stops accepting and disconnects every viewer.
    pub async fn bind(
        bind: SocketAddr,
        allow_seek: bool,
        state: watch::Receiver<Snapshot>,
        on_seek: SeekHandler,
        shutdown: watch::Receiver<bool>,
    ) -> Result<Self> {
        let router = Router::new()
            .route("/ws", get(upgrade))
            .fallback(get(asset))
            .with_state(HostState {
                state,
                shutdown: shutdown.clone(),
                allow_seek,
                on_seek,
            });
        let listener = TcpListener::bind(bind)
            .await
            .with_context(|| format!("binding web view to {bind}"))?;
        let local_addr = listener.local_addr().context("web view local address")?;

        Ok(Self {
            local_addr,
            listener,
            router,
            shutdown,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn serve(self) -> Result<()> {
        let mut shutdown = self.shutdown;
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(async move {
                // Resolves on shutdown or when the sender is dropped.
                let _ = shutdown.wait_for(|stopped| *stopped).await;
            })
            .await
            .context("serving web view")
    }
}

async fn upgrade(upgrade: WebSocketUpgrade, State(host): State<HostState>) -> Response {
    upgrade.on_upgrade(move |socket| stream_state(socket, host))
}

async fn stream_state(mut socket: WebSocket, mut host: HostState) {
    if send_state(&mut socket, &mut host).await.is_err() {
        return;
    }

    let mut shutdown = host.shutdown.clone();
    loop {
        tokio::select! {
            changed = host.state.changed() => {
                if changed.is_err() || send_state(&mut socket, &mut host).await.is_err() {
                    return;
                }
            }
            received = socket.recv() => match received {
                Some(Ok(Message::Text(text))) => handle_client_message(&host, text.as_str()),
                Some(Ok(_)) => {}
                _ => return,
            },
            // Wrapped so the non-Send watch::Ref drops inside the arm.
            _ = async { let _ = shutdown.wait_for(|stopped| *stopped).await; } => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        }
    }
}

async fn send_state(socket: &mut WebSocket, host: &mut HostState) -> Result<(), axum::Error> {
    let state = host.state.borrow_and_update().aged(host.allow_seek);
    let message = serde_json::to_string(&ServerMessage::State(state))
        .expect("view state serializes to JSON");

    socket.send(Message::text(message)).await
}

fn handle_client_message(host: &HostState, text: &str) {
    match serde_json::from_str(text) {
        Ok(ClientMessage::Seek { position_ms }) if host.allow_seek => {
            (host.on_seek)(Duration::from_millis(position_ms));
        }
        _ => {}
    }
}

/// Serves the embedded web app, falling back to `index.html` for unknown
/// paths so client-side routing keeps working.
async fn asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = WEB_ASSETS.get_file(path) {
        return serve_file(file, cache_policy(path));
    }

    // Hashed assets must never fall back to index.html: a stale cached page
    // would receive HTML under a JavaScript MIME type instead of a clean 404.
    if path.starts_with("assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }

    match WEB_ASSETS.get_file("index.html") {
        Some(file) => serve_file(file, "no-cache"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn serve_file(file: &'static include_dir::File<'static>, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type(file.path())),
            (header::CACHE_CONTROL, cache),
        ],
        file.contents(),
    )
        .into_response()
}

fn cache_policy(path: &str) -> &'static str {
    // Vite content-hashes everything under assets/, so cache those forever;
    // the entry page must revalidate so app updates reach browsers.
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json" | "map") => "application/json",
        _ => "application/octet-stream",
    }
}
