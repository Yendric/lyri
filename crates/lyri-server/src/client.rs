//! Mirrors a remote Lyri host: streams its [`ViewState`] and forwards seek
//! requests, reconnecting until shut down.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::{ClientMessage, ServerMessage, ViewState};

const RECONNECT_DELAY: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    Connected,
    State(ViewState),
    Disconnected(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientCommand {
    Seek(Duration),
    Shutdown,
}

/// Streams a remote host's state to `on_event` until a
/// [`ClientCommand::Shutdown`] arrives (or all command senders drop).
pub async fn run_view_client(
    host: &str,
    port: u16,
    mut commands: UnboundedReceiver<ClientCommand>,
    on_event: impl Fn(ClientEvent),
) {
    let url = format!("ws://{host}:{port}/ws");

    loop {
        match connect_async(&url).await {
            Ok((socket, _)) => {
                on_event(ClientEvent::Connected);
                if mirror(socket, &mut commands, &on_event).await.is_break() {
                    return;
                }
            }
            Err(error) => on_event(ClientEvent::Disconnected(error.to_string())),
        }

        // Wait before reconnecting, still reacting promptly to shutdown.
        tokio::select! {
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
            command = commands.recv() => {
                if matches!(command, Some(ClientCommand::Shutdown) | None) {
                    return;
                }
            }
        }
    }
}

async fn mirror(
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    commands: &mut UnboundedReceiver<ClientCommand>,
    on_event: &impl Fn(ClientEvent),
) -> std::ops::ControlFlow<()> {
    loop {
        tokio::select! {
            received = socket.next() => match received {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ServerMessage::State(state)) = serde_json::from_str(text.as_str()) {
                        on_event(ClientEvent::State(state));
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    on_event(ClientEvent::Disconnected(error.to_string()));
                    return std::ops::ControlFlow::Continue(());
                }
                None => {
                    on_event(ClientEvent::Disconnected("connection closed".to_string()));
                    return std::ops::ControlFlow::Continue(());
                }
            },
            command = commands.recv() => match command {
                Some(ClientCommand::Seek(position)) => {
                    let message = serde_json::to_string(&ClientMessage::Seek {
                        position_ms: position.as_millis() as u64,
                    })
                    .expect("seek message serializes to JSON");
                    // A send failure surfaces as a read error on the next poll.
                    let _ = socket.send(Message::text(message)).await;
                }
                Some(ClientCommand::Shutdown) | None => {
                    let _ = socket.close(None).await;
                    return std::ops::ControlFlow::Break(());
                }
            }
        }
    }
}
