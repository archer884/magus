mod app;
mod builder;
mod deckfile;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use magus_core::CardPool;
use magus_protocol::{ClientMsg, DEFAULT_PORT, PROTOCOL_VERSION};
use rand::seq::SliceRandom;
use tokio::net::TcpListener;

use crate::app::{App, DeckChoice};

/// Play Magus in your terminal.
#[derive(Parser)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    play: Args,
}

#[derive(Subcommand)]
enum Command {
    /// Build a new deck, or edit one you made before, and save it to a file.
    DeckBuilder {
        /// The deck file: opened for editing if it exists, created otherwise.
        #[arg(long, short)]
        deck: PathBuf,
        /// A card pack (TOML) whose cards to offer too. Repeat for several.
        #[arg(long = "cards", value_name = "PATH")]
        packs: Vec<PathBuf>,
    },
}

/// Options for playing (the default when no subcommand is given).
#[derive(clap::Args)]
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
    /// Deck to play: the name of one the server offers, or the path to your
    /// own deck file (from `magus deck-builder`), whose cards the server must
    /// have. If omitted you'll pick one from a list.
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

/// What `--deck` means: a deck file if it names one, otherwise the key of a
/// deck the server offers.
fn deck_choice(arg: &str) -> anyhow::Result<DeckChoice> {
    let path = Path::new(arg);
    if path.is_file() {
        return Ok(DeckChoice::Custom(deckfile::to_custom(deckfile::load(
            path,
        )?)));
    }
    // Deck keys never look like paths, so this was meant to be a file.
    if arg.ends_with(".toml") || arg.contains(std::path::MAIN_SEPARATOR) {
        anyhow::bail!("no deck file at {arg}");
    }
    Ok(DeckChoice::Offered(arg.to_string()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::DeckBuilder { deck, packs }) => builder::run(&deck, &packs),
        None => play(cli.play).await,
    }
}

async fn play(mut args: Args) -> anyhow::Result<()> {
    // Read a deck file first, so a bad one fails before anything starts.
    let deck = args.deck.take().map(|d| deck_choice(&d)).transpose()?;
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

    let mut app = App::new(tx, args.room, deck);
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
    use magus_core::view::Prompt;
    use magus_core::{Action, CardPool, Game, GameView};
    use magus_protocol::{ClientMsg, ServerMsg};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::{App, DeckChoice};

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
            ("Ann".to_string(), pool.decks()[0].cards),
            ("Bob".to_string(), pool.decks()[1].cards),
        ];
        let mut game = Game::new(&pool, &seats, 3);
        let me = game.waiting_on().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            tx,
            "practice".into(),
            Some(DeckChoice::Offered("ember-thorn".into())),
        );
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

    /// A game in progress, seen by the player whose move it is, plus an app
    /// showing it and the channel the app sends actions on.
    fn started() -> (
        GameView,
        App,
        tokio::sync::mpsc::UnboundedReceiver<ClientMsg>,
    ) {
        let pool = CardPool::builtin();
        let seats = [
            ("Ann".to_string(), pool.decks()[0].cards),
            ("Bob".to_string(), pool.decks()[1].cards),
        ];
        let game = Game::new(&pool, &seats, 3);
        let me = game.waiting_on().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            tx,
            "practice".into(),
            Some(DeckChoice::Offered("ember-thorn".into())),
        );
        app.on_server(ServerMsg::Started {
            seat: me,
            players: vec!["Ann".into(), "Bob".into()],
        });
        (game.view(me), app, rx)
    }

    #[test]
    fn costs_show_colored_mana_symbols() {
        let (view, mut app, _rx) = started();
        app.on_server(ServerMsg::State {
            view: Box::new(view),
        });
        let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Ember & Thorn is red and green: some red or green symbol is colored.
        let colored = buffer.content().iter().any(|cell| {
            matches!(
                (cell.symbol(), cell.fg),
                ("R", ratatui::style::Color::LightRed) | ("G", ratatui::style::Color::LightGreen)
            )
        });
        assert!(colored);
    }

    #[test]
    fn choose_card_popup() {
        let (mut view, mut app, mut rx) = started();
        let card = view.hand[0].clone();
        view.prompt = Prompt::ChooseCard {
            reason: "Beckon the Wild: you may put a creature card from your hand onto the \
                     battlefield"
                .into(),
            options: vec![card.id],
            optional: true,
        };
        app.on_server(ServerMsg::State {
            view: Box::new(view),
        });
        let shown = screen(&app);
        println!("{shown}");
        assert!(shown.contains("Choose a card"));
        assert!(shown.contains("Beckon the Wild: you may put"));
        assert!(shown.contains(&card.name));
        assert!(shown.contains("(nothing)"));
        app.on_key(key(KeyCode::Esc));
        let Ok(ClientMsg::Act { action, .. }) = rx.try_recv() else {
            panic!("no action sent")
        };
        assert_eq!(action, Action::ChooseCard { card: None });
    }

    #[test]
    fn graveyard_popup_lists_cards() {
        let (mut view, mut app, _rx) = started();
        let card = view.hand[0].clone();
        let me = view.you;
        view.players[me].graveyard.push(card.clone());
        app.on_server(ServerMsg::State {
            view: Box::new(view),
        });
        app.on_key(key(KeyCode::Char('g')));
        let shown = screen(&app);
        println!("{shown}");
        assert!(shown.contains("Graveyards"));
        assert!(shown.contains(&card.name));
        assert!(shown.contains("yours"));
        app.on_key(key(KeyCode::Esc));
        assert!(!screen(&app).contains("Graveyards ("));
    }

    #[test]
    fn deck_is_a_file_or_a_server_deck() {
        let dir = std::env::temp_dir().join(format!("magus-deck-arg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("burn.toml");
        std::fs::write(
            &path,
            "[[deck]]\nkey = \"burn\"\nname = \"Burn\"\n[deck.cards]\nblaze = 4\ncrag = 56\n",
        )
        .unwrap();
        let file = crate::deck_choice(path.to_str().unwrap()).unwrap();
        assert!(matches!(file, DeckChoice::Custom(deck) if deck.name == "Burn"));
        let offered = crate::deck_choice("ember-thorn").unwrap();
        assert!(matches!(offered, DeckChoice::Offered(key) if key == "ember-thorn"));
        let missing = crate::deck_choice("nope.toml").unwrap_err().to_string();
        assert_eq!(missing, "no deck file at nope.toml");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
