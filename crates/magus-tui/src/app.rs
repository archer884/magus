//! Client state and input handling, independent of how it's drawn.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use magus_core::view::{AttackOption, BlockOption, CardView, PlayKind, Prompt};
use magus_core::{Action, Attack, Block, GameView, ObjectId, PlayerId, Step, Target};
use magus_protocol::{ClientMsg, CustomDeck, DeckInfo, ServerMsg};
use tokio::sync::mpsc::UnboundedSender;

pub enum Screen {
    Connecting,
    PickDeck,
    Waiting { room: String },
    Game,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Hand,
    Board,
    Stack,
}

/// A multi-step choice in progress.
pub enum Mode {
    Idle,
    Target {
        card: ObjectId,
        targets: Vec<Target>,
        sel: usize,
    },
    Attack {
        options: Vec<AttackOption>,
        chosen: Vec<Option<usize>>,
        sel: usize,
    },
    Block {
        options: Vec<BlockOption>,
        chosen: Vec<Option<usize>>,
        sel: usize,
    },
    Discard {
        count: usize,
        marked: HashSet<ObjectId>,
    },
    /// Browsing every graveyard, to read cards or cast one with flashback.
    Graveyard {
        sel: usize,
    },
    /// Answering a [`Prompt::ChooseCard`]. When `optional`, the entry after
    /// the last option means "nothing".
    Choose {
        reason: String,
        options: Vec<ObjectId>,
        optional: bool,
        sel: usize,
    },
}

/// The deck to join with, if chosen before connecting.
#[derive(Debug, Clone)]
pub enum DeckChoice {
    /// One the server offers, by key.
    Offered(String),
    /// The player's own decklist (from a deck file given as `--deck`).
    Custom(CustomDeck),
}

pub struct App {
    pub room: String,
    pub deck: Option<DeckChoice>,
    pub screen: Screen,
    pub decks: Vec<DeckInfo>,
    pub deck_sel: usize,
    pub view: Option<GameView>,
    pub mode: Mode,
    pub focus: Focus,
    pub hand_sel: usize,
    pub board_sel: usize,
    pub stack_sel: usize,
    /// When off, priority is passed automatically at steps where you rarely
    /// want to act. When on, you're asked every time you could act.
    pub full_control: bool,
    pub status: Option<String>,
    pub confirm_quit: bool,
    pub disconnected: bool,
    pub quit: bool,
    tx: UnboundedSender<ClientMsg>,
}

impl App {
    pub fn new(tx: UnboundedSender<ClientMsg>, room: String, deck: Option<DeckChoice>) -> App {
        App {
            room,
            deck,
            screen: Screen::Connecting,
            decks: Vec::new(),
            deck_sel: 0,
            view: None,
            mode: Mode::Idle,
            focus: Focus::Hand,
            hand_sel: 0,
            board_sel: 0,
            stack_sel: 0,
            full_control: false,
            status: None,
            confirm_quit: false,
            disconnected: false,
            quit: false,
            tx,
        }
    }

    fn send(&mut self, msg: ClientMsg) {
        if self.tx.send(msg).is_err() {
            self.disconnected = true;
        }
    }

    fn act(&mut self, action: Action) {
        if let Some(version) = self.view.as_ref().map(|v| v.version) {
            self.status = None;
            self.mode = Mode::Idle;
            self.send(ClientMsg::Act { version, action });
        }
    }

    fn join(&mut self, deck: String) {
        let room = self.room.clone();
        self.send(ClientMsg::Join { room, deck });
    }

    pub fn on_server(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome { decks } => {
                self.decks = decks;
                match self.deck.clone() {
                    Some(DeckChoice::Offered(deck)) => self.join(deck),
                    Some(DeckChoice::Custom(deck)) => {
                        let room = self.room.clone();
                        self.send(ClientMsg::JoinCustom { room, deck });
                    }
                    None => self.screen = Screen::PickDeck,
                }
            }
            ServerMsg::Waiting { room } => self.screen = Screen::Waiting { room },
            ServerMsg::Started { .. } => self.screen = Screen::Game,
            ServerMsg::State { view } => self.set_view(*view),
            ServerMsg::Error { message } => {
                if matches!(self.screen, Screen::Connecting) {
                    self.screen = Screen::PickDeck;
                }
                self.status = Some(message);
            }
        }
    }

    fn set_view(&mut self, view: GameView) {
        self.mode = match &view.prompt {
            Prompt::DeclareAttackers { options } => Mode::Attack {
                chosen: vec![None; options.len()],
                options: options.clone(),
                sel: 0,
            },
            Prompt::DeclareBlockers { options } => Mode::Block {
                chosen: vec![None; options.len()],
                options: options.clone(),
                sel: 0,
            },
            Prompt::ChooseCard {
                reason,
                options,
                optional,
            } => Mode::Choose {
                reason: reason.clone(),
                options: options.clone(),
                optional: *optional,
                sel: 0,
            },
            Prompt::Discard { count } => {
                self.focus = Focus::Hand;
                Mode::Discard {
                    count: *count,
                    marked: HashSet::new(),
                }
            }
            _ => Mode::Idle,
        };
        self.hand_sel = self.hand_sel.min(view.hand.len().saturating_sub(1));
        self.stack_sel = self.stack_sel.min(view.stack.len().saturating_sub(1));
        self.view = Some(view);
        self.board_sel = self
            .board_sel
            .min(self.board_order().len().saturating_sub(1));
        self.maybe_auto_pass();
    }

    /// Whether this is a moment you'd usually want to stop and think.
    fn wants_stop(view: &GameView) -> bool {
        if let Some(top) = view.stack.last() {
            return top.controller != view.you;
        }
        let mine = view.active == view.you;
        match view.step {
            Step::Main1 | Step::Main2 => mine,
            Step::DeclareAttackers | Step::End => !mine,
            Step::DeclareBlockers => true,
            _ => false,
        }
    }

    fn maybe_auto_pass(&mut self) {
        let Some(view) = &self.view else { return };
        if !self.full_control
            && matches!(view.prompt, Prompt::Priority { .. })
            && !Self::wants_stop(view)
        {
            self.act(Action::Pass);
        }
    }

    /// Creatures in display order: opponents first, then yours.
    pub fn board_order(&self) -> Vec<ObjectId> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        let creatures = |mine: bool| {
            view.battlefield
                .iter()
                .filter(move |p| !p.card.is_land && (p.controller == view.you) == mine)
                .map(|p| p.card.id)
        };
        creatures(false).chain(creatures(true)).collect()
    }

    /// The card the cursor is on, for the detail pane.
    pub fn selected_card(&self) -> Option<CardView> {
        let view = self.view.as_ref()?;
        let id = match &self.mode {
            Mode::Target { targets, sel, .. } => targets.get(*sel)?.object()?,
            Mode::Attack { options, sel, .. } => options.get(*sel)?.attacker,
            Mode::Block {
                options,
                sel,
                chosen,
            } => {
                // Show the attacker being blocked if one is chosen, else the blocker.
                let option = options.get(*sel)?;
                match chosen[*sel] {
                    Some(i) => option.attackers[i],
                    None => option.blocker,
                }
            }
            Mode::Discard { .. } => view.hand.get(self.hand_sel)?.id,
            Mode::Graveyard { sel } => graveyard_entries(view).get(*sel)?.1.id,
            Mode::Choose { options, sel, .. } => *options.get(*sel)?,
            Mode::Idle => match self.focus {
                Focus::Hand => view.hand.get(self.hand_sel)?.id,
                Focus::Board => *self.board_order().get(self.board_sel)?,
                Focus::Stack => view.stack.iter().rev().nth(self.stack_sel)?.card.id,
            },
        };
        view.hand_card(id)
            .or_else(|| view.permanent(id).map(|p| &p.card))
            .or_else(|| {
                view.stack
                    .iter()
                    .find(|s| s.id == id || s.card.id == id)
                    .map(|s| &s.card)
            })
            .or_else(|| view.graveyard_card(id).map(|(_, card)| card))
            .cloned()
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.screen {
            Screen::Connecting | Screen::Waiting { .. } => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    self.quit = true;
                }
            }
            Screen::PickDeck => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.deck_sel = self.deck_sel.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.deck_sel = (self.deck_sel + 1).min(self.decks.len().saturating_sub(1))
                }
                KeyCode::Enter => {
                    if let Some(deck) = self.decks.get(self.deck_sel) {
                        let key = deck.key.clone();
                        self.status = None;
                        self.join(key);
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
                _ => {}
            },
            Screen::Game => self.on_game_key(key),
        }
    }

    fn on_game_key(&mut self, key: KeyEvent) {
        let game_over = self
            .view
            .as_ref()
            .is_some_and(|v| matches!(v.prompt, Prompt::GameOver { .. }));
        if key.code == KeyCode::Char('q') {
            if game_over || self.disconnected || self.confirm_quit {
                self.quit = true;
            } else {
                self.confirm_quit = true;
                self.status = Some("Press q again to leave the game (you'll concede).".into());
            }
            return;
        }
        if self.confirm_quit {
            self.confirm_quit = false;
            self.status = None;
        }
        if key.code == KeyCode::Char('f') {
            self.full_control = !self.full_control;
            let state = if self.full_control {
                "on: you'll be asked at every step"
            } else {
                "off"
            };
            self.status = Some(format!("Full control {state}."));
            self.maybe_auto_pass();
            return;
        }
        let up = matches!(key.code, KeyCode::Up | KeyCode::Char('k'));
        let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
        let Some(view) = self.view.clone() else {
            return;
        };
        match &mut self.mode {
            Mode::Idle => {
                if up || down {
                    let board_len = self.board_order().len();
                    let (sel, len) = match self.focus {
                        Focus::Hand => (&mut self.hand_sel, view.hand.len()),
                        Focus::Board => (&mut self.board_sel, board_len),
                        Focus::Stack => (&mut self.stack_sel, view.stack.len()),
                    };
                    step(sel, len, up);
                    return;
                }
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab => {
                        let order = [Focus::Hand, Focus::Board, Focus::Stack];
                        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                        let i = if key.code == KeyCode::Tab {
                            i + 1
                        } else {
                            i + 2
                        };
                        self.focus = order[i % 3];
                    }
                    KeyCode::Char(' ') | KeyCode::Char('p') => {
                        if matches!(view.prompt, Prompt::Priority { .. }) {
                            self.act(Action::Pass);
                        }
                    }
                    KeyCode::Enter => self.play_selected(&view),
                    KeyCode::Char('g') => {
                        if graveyard_entries(&view).is_empty() {
                            self.status = Some("All graveyards are empty.".into());
                        } else {
                            self.mode = Mode::Graveyard { sel: 0 };
                        }
                    }
                    _ => {}
                }
            }
            Mode::Choose {
                options,
                optional,
                sel,
                ..
            } => match key.code {
                _ if up || down => step(sel, options.len() + usize::from(*optional), up),
                KeyCode::Enter => {
                    let card = options.get(*sel).copied();
                    self.act(Action::ChooseCard { card });
                }
                KeyCode::Esc if *optional => self.act(Action::ChooseCard { card: None }),
                _ => {}
            },
            Mode::Graveyard { sel } => match key.code {
                _ if up || down => step(sel, graveyard_entries(&view).len(), up),
                KeyCode::Enter => {
                    let Some((owner, card)) = graveyard_entries(&view).get(*sel).cloned() else {
                        return;
                    };
                    if owner == view.you && self.is_playable(card.id) {
                        self.mode = Mode::Idle;
                        self.play_card(&view, &card);
                    } else {
                        self.status = Some(format!("You can't cast {} right now.", card.name));
                    }
                }
                KeyCode::Esc | KeyCode::Char('g') => self.mode = Mode::Idle,
                _ => {}
            },
            Mode::Target { card, targets, sel } => match key.code {
                _ if up || down => step(sel, targets.len(), up),
                KeyCode::Enter => {
                    let action = Action::Cast {
                        card: *card,
                        target: targets.get(*sel).copied(),
                    };
                    self.act(action);
                }
                KeyCode::Esc => self.mode = Mode::Idle,
                _ => {}
            },
            Mode::Attack {
                options,
                chosen,
                sel,
            } => match key.code {
                _ if up || down => step(sel, options.len(), up),
                KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                    cycle(&mut chosen[*sel], options[*sel].defenders.len(), true)
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    cycle(&mut chosen[*sel], options[*sel].defenders.len(), false)
                }
                KeyCode::Char('a') => {
                    let all = chosen.iter().all(Option::is_some);
                    chosen
                        .iter_mut()
                        .for_each(|c| *c = if all { None } else { Some(0) });
                }
                KeyCode::Enter => {
                    let attacks = options
                        .iter()
                        .zip(chosen.iter())
                        .filter_map(|(o, c)| {
                            c.map(|i| Attack {
                                attacker: o.attacker,
                                defender: o.defenders[i],
                            })
                        })
                        .collect();
                    self.act(Action::DeclareAttackers { attacks });
                }
                _ => {}
            },
            Mode::Block {
                options,
                chosen,
                sel,
            } => match key.code {
                _ if up || down => step(sel, options.len(), up),
                KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                    cycle(&mut chosen[*sel], options[*sel].attackers.len(), true)
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    cycle(&mut chosen[*sel], options[*sel].attackers.len(), false)
                }
                KeyCode::Enter => {
                    let blocks = options
                        .iter()
                        .zip(chosen.iter())
                        .filter_map(|(o, c)| {
                            c.map(|i| Block {
                                blocker: o.blocker,
                                attacker: o.attackers[i],
                            })
                        })
                        .collect();
                    self.act(Action::DeclareBlockers { blocks });
                }
                _ => {}
            },
            Mode::Discard { count, marked } => match key.code {
                _ if up || down => step(&mut self.hand_sel, view.hand.len(), up),
                KeyCode::Char(' ') => {
                    if let Some(card) = view.hand.get(self.hand_sel)
                        && !marked.remove(&card.id)
                        && marked.len() < *count
                    {
                        marked.insert(card.id);
                    }
                }
                KeyCode::Enter => {
                    if marked.len() == *count {
                        let cards = marked.iter().copied().collect();
                        self.act(Action::Discard { cards });
                    } else {
                        self.status = Some(format!("Mark exactly {count} card(s) to discard."));
                    }
                }
                _ => {}
            },
        }
    }

    fn play_selected(&mut self, view: &GameView) {
        if !matches!(view.prompt, Prompt::Priority { .. }) {
            self.status = Some("It isn't your priority.".into());
            return;
        }
        if self.focus != Focus::Hand {
            self.status = Some("Select a card in your hand (Tab) to play it.".into());
            return;
        }
        let Some(card) = view.hand.get(self.hand_sel) else {
            return;
        };
        self.play_card(view, card);
    }

    /// Plays `card` (from hand, or from the graveyard with flashback), asking for
    /// a target first if it needs one.
    fn play_card(&mut self, view: &GameView, card: &CardView) {
        let Prompt::Priority { plays } = &view.prompt else {
            self.status = Some("It isn't your priority.".into());
            return;
        };
        let Some(play) = plays.iter().find(|p| p.card == card.id) else {
            self.status = Some(format!("You can't play {} right now.", card.name));
            return;
        };
        match (&play.kind, &play.targets) {
            (PlayKind::Land, _) => self.act(Action::PlayLand { card: card.id }),
            (PlayKind::Spell, None) => self.act(Action::Cast {
                card: card.id,
                target: None,
            }),
            (PlayKind::Spell, Some(targets)) => {
                self.mode = Mode::Target {
                    card: card.id,
                    targets: targets.clone(),
                    sel: 0,
                };
            }
        }
    }

    pub fn is_playable(&self, id: ObjectId) -> bool {
        matches!(&self.view, Some(GameView { prompt: Prompt::Priority { plays }, .. }) if plays.iter().any(|p| p.card == id))
    }
}

/// Every card in every graveyard, yours first, with its owner.
pub fn graveyard_entries(view: &GameView) -> Vec<(PlayerId, CardView)> {
    let mut players: Vec<_> = view.players.iter().collect();
    players.sort_by_key(|p| p.id != view.you);
    players
        .into_iter()
        .flat_map(|p| p.graveyard.iter().map(move |c| (p.id, c.clone())))
        .collect()
}

fn step(sel: &mut usize, len: usize, up: bool) {
    if len == 0 {
        *sel = 0;
    } else if up {
        *sel = sel.saturating_sub(1);
    } else {
        *sel = (*sel + 1).min(len - 1);
    }
}

/// Cycles a choice through None → 0 → 1 → … → None.
fn cycle(choice: &mut Option<usize>, len: usize, forward: bool) {
    *choice = match (*choice, forward) {
        (None, true) => Some(0),
        (None, false) => len.checked_sub(1),
        (Some(i), true) if i + 1 < len => Some(i + 1),
        (Some(_), true) => None,
        (Some(0), false) => None,
        (Some(i), false) => Some(i - 1),
    };
}
