//! `magus deck-builder`: pick cards from the pool into a deck and save it as a
//! deck file. Works offline; the pool is the built-in cards plus any packs.
//!
//! `Builder` is the state and input handling; `draw` only reads it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use magus_core::card::{CardDef, CardKind};
use magus_core::mana::Color;
use magus_core::pool::{DECK_SIZE, DeckSpec, MAX_COPIES};
use magus_core::view::CardView;
use magus_core::{CardPool, ObjectId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as Tc, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::deckfile;
use crate::ui::{SELECTED, card_lines, card_name, color_of, mana_text, panel};

/// Opens the deck builder on the deck file at `path` (loading it if it exists)
/// and runs it until the player quits.
pub fn run(path: &Path, packs: &[PathBuf]) -> anyhow::Result<()> {
    let pool = magus_server::load_pool(packs)?;
    let deck = if path.exists() {
        deckfile::load(path)?
    } else {
        let (key, name) = deckfile::names_for(path);
        DeckSpec {
            key,
            name,
            description: String::new(),
            cards: BTreeMap::new(),
        }
    };
    let mut builder = Builder::new(pool, deck, path.to_path_buf());
    let mut terminal = ratatui::init();
    let result = loop {
        if let Err(e) = terminal.draw(|frame| draw(frame, &builder)) {
            break Err(e.into());
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => builder.on_key(key),
            Ok(_) => {}
            Err(e) => break Err(e.into()),
        }
        if builder.quit {
            break Ok(());
        }
    };
    ratatui::restore();
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Catalog,
    Deck,
}

/// Which cards the catalog shows, by color. Lands count as the color of mana
/// they make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFilter {
    All,
    Only(Color),
}

/// Which cards the catalog shows, by card type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Creature,
    Instant,
    Sorcery,
    Land,
}

impl TypeFilter {
    fn next(self) -> TypeFilter {
        match self {
            TypeFilter::All => TypeFilter::Creature,
            TypeFilter::Creature => TypeFilter::Instant,
            TypeFilter::Instant => TypeFilter::Sorcery,
            TypeFilter::Sorcery => TypeFilter::Land,
            TypeFilter::Land => TypeFilter::All,
        }
    }

    fn name(self) -> &'static str {
        match self {
            TypeFilter::All => "all",
            TypeFilter::Creature => "creatures",
            TypeFilter::Instant => "instants",
            TypeFilter::Sorcery => "sorceries",
            TypeFilter::Land => "lands",
        }
    }

    fn matches(self, def: &CardDef) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Creature => def.is_creature(),
            TypeFilter::Instant => matches!(def.kind, CardKind::Instant),
            TypeFilter::Sorcery => matches!(def.kind, CardKind::Sorcery),
            TypeFilter::Land => def.is_land(),
        }
    }
}

/// Typing a new name for the deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renaming(pub String);

pub struct Builder {
    pub pool: CardPool,
    pub deck: DeckSpec,
    pub path: PathBuf,
    pub focus: Focus,
    pub color: ColorFilter,
    pub kind: TypeFilter,
    pub catalog_sel: usize,
    pub deck_sel: usize,
    pub renaming: Option<Renaming>,
    /// Changed since the last save.
    pub dirty: bool,
    pub confirm_quit: bool,
    pub status: Option<String>,
    pub quit: bool,
}

impl Builder {
    pub fn new(pool: CardPool, deck: DeckSpec, path: PathBuf) -> Builder {
        Builder {
            pool,
            deck,
            path,
            focus: Focus::Catalog,
            color: ColorFilter::All,
            kind: TypeFilter::All,
            catalog_sel: 0,
            deck_sel: 0,
            renaming: None,
            dirty: false,
            confirm_quit: false,
            status: None,
            quit: false,
        }
    }

    /// The cards the catalog shows, in display order.
    pub fn catalog(&self) -> Vec<&'static CardDef> {
        let mut cards: Vec<_> = self
            .pool
            .cards()
            .filter(|def| self.kind.matches(def))
            .filter(|def| match self.color {
                ColorFilter::All => true,
                ColorFilter::Only(color) => colors(def).contains(&color),
            })
            .collect();
        cards.sort_by_key(|def| sort_key(def));
        cards
    }

    /// The deck's entries in display order: the card (if the pool has it),
    /// its key, and how many copies.
    pub fn entries(&self) -> Vec<(Option<&'static CardDef>, String, u32)> {
        let mut entries: Vec<_> = self
            .deck
            .cards
            .iter()
            .map(|(key, &n)| (self.pool.card(key), key.clone(), n))
            .collect();
        entries.sort_by_key(|(def, key, _)| (def.map(sort_key), key.clone()));
        entries
    }

    pub fn size(&self) -> u64 {
        self.deck.cards.values().map(|&n| u64::from(n)).sum()
    }

    pub fn problems(&self) -> Vec<String> {
        self.pool
            .decklist_problems(self.deck.cards.iter().map(|(k, &n)| (k.as_str(), n)))
    }

    /// The card under the cursor, for the detail pane.
    pub fn selected(&self) -> Option<&'static CardDef> {
        match self.focus {
            Focus::Catalog => self.catalog().get(self.catalog_sel).copied(),
            Focus::Deck => self.entries().get(self.deck_sel)?.0,
        }
    }

    /// The key of the card under the cursor, even one the pool lacks.
    fn selected_key(&self) -> Option<String> {
        match self.focus {
            Focus::Catalog => Some(self.catalog().get(self.catalog_sel)?.key.to_string()),
            Focus::Deck => Some(self.entries().get(self.deck_sel)?.1.clone()),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if let Some(Renaming(name)) = &mut self.renaming {
            match key.code {
                KeyCode::Enter => {
                    let name = name.trim().to_string();
                    if !name.is_empty() && name != self.deck.name {
                        self.deck.name = name;
                        self.dirty = true;
                    }
                    self.renaming = None;
                }
                KeyCode::Esc => self.renaming = None,
                KeyCode::Backspace => {
                    name.pop();
                }
                KeyCode::Char(c) if !c.is_control() && name.chars().count() < 40 => name.push(c),
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Char('q') {
            if !self.dirty || self.confirm_quit {
                self.quit = true;
            } else {
                self.confirm_quit = true;
                self.status = Some(
                    "Unsaved changes. Press s to save, or q again to quit without saving.".into(),
                );
            }
            return;
        }
        self.confirm_quit = false;
        self.status = None;
        let up = matches!(key.code, KeyCode::Up | KeyCode::Char('k'));
        let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
        if up || down {
            let (sel, len) = match self.focus {
                Focus::Catalog => (self.catalog_sel, self.catalog().len()),
                Focus::Deck => (self.deck_sel, self.entries().len()),
            };
            let sel = match (up, len) {
                (_, 0) => 0,
                (true, _) => (sel + len - 1) % len,
                (false, _) => (sel + 1) % len,
            };
            match self.focus {
                Focus::Catalog => self.catalog_sel = sel,
                Focus::Deck => self.deck_sel = sel,
            }
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Catalog => Focus::Deck,
                    Focus::Deck => Focus::Catalog,
                };
            }
            KeyCode::Enter | KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => {
                if let Some(key) = self.selected_key() {
                    self.add(&key);
                }
            }
            KeyCode::Char('-') | KeyCode::Backspace | KeyCode::Delete | KeyCode::Left => {
                if let Some(key) = self.selected_key() {
                    self.remove(&key);
                }
            }
            KeyCode::Char('c') => {
                self.color = match self.color {
                    ColorFilter::All => ColorFilter::Only(Color::ALL[0]),
                    ColorFilter::Only(c) => match Color::ALL.iter().position(|&x| x == c) {
                        Some(i) if i + 1 < Color::ALL.len() => ColorFilter::Only(Color::ALL[i + 1]),
                        _ => ColorFilter::All,
                    },
                };
                self.catalog_sel = 0;
            }
            KeyCode::Char('t') => {
                self.kind = self.kind.next();
                self.catalog_sel = 0;
            }
            KeyCode::Char('r') => self.renaming = Some(Renaming(self.deck.name.clone())),
            KeyCode::Char('s') => self.save(),
            _ => {}
        }
    }

    /// Adds a copy of `key`, unless that would break the copy limit.
    pub fn add(&mut self, key: &str) {
        let have = self.deck.cards.get(key).copied().unwrap_or(0);
        if let Some(def) = self.pool.card(key)
            && !def.is_land()
            && have >= MAX_COPIES
        {
            self.status = Some(format!(
                "A deck can have at most {MAX_COPIES} copies of {} (basic lands are exempt).",
                def.name
            ));
            return;
        }
        *self.deck.cards.entry(key.to_string()).or_default() += 1;
        self.dirty = true;
    }

    pub fn remove(&mut self, key: &str) {
        let Some(n) = self.deck.cards.get_mut(key) else {
            return;
        };
        *n -= 1;
        if *n == 0 {
            self.deck.cards.remove(key);
            let len = self.entries().len();
            self.deck_sel = self.deck_sel.min(len.saturating_sub(1));
        }
        self.dirty = true;
    }

    /// Saves even an unfinished deck, so it can be finished later.
    pub fn save(&mut self) {
        match deckfile::save(&self.path, &self.deck) {
            Ok(()) => {
                self.dirty = false;
                let unfinished = if self.problems().is_empty() {
                    ""
                } else {
                    " It isn't playable yet: see Problems."
                };
                self.status = Some(format!("Saved {}.{unfinished}", self.path.display()));
            }
            Err(e) => self.status = Some(format!("Couldn't save: {e:#}")),
        }
    }
}

/// A card's colors for filtering and sorting: lands are colorless, but count as
/// the color of mana they make.
fn colors(def: &CardDef) -> Vec<Color> {
    match def.kind {
        CardKind::Land(color) => vec![color],
        _ => def.colors(),
    }
}

/// Spells by color then cost then name, with lands at the end.
fn sort_key(def: &'static CardDef) -> (bool, Vec<Color>, u32, &'static str) {
    (
        def.is_land(),
        colors(def),
        def.mana_cost().mana_value(),
        def.name,
    )
}

fn view_of(def: &CardDef) -> CardView {
    CardView::new(ObjectId(0), def)
}

// ----- drawing -----

pub fn draw(frame: &mut Frame, b: &Builder) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    let [catalog, deck, side] = Layout::horizontal([
        Constraint::Fill(3),
        Constraint::Fill(2),
        Constraint::Length(42),
    ])
    .areas(body);
    let [detail, stats] =
        Layout::vertical([Constraint::Length(12), Constraint::Fill(1)]).areas(side);

    let dirty = if b.dirty { " (unsaved)" } else { "" };
    frame.render_widget(
        Line::from(vec![
            " MAGUS ".bold().black().on_cyan(),
            Span::raw("  Deck builder · "),
            Span::styled(b.deck.name.clone(), Style::new().bold()),
            format!(" · {}{dirty}", b.path.display()).dark_gray(),
        ]),
        header,
    );
    draw_catalog(frame, catalog, b);
    draw_deck(frame, deck, b);
    draw_detail(frame, detail, b);
    draw_stats(frame, stats, b);
    draw_footer(frame, footer, b);
}

fn draw_catalog(frame: &mut Frame, area: Rect, b: &Builder) {
    let items: Vec<ListItem> = b
        .catalog()
        .into_iter()
        .map(|def| {
            let card = view_of(def);
            let n = b.deck.cards.get(def.key).copied().unwrap_or(0);
            let count = if n > 0 {
                format!("{n}× ").green()
            } else {
                "   ".into()
            };
            let mut spans = vec![count, card_name(&card), Span::raw(" ")];
            spans.extend(mana_text(&card.cost, Style::new()));
            spans.push(format!("  {}", card.type_line).dark_gray());
            ListItem::new(Line::from(spans))
        })
        .collect();
    let color = match b.color {
        ColorFilter::All => Span::raw("all"),
        ColorFilter::Only(c) => Span::styled(c.name(), Style::new().fg(color_of(&[c]))),
    };
    let title = Line::from(vec![
        Span::raw(" Cards · color: "),
        color,
        Span::raw(format!(" · type: {} ", b.kind.name())),
    ]);
    let focused = b.focus == Focus::Catalog;
    let mut state = ListState::default().with_selected(focused.then_some(b.catalog_sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(title, focused))
            .highlight_style(SELECTED),
        area,
        &mut state,
    );
}

fn draw_deck(frame: &mut Frame, area: Rect, b: &Builder) {
    let items: Vec<ListItem> = b
        .entries()
        .into_iter()
        .map(|(def, key, n)| match def {
            Some(def) => {
                let card = view_of(def);
                let mut spans = vec![
                    Span::raw(format!("{n}× ")),
                    card_name(&card),
                    Span::raw(" "),
                ];
                spans.extend(mana_text(&card.cost, Style::new()));
                ListItem::new(Line::from(spans))
            }
            None => ListItem::new(Line::from(format!("{n}× {key} (not in this card pool)")).red()),
        })
        .collect();
    let size = b.size();
    let count = format!(" Deck · {size}/{DECK_SIZE} ");
    let title = if size == u64::from(DECK_SIZE) {
        Line::from(count).green()
    } else {
        Line::from(count)
    };
    let focused = b.focus == Focus::Deck;
    let mut state = ListState::default().with_selected(focused.then_some(b.deck_sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(title, focused))
            .highlight_style(SELECTED),
        area,
        &mut state,
    );
}

fn draw_detail(frame: &mut Frame, area: Rect, b: &Builder) {
    let block = panel(" Card ", false);
    let Some(def) = b.selected() else {
        frame.render_widget(
            Paragraph::new("Nothing selected.").dark_gray().block(block),
            area,
        );
        return;
    };
    let card = view_of(def);
    let mut lines = card_lines(&card);
    if let (Some(p), Some(t)) = (card.power, card.toughness) {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("{p}/{t}")).bold());
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn draw_stats(frame: &mut Frame, area: Rect, b: &Builder) {
    let entries = b.entries();
    let mut lines = Vec::new();

    // Mana curve: how many non-land cards cost 0, 1, 2, … 6+ mana.
    let mut curve = [0u32; 7];
    let (mut lands, mut spells) = (0u32, 0u32);
    let mut by_color: BTreeMap<Color, u32> = BTreeMap::new();
    for (def, _, n) in &entries {
        let Some(def) = def else { continue };
        if def.is_land() {
            lands += n;
            continue;
        }
        spells += n;
        let value = def.mana_cost().mana_value().min(6) as usize;
        curve[value] += n;
        for color in def.colors() {
            *by_color.entry(color).or_default() += n;
        }
    }
    lines.push(Line::from(format!("{spells} spells · {lands} lands")));
    lines.push(Line::from(""));
    lines.push(Line::from("Mana curve").bold());
    for (cost, n) in curve.iter().enumerate() {
        let label = if cost == 6 {
            "6+".to_string()
        } else {
            format!("{cost} ")
        };
        let bar = "█".repeat((*n as usize).min(30));
        lines.push(Line::from(vec![
            Span::raw(format!("{label} ")),
            Span::styled(bar, Style::new().fg(Tc::Cyan)),
            format!(" {n}").dark_gray(),
        ]));
    }
    if !by_color.is_empty() {
        lines.push(Line::from(""));
        let mut spans = vec![Span::raw("Colors: ")];
        for (color, n) in by_color {
            spans.push(Span::styled(
                format!("{} {n}  ", color.name()),
                Style::new().fg(color_of(&[color])),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    let problems = b.problems();
    if problems.is_empty() {
        lines.push(Line::from("Ready to play.").green().bold());
    } else {
        lines.push(Line::from("Problems").red().bold());
        lines.extend(
            problems
                .into_iter()
                .map(|p| Line::from(format!("• {p}")).red()),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Stats ", false)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, b: &Builder) {
    let keys = match &b.renaming {
        Some(Renaming(name)) => Line::from(vec![
            Span::raw("Deck name: "),
            Span::styled(format!("{name}▏"), Style::new().bold()),
            "   Enter: done · Esc: cancel".dark_gray(),
        ]),
        None => Line::from(
            "↑↓: select · Tab: cards/deck · Enter/+: add · -: remove · c: color · t: type · \
             r: rename · s: save · q: quit",
        )
        .dark_gray(),
    };
    let status = match &b.status {
        Some(s) => Line::from(s.clone()).yellow(),
        None => Line::default(),
    };
    frame.render_widget(
        Paragraph::new(vec![status, keys]).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn builder() -> Builder {
        let deck = DeckSpec {
            key: "test".into(),
            name: "Test".into(),
            description: String::new(),
            cards: BTreeMap::new(),
        };
        let path = std::env::temp_dir().join("magus-builder-test-unused.toml");
        Builder::new(CardPool::builtin(), deck, path)
    }

    fn screen(b: &Builder) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|frame| draw(frame, b)).unwrap();
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

    #[test]
    fn adds_cards_up_to_the_copy_limit() {
        let mut b = builder();
        for _ in 0..5 {
            b.add("blaze");
        }
        assert_eq!(b.deck.cards["blaze"], 4);
        assert!(
            b.status
                .as_deref()
                .unwrap()
                .contains("at most 4 copies of Blaze")
        );
        for _ in 0..20 {
            b.add("crag");
        }
        assert_eq!(b.deck.cards["crag"], 20, "basic lands have no limit");
        b.remove("blaze");
        assert_eq!(b.deck.cards["blaze"], 3);
        assert!(b.dirty);
    }

    #[test]
    fn filters_narrow_the_catalog() {
        let mut b = builder();
        let all = b.catalog().len();
        b.on_key(key(KeyCode::Char('t'))); // creatures
        assert!(b.catalog().iter().all(|d| d.is_creature()));
        b.on_key(key(KeyCode::Char('c'))); // white
        let white = b.catalog();
        assert!(!white.is_empty() && white.len() < all);
        assert!(white.iter().all(|d| d.colors() == [Color::White]));
    }

    #[test]
    fn renders_a_deck_in_progress() {
        let mut b = builder();
        for _ in 0..4 {
            b.add("blaze");
            b.add("cinderhound");
        }
        for _ in 0..24 {
            b.add("crag");
        }
        let shown = screen(&b);
        println!("{shown}");
        assert!(shown.contains("Deck · 32/60"));
        assert!(shown.contains("4× Blaze"));
        assert!(shown.contains("has 32 cards, but decks must have"));
        assert!(shown.contains("Mana curve"));
    }

    #[test]
    fn renaming_and_quitting_with_unsaved_changes() {
        let mut b = builder();
        b.on_key(key(KeyCode::Char('r')));
        for _ in 0..4 {
            b.on_key(key(KeyCode::Backspace));
        }
        for c in "Burn".chars() {
            b.on_key(key(KeyCode::Char(c)));
        }
        b.on_key(key(KeyCode::Enter));
        assert_eq!(b.deck.name, "Burn");
        b.on_key(key(KeyCode::Char('q')));
        assert!(!b.quit, "asks first: there are unsaved changes");
        b.on_key(key(KeyCode::Char('q')));
        assert!(b.quit);
    }
}
