//! The client/server protocol: one JSON object per line over TCP.
//!
//! A session goes: client `hello` → server `welcome` (deck list) → client
//! `join` a room → server `waiting` until a second player joins the same room →
//! server `started` → then a `state` after every change, which the client
//! answers with `act` whenever its prompt asks for a decision.

use futures::{SinkExt, StreamExt};
use magus_core::CardPool;
use magus_core::{Action, GameView, PlayerId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 7878;
const MAX_LINE: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        name: String,
        protocol: u32,
    },
    Join {
        room: String,
        deck: String,
    },
    /// `version` must match the latest `GameView::version`, so a stale action
    /// can never be applied to a game that has moved on.
    Act {
        version: u64,
        action: Action,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        decks: Vec<DeckInfo>,
    },
    Waiting {
        room: String,
    },
    Started {
        seat: PlayerId,
        players: Vec<String>,
    },
    State {
        view: Box<GameView>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckInfo {
    pub key: String,
    pub name: String,
    pub description: String,
}

/// The decks a server offers, for its welcome message.
pub fn deck_infos(pool: &CardPool) -> Vec<DeckInfo> {
    pool.decks()
        .iter()
        .map(|d| DeckInfo {
            key: d.key.into(),
            name: d.name.into(),
            description: d.description.into(),
        })
        .collect()
}

pub fn encode<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("protocol messages always serialize")
}

pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line)
}

pub type LineReader = FramedRead<OwnedReadHalf, LinesCodec>;
pub type LineWriter = FramedWrite<OwnedWriteHalf, LinesCodec>;

/// Splits a connection into line-framed halves. Overlong lines are an error.
pub fn split(stream: TcpStream) -> (LineReader, LineWriter) {
    let _ = stream.set_nodelay(true);
    let (r, w) = stream.into_split();
    (
        FramedRead::new(r, LinesCodec::new_with_max_length(MAX_LINE)),
        FramedWrite::new(w, LinesCodec::new()),
    )
}

/// Connects to a server, returning a channel for outgoing messages and one for
/// incoming messages. The incoming channel closes when the connection drops.
pub async fn connect(
    addr: &str,
) -> std::io::Result<(
    mpsc::UnboundedSender<ClientMsg>,
    mpsc::UnboundedReceiver<ServerMsg>,
)> {
    let stream = TcpStream::connect(addr).await?;
    let (mut reader, mut writer) = split(stream);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMsg>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<ServerMsg>();
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if writer.send(encode(&msg)).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(line)) = reader.next().await {
            match decode::<ServerMsg>(&line) {
                Ok(msg) => {
                    if in_tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = in_tx.send(ServerMsg::Error {
                        message: format!("unreadable server message: {e}"),
                    });
                }
            }
        }
    });
    Ok((out_tx, in_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use magus_core::{Game, Target};

    #[test]
    fn messages_round_trip() {
        let msg = ClientMsg::Act {
            version: 3,
            action: Action::Cast {
                card: magus_core::ObjectId(9),
                target: Some(Target::Player(1)),
            },
        };
        let line = encode(&msg);
        assert_eq!(
            line,
            r#"{"type":"act","version":3,"action":{"type":"cast","card":9,"target":{"kind":"player","id":1}}}"#
        );
        assert_eq!(decode::<ClientMsg>(&line).unwrap(), msg);

        let pool = CardPool::builtin();
        let seats = [
            ("A".to_string(), pool.decks()[0]),
            ("B".to_string(), pool.decks()[1]),
        ];
        let view = Game::new(&pool, &seats, 1).view(0);
        let msg = ServerMsg::State {
            view: Box::new(view),
        };
        assert_eq!(decode::<ServerMsg>(&encode(&msg)).unwrap(), msg);
    }
}
