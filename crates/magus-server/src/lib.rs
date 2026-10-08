//! The game server. Players join named rooms; when a room has two players a
//! game starts, owned by its own task, which is the single source of truth.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use magus_core::cards::DeckList;
use magus_core::{Action, CardPool, Game, Pack, PlayerId};
use magus_protocol::{
    ClientMsg, CustomDeck, LineReader, PROTOCOL_VERSION, ServerMsg, deck_infos, decode, encode,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

const MAX_NAME_LEN: usize = 24;

struct Seat {
    name: String,
    deck: SeatDeck,
    out: mpsc::UnboundedSender<ServerMsg>,
}

/// The deck a player brought: one the server offers, or their own list.
enum SeatDeck {
    Offered(&'static DeckList),
    Custom(CustomDeck),
}

impl SeatDeck {
    fn name(&self) -> &str {
        match self {
            SeatDeck::Offered(deck) => deck.name,
            SeatDeck::Custom(deck) => &deck.name,
        }
    }

    fn entries(&self) -> Vec<(&str, u32)> {
        match self {
            SeatDeck::Offered(deck) => deck.cards.to_vec(),
            SeatDeck::Custom(deck) => deck.cards.iter().map(|(k, &n)| (k.as_str(), n)).collect(),
        }
    }
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

/// Reads a card pack from a TOML file. Validation happens when it's added to a
/// [`CardPool`].
pub fn load_pack(path: &Path) -> anyhow::Result<Pack> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The built-in cards plus each pack in `paths`, in order.
pub fn load_pool(paths: &[impl AsRef<Path>]) -> anyhow::Result<CardPool> {
    let mut pool = CardPool::builtin();
    for path in paths {
        let path = path.as_ref();
        pool.add_pack(load_pack(path)?)
            .with_context(|| format!("{} has problems", path.display()))?;
        tracing::info!(pack = %path.display(), "loaded card pack");
    }
    Ok(pool)
}

/// Accepts players and runs their games, offering the decks in `pool`.
pub async fn serve(listener: TcpListener, pool: Arc<CardPool>) -> std::io::Result<()> {
    let lobby: Lobby = Arc::default();
    loop {
        let (stream, addr) = listener.accept().await?;
        let (lobby, pool) = (lobby.clone(), pool.clone());
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, lobby, pool).await {
                tracing::warn!(%addr, "connection error: {e:#}");
            }
        });
    }
}

/// A player-supplied name, trimmed, without control characters, and at most
/// `MAX_NAME_LEN` characters; `default` if nothing is left.
fn clean_name(name: &str, default: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect();
    if name.is_empty() {
        default.to_string()
    } else {
        name
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

async fn handle_connection(
    stream: TcpStream,
    lobby: Lobby,
    pool: Arc<CardPool>,
) -> anyhow::Result<()> {
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
            clean_name(&name, "Anonymous")
        }
        Some(_) => bail!("expected hello"),
        None => return Ok(()),
    };
    let _ = out.send(ServerMsg::Welcome {
        decks: deck_infos(&pool),
    });

    let (room, deck) = loop {
        match next_msg(&mut reader).await? {
            Some(ClientMsg::JoinCustom { room, mut deck }) => {
                let problems =
                    pool.decklist_problems(deck.cards.iter().map(|(k, &n)| (k.as_str(), n)));
                if problems.is_empty() {
                    deck.name = clean_name(&deck.name, "Custom deck");
                    break (room.trim().to_lowercase(), SeatDeck::Custom(deck));
                }
                let _ = out.send(ServerMsg::Error {
                    message: format!("this server can't use your deck: {}", problems.join("; ")),
                });
            }
            Some(ClientMsg::Join { room, deck }) => match pool.deck(&deck) {
                Some(deck) => break (room.trim().to_lowercase(), SeatDeck::Offered(deck)),
                None => {
                    let _ = out.send(ServerMsg::Error {
                        message: format!(
                            "this server has no deck called {deck:?} (to play your own deck, \
                             give the path to its file)"
                        ),
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
    tracing::info!(%name, %room, deck = deck.name(), "player joined");

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
                tokio::spawn(run_game(pool, vec![waiting.seat, me], game_rx));
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

async fn run_game(
    pool: Arc<CardPool>,
    seats: Vec<Seat>,
    mut inputs: mpsc::UnboundedReceiver<(PlayerId, GameInput)>,
) {
    let names: Vec<String> = seats.iter().map(|s| s.name.clone()).collect();
    let decks: Vec<Vec<(&str, u32)>> = seats.iter().map(|s| s.deck.entries()).collect();
    let players: Vec<(String, &[(&str, u32)])> = seats
        .iter()
        .zip(&decks)
        .map(|(s, d)| (s.name.clone(), d.as_slice()))
        .collect();
    let mut game = Game::new(&pool, &players, rand::random());
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../magus-core/tests/fixtures/sample-pack.toml"
    );

    #[test]
    fn loads_the_sample_pack() {
        let pool = load_pool(&[SAMPLE]).unwrap();
        assert!(pool.card("gullwing-courier").is_some());
        assert_eq!(
            deck_infos(&pool).last().unwrap().key,
            "harbor-tides",
            "pack decks are offered to players"
        );
    }

    #[test]
    fn typos_are_reported_not_ignored() {
        let text = std::fs::read_to_string(SAMPLE)
            .unwrap()
            .replace("count = 1 }]", "cuont = 1 }]");
        let err = toml::from_str::<Pack>(&text).unwrap_err().to_string();
        assert!(err.contains("cuont"), "{err}");
    }

    #[test]
    fn invalid_packs_name_the_file_and_every_problem() {
        let dir = std::env::temp_dir().join(format!("magus-pack-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(
            &path,
            r#"
            [[card]]
            key = "lagoon"
            name = "Another Lagoon"
            type = "land"
            mana = "U"

            [[card]]
            key = "glass-golem"
            name = "Glass Golem"
            type = "creature"
            cost = "3"
            power = 2
            "#,
        )
        .unwrap();
        let err = format!("{:#}", load_pool(&[&path]).unwrap_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("bad.toml has problems"), "{err}");
        assert!(
            err.contains(r#"card "lagoon": that key is already taken"#),
            "{err}"
        );
        assert!(
            err.contains(r#"card "glass-golem": a creature needs both power and toughness"#),
            "{err}"
        );
    }

    /// Connects to `addr`, says hello, and sends `join`; returns the reply.
    async fn join_with(addr: &str, join: ClientMsg) -> ServerMsg {
        let (tx, mut rx) = magus_protocol::connect(addr).await.unwrap();
        tx.send(ClientMsg::Hello {
            name: "Tester".into(),
            protocol: PROTOCOL_VERSION,
        })
        .unwrap();
        let Some(ServerMsg::Welcome { .. }) = rx.recv().await else {
            panic!("no welcome");
        };
        tx.send(join).unwrap();
        let reply = rx.recv().await.unwrap();
        drop(tx);
        reply
    }

    #[tokio::test]
    async fn custom_decks_are_checked_against_the_servers_pool() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(serve(listener, Arc::new(CardPool::builtin())));
        let deck = |cards: &[(&str, u32)]| CustomDeck {
            name: "Mine".into(),
            cards: cards.iter().map(|&(k, n)| (k.to_string(), n)).collect(),
        };
        let custom = |cards: &[(&str, u32)]| ClientMsg::JoinCustom {
            room: "r".into(),
            deck: deck(cards),
        };

        let reply = join_with(
            &addr,
            custom(&[("blaze", 5), ("gullwing-courier", 4), ("crag", 51)]),
        )
        .await;
        let ServerMsg::Error { message } = reply else {
            panic!("expected an error, got {reply:?}");
        };
        assert_eq!(
            message,
            "this server can't use your deck: 5 copies of Blaze, but at most 4 are allowed \
             (basic lands are exempt); unknown card \"gullwing-courier\""
        );

        let reply = join_with(&addr, custom(&[("blaze", 4), ("crag", 56)])).await;
        assert_eq!(reply, ServerMsg::Waiting { room: "r".into() });
    }

    #[tokio::test]
    async fn two_custom_decks_start_a_game() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(serve(listener, Arc::new(CardPool::builtin())));
        let mut players = Vec::new();
        for (name, card, land) in [("A", "blaze", "crag"), ("B", "grovekin", "thicket")] {
            let (tx, rx) = magus_protocol::connect(&addr).await.unwrap();
            tx.send(ClientMsg::Hello {
                name: name.into(),
                protocol: PROTOCOL_VERSION,
            })
            .unwrap();
            let cards = [(card.to_string(), 4), (land.to_string(), 56)];
            tx.send(ClientMsg::JoinCustom {
                room: "pair".into(),
                deck: CustomDeck {
                    name: format!("{name}'s deck"),
                    cards: cards.into_iter().collect(),
                },
            })
            .unwrap();
            players.push((tx, rx));
        }
        for (_, rx) in &mut players {
            let mut started = false;
            while let Some(msg) = rx.recv().await {
                match msg {
                    ServerMsg::Started { .. } => started = true,
                    ServerMsg::State { view } => {
                        assert!(started);
                        assert_eq!(view.hand.len() + view.players[view.you].library_size, 60);
                        break;
                    }
                    ServerMsg::Error { message } => panic!("{message}"),
                    _ => {}
                }
            }
        }
    }
}
