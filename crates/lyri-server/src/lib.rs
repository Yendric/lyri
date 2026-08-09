//! [`host`] serves the built web view and streams state over WebSocket;
//! [`client`] mirrors a remote host. Both speak the JSON [`protocol`].

pub mod client;
pub mod host;
pub mod protocol;
