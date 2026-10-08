mod app;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use magus_core::CardPool;
use magus_protocol::{ClientMsg, DEFAULT_PORT, PROTOCOL_VERSION};
use rand::seq::SliceRandom;
use tokio::net::TcpListener;

use crate::app::App;

/// Play Magus in your terminal.
#[derive(Parser)]
struct Args {
    /// Server address (host:port).
    #[arg(long, short, default_value_t = format!("127.0.0.1:{DEFAULT_PORT}"))]
    server: String,
    /// Your name at the table.
    #[arg(long, short, env = "USER", default_value = "Player")]
    name: String,
    /// Room to join. You'll be paired with the next person who joins the same room.
    #[arg(long, short, default_value = "practice")]
    room: String,
    /// Deck to play; if omitted you'll pick one from a list.
    #[arg(long, short)]
    deck: Option<String>,
    /// Play against the computer: starts a private server and a bot inside
    /// this process, so no separate server is needed. Ignores --server/--room.
    #[arg(long)]
    solo: bool,
    /// The bot's deck in solo mode (default: a random deck).
    #[arg(long, requires = "solo")]
    bot_deck: Option<String>,
    /// Pause before each bot move in solo mode, in milliseconds.
    #[arg(long, default_value_t = 600, requires = "solo")]
    bot_delay_ms: u64,
    /// In solo mode, a card pack (TOML) to add to the built-in cards and
    /// decks. Repeat to load several.
    #[arg(long = "cards", value_name = "PATH", requires = "solo")]
    packs: Vec<PathBuf>,
}

/// Starts a server on a random local port with a bot waiting in `room`, and
/// returns the server's address.
async fn start_solo(
    pool: CardPool,
    room: &str,
    bot_deck: Option<String>,
    delay: Duration,
) -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?.to_string();
    let deck = match bot_deck {
        Some(deck) => deck,
        None => pool
            .decks()
            .choose(&mut rand::thread_rng())
            .expect("there are decks")
            .key
            .to_string(),
    };
    tokio::spawn(magus_server::serve(listener, Arc::new(pool)));
    let (bot_addr, room) = (addr.clone(), room.to_string());
    tokio::spawn(async move { magus_bot::run(&bot_addr, "Bot", &room, &deck, delay).await });
    Ok(addr)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = Args::parse();
    if args.solo {
        let pool = magus_server::load_pool(&args.packs)?;
        if let Some(deck) = args.bot_deck.as_deref().filter(|d| pool.deck(d).is_none()) {
            anyhow::bail!("unknown bot deck {deck:?}");
        }
        args.room = "solo".into();
        let delay = Duration::from_millis(args.bot_delay_ms);
        args.server = start_solo(pool, &args.room, args.bot_deck.take(), delay).await?;
    }
    let (tx, mut rx) = magus_protocol::connect(&args.server)
        .await
        .map_err(|e| anyhow::anyhow!("couldn't connect to {}: {e}", args.server))?;
    tx.send(ClientMsg::Hello {
        name: args.name.clone(),
        protocol: PROTOCOL_VERSION,
    })?;

    let mut app = App::new(tx, args.room, args.deck);
    let mut terminal = ratatui::init();
    let mut events = EventStream::new();
    let result = loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &app)) {
            break Err(e.into());
        }
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => app.on_key(key),
                Some(Ok(_)) => {}
                Some(Err(e)) => break Err(e.into()),
                None => break Ok(()),
            },
            msg = rx.recv(), if !app.disconnected => match msg {
                Some(msg) => app.on_server(msg),
                None => app.disconnected = true,
            },
        }
        if app.quit {
            break Ok(());
        }
    };
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use magus_core::{CardPool, Game};
    use magus_protocol::{ClientMsg, ServerMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::App;

    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn solo_mode_starts_a_game_against_the_bot() {
        let addr = crate::start_solo(
            CardPool::builtin(),
            "solo",
            Some("tide-ash".into()),
            std::time::Duration::ZERO,
        )
        .await
        .unwrap();
        let (tx, mut rx) = magus_protocol::connect(&addr).await.unwrap();
        tx.send(ClientMsg::Hello {
            name: "Me".into(),
            protocol: magus_protocol::PROTOCOL_VERSION,
        })
        .unwrap();
        let mut started = false;
        while let Some(msg) = rx.recv().await {
            match msg {
                ServerMsg::Welcome { .. } => tx
                    .send(ClientMsg::Join {
                        room: "solo".into(),
                        deck: "ember-thorn".into(),
                    })
                    .unwrap(),
                ServerMsg::Started { players, .. } => {
                    assert!(players.contains(&"Bot".to_string()));
                    started = true;
                }
                ServerMsg::State { .. } => break,
                ServerMsg::Waiting { .. } | ServerMsg::Error { .. } => {}
            }
        }
        assert!(started);
    }

    #[test]
    fn renders_a_game_and_plays_a_land() {
        let pool = CardPool::builtin();
        let seats = [
            ("Ann".to_string(), pool.decks()[0]),
            ("Bob".to_string(), pool.decks()[1]),
        ];
        let mut game = Game::new(&pool, &seats, 3);
        let me = game.waiting_on().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(tx, "practice".into(), Some("ember-thorn".into()));
        app.on_server(ServerMsg::Started {
            seat: me,
            players: vec!["Ann".into(), "Bob".into()],
        });
        app.on_server(ServerMsg::State {
            view: Box::new(game.view(me)),
        });
        let before = screen(&app);
        println!("{before}");
        assert!(before.contains("Your hand ("));
        assert!(before.contains("Your move"));

        // Find a land in hand, select it, and play it.
        let view = game.view(me);
        let land = view
            .hand
            .iter()
            .position(|c| c.is_land)
            .expect("opening hand has a land");
        for _ in 0..land {
            app.on_key(key(KeyCode::Down));
        }
        app.on_key(key(KeyCode::Enter));
        let Ok(ClientMsg::Act { version, action }) = rx.try_recv() else {
            panic!("no action sent")
        };
        game.apply(me, action).unwrap();
        assert_eq!(version + 1, game.version());
        app.on_server(ServerMsg::State {
            view: Box::new(game.view(me)),
        });
        let after = screen(&app);
        println!("{after}");
        assert!(after.contains("Lands: 1"));
    }
}
