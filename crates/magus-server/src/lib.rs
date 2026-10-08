//! The game server. Players join named rooms; when a room has two players a
//! game starts, owned by its own task, which is the single source of truth.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use magus_core::cards::{self, DeckList};
use magus_core::{Action, Game, PlayerId};
use magus_protocol::{
    ClientMsg, LineReader, PROTOCOL_VERSION, ServerMsg, deck_infos, decode, encode,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

const MAX_NAME_LEN: usize = 24;

struct Seat {
    name: String,
    deck: &'static DeckList,
    out: mpsc::UnboundedSender<ServerMsg>,
}

struct WaitingSeat {
    seat: Seat,
    notify: oneshot::Sender<Assignment>,
}

struct Assignment {
    seat: PlayerId,
    game: mpsc::UnboundedSender<(PlayerId, GameInput)>,
}

enum GameInput {
    Act { version: u64, action: Action },
    Left,
}

type Lobby = Arc<Mutex<HashMap<String, WaitingSeat>>>;

pub async fn serve(listener: TcpListener) -> std::io::Result<()> {
    let lobby: Lobby = Arc::default();
    loop {
        let (stream, addr) = listener.accept().await?;
        let lobby = lobby.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, lobby).await {
                tracing::warn!(%addr, "connection error: {e:#}");
            }
        });
    }
}

async fn next_msg(reader: &mut LineReader) -> anyhow::Result<Option<ClientMsg>> {
    match reader.next().await {
        None => Ok(None),
        Some(line) => Ok(Some(
            decode(&line.context("reading from client")?).context("decoding client message")?,
        )),
    }
}

async fn handle_connection(stream: TcpStream, lobby: Lobby) -> anyhow::Result<()> {
    let (mut reader, mut writer) = magus_protocol::split(stream);
    let (out, mut out_rx) = mpsc::unbounded_channel::<ServerMsg>();
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if writer.send(encode(&msg)).await.is_err() {
                break;
            }
        }
    });

    let name = match next_msg(&mut reader).await? {
        Some(ClientMsg::Hello { name, protocol }) => {
            if protocol != PROTOCOL_VERSION {
                let message =
                    format!("server speaks protocol {PROTOCOL_VERSION}, client speaks {protocol}");
                let _ = out.send(ServerMsg::Error { message });
                return Ok(());
            }
            let name: String = name
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME_LEN)
                .collect();
            if name.is_empty() {
                "Anonymous".to_string()
            } else {
                name
            }
        }
        Some(_) => bail!("expected hello"),
        None => return Ok(()),
    };
    let _ = out.send(ServerMsg::Welcome {
        decks: deck_infos(),
    });

    let (room, deck) = loop {
        match next_msg(&mut reader).await? {
            Some(ClientMsg::Join { room, deck }) => match cards::deck(&deck) {
                Some(deck) => break (room.trim().to_lowercase(), deck),
                None => {
                    let _ = out.send(ServerMsg::Error {
                        message: format!("unknown deck {deck:?}"),
                    });
                }
            },
            Some(_) => {
                let _ = out.send(ServerMsg::Error {
                    message: "join a room first".into(),
                });
            }
            None => return Ok(()),
        }
    };
    tracing::info!(%name, %room, deck = deck.key, "player joined");

    let me = Seat {
        name,
        deck,
        out: out.clone(),
    };
    let pending = {
        let mut rooms = lobby.lock().expect("lobby lock");
        match rooms.remove(&room) {
            Some(waiting) if !waiting.notify.is_closed() => {
                let (game_tx, game_rx) = mpsc::unbounded_channel();
                tokio::spawn(run_game(vec![waiting.seat, me], game_rx));
                if waiting
                    .notify
                    .send(Assignment {
                        seat: 0,
                        game: game_tx.clone(),
                    })
                    .is_err()
                {
                    // They hung up at the last moment; they forfeit.
                    let _ = game_tx.send((0, GameInput::Left));
                }
                Ok(Assignment {
                    seat: 1,
                    game: game_tx,
                })
            }
            _ => {
                let (tx, rx) = oneshot::channel();
                rooms.insert(
                    room.clone(),
                    WaitingSeat {
                        seat: me,
                        notify: tx,
                    },
                );
                Err(rx)
            }
        }
    };

    let assignment = match pending {
        Ok(assignment) => assignment,
        Err(mut rx) => {
            let _ = out.send(ServerMsg::Waiting { room: room.clone() });
            let assigned = loop {
                tokio::select! {
                    assignment = &mut rx => break assignment.ok(),
                    line = reader.next() => match line {
                        Some(Ok(_)) => continue,
                        _ => break None,
                    },
                }
            };
            match assigned {
                Some(assignment) => assignment,
                None => {
                    drop(rx);
                    lobby
                        .lock()
                        .expect("lobby lock")
                        .retain(|_, w| !w.notify.is_closed());
                    return Ok(());
                }
            }
        }
    };

    while let Some(line) = reader.next().await {
        let Ok(line) = line else { break };
        match decode::<ClientMsg>(&line) {
            Ok(ClientMsg::Act { version, action }) => {
                if assignment
                    .game
                    .send((assignment.seat, GameInput::Act { version, action }))
                    .is_err()
                {
                    break;
                }
            }
            Ok(_) => {
                let _ = out.send(ServerMsg::Error {
                    message: "you're already in a game".into(),
                });
            }
            Err(e) => {
                let _ = out.send(ServerMsg::Error {
                    message: format!("bad message: {e}"),
                });
            }
        }
    }
    let _ = assignment.game.send((assignment.seat, GameInput::Left));
    Ok(())
}

async fn run_game(seats: Vec<Seat>, mut inputs: mpsc::UnboundedReceiver<(PlayerId, GameInput)>) {
    let names: Vec<String> = seats.iter().map(|s| s.name.clone()).collect();
    let players: Vec<(String, &'static DeckList)> =
        seats.iter().map(|s| (s.name.clone(), s.deck)).collect();
    let mut game = Game::new(&players, rand::random());
    tracing::info!(players = %names.join(" vs "), "game started");

    let send_state = |game: &Game, p: PlayerId| {
        let _ = seats[p].out.send(ServerMsg::State {
            view: Box::new(game.view(p)),
        });
    };
    let broadcast = |game: &Game| (0..seats.len()).for_each(|p| send_state(game, p));

    for (p, seat) in seats.iter().enumerate() {
        let _ = seat.out.send(ServerMsg::Started {
            seat: p,
            players: names.clone(),
        });
    }
    broadcast(&game);

    while let Some((p, input)) = inputs.recv().await {
        match input {
            GameInput::Act { version, action } => {
                if version != game.version() {
                    let message = "that action was for an older game state; ignored".into();
                    let _ = seats[p].out.send(ServerMsg::Error { message });
                    send_state(&game, p);
                    continue;
                }
                match game.apply(p, action) {
                    Ok(()) => broadcast(&game),
                    Err(e) => {
                        let _ = seats[p].out.send(ServerMsg::Error {
                            message: e.to_string(),
                        });
                    }
                }
            }
            GameInput::Left => {
                if !game.is_over() && game.apply(p, Action::Concede).is_ok() {
                    broadcast(&game);
                }
            }
        }
    }
    tracing::info!(players = %names.join(" vs "), "game finished");
}
