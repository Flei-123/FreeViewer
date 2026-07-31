//! Relay transport: a plain WebSocket connection (wss:// by default).
//!
//! The relay is a dumb pipe: JSON text frames are control messages for the
//! relay itself, binary frames are forwarded verbatim to the paired peer.

use anyhow::Result;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn connect(url: &str) -> Result<Ws> {
    let (ws, _resp) = connect_async(url).await?;
    Ok(ws)
}

/// Registers this machine. The name is what other people see in their partner
/// list; it is the only thing besides the ID the relay ever learns about us.
pub fn json_register(secret: &str, name: &str) -> String {
    format!(
        "{{\"t\":\"host_register\",\"secret\":\"{}\",\"name\":\"{}\"}}",
        secret,
        crate::presence::clean(name)
    )
}

pub fn json_connect(id: &str) -> String {
    let digits: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
    format!("{{\"t\":\"connect\",\"id\":\"{}\"}}", digits)
}

pub fn json_bye() -> String {
    "{\"t\":\"bye\"}".to_string()
}

/// Reads the "t" field of a relay control message.
pub fn msg_type(v: &serde_json::Value) -> &str {
    v.get("t").and_then(|x| x.as_str()).unwrap_or("")
}
