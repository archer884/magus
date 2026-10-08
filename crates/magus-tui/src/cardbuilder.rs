//! `magus card-builder`: create and edit the cards in a card pack (TOML).
//!
//! Cards are edited as `CardSpec`s, exactly what the pack file holds. The
//! effect editor works on an effect's serialized form, so it offers every kind
//! of effect and field the engine knows (`Effect::examples`) without a form
//! written per effect.
//!
//! `CardBuilder` is the state and input handling; `draw` only reads it.

use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use magus_core::card::{
    Boost, CardType, CastZone, Condition, Destination, Effect, Keyword, TargetKind, Trigger, Who,
    Whose, signed,
};
use magus_core::mana::{Color, ManaCost};
use magus_core::pool::{AbilitySpec, CardSpec, CastOptionSpec, CostSpec, SpecKind, preview_card};
use magus_core::view::CardView;
use magus_core::{CardPool, ObjectId, Pack};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use serde_json::Value;

use crate::ui::{SELECTED, card_lines, color_of, mana_text, panel};

/// Opens the card builder on the pack at `path` (loading it if it exists) and
/// runs it until the player quits.
pub fn run(path: &Path) -> anyhow::Result<()> {
    let pack = if path.exists() {
        magus_server::load_pack(path)?
    } else {
        Pack::default()
    };
    let mut builder = CardBuilder::new(pack, path.to_path_buf());
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
    Cards,
    Form,
}

/// One line of the card form. Which rows appear depends on the card's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Key,
    Name,
    Kind,
    Cost,
    Power,
    Toughness,
    /// A planeswalker's starting loyalty.
    Loyalty,
    /// Whether a creature is legendary (planeswalkers always are).
    Legendary,
    Mana,
    Subtype,
    Keywords,
    Flashback,
    Flavor,
    /// Cast option `i`'s action ("as you cast this, you may …").
    CastOption(usize),
    /// Cast option `i`'s cost reduction.
    Reduction(usize),
    AddCastOption,
    /// A spell's `i`th effect.
    Effect(usize),
    AddEffect,
    /// Triggered ability `a`'s trigger.
    When(usize),
    /// Triggered ability `a`'s condition.
    If(usize),
    /// Activated ability `a`'s cost.
    ActivationCost(usize),
    /// Static ability `a`.
    Static(usize),
    /// Ability `a`'s `i`th effect (triggered or activated).
    AbilityEffect(usize, usize),
    AddAbilityEffect(usize),
    /// Add a triggered ability.
    AddAbility,
    AddActivated,
    AddStatic,
}

/// Where an effect being edited lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectAt {
    Spell(usize),
    Ability(usize, usize),
    /// A cast option's action.
    CastOption(usize),
    /// Static ability `a` (not an effect, but edited the same way).
    Static(usize),
}

/// A popup or text field that has the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub enum Editing {
    /// Typing into a text field of the card.
    Text {
        row: Row,
        buffer: String,
    },
    /// The effect editor. `effect` is the effect's serialized form, and `new`
    /// means it isn't in the card yet (Esc discards it).
    Effect {
        at: EffectAt,
        effect: Value,
        sel: usize,
        new: bool,
    },
    Keywords {
        sel: usize,
    },
}

/// What the preview shows for a card: how it reads, or why it can't be built.
pub type Preview = Result<CardView, Vec<String>>;

pub struct CardBuilder {
    pub pack: Pack,
    pub path: PathBuf,
    /// For telling when a card's key is already a built-in card's.
    builtin: CardPool,
    pub focus: Focus,
    pub card_sel: usize,
    pub row_sel: usize,
    pub editing: Option<Editing>,
    /// One per card, rebuilt when that card changes. Building leaks the
    /// card's definition (see `preview_card`), so it isn't redone per frame.
    previews: Vec<Preview>,
    pub dirty: bool,
    confirm_quit: bool,
    confirm_delete: bool,
    pub status: Option<String>,
    pub quit: bool,
}

impl CardBuilder {
    pub fn new(pack: Pack, path: PathBuf) -> CardBuilder {
        let previews = pack.cards.iter().map(build_preview).collect();
        CardBuilder {
            pack,
            path,
            builtin: CardPool::builtin(),
            focus: Focus::Cards,
            card_sel: 0,
            row_sel: 0,
            editing: None,
            previews,
            dirty: false,
            confirm_quit: false,
            confirm_delete: false,
            status: None,
            quit: false,
        }
    }

    /// Every card name a search could look for: the built-in cards' and this
    /// pack's.
    pub fn card_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .builtin
            .cards()
            .map(|c| c.name.to_string())
            .chain(self.pack.cards.iter().map(|c| c.name.clone()))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn card(&self) -> Option<&CardSpec> {
        self.pack.cards.get(self.card_sel)
    }

    pub fn preview(&self) -> Option<&Preview> {
        self.previews.get(self.card_sel)
    }

    pub fn rows(&self) -> Vec<Row> {
        self.card().map(rows).unwrap_or_default()
    }

    /// Problems with the selected card beyond the card itself: its key
    /// clashing with a built-in card or another card in the pack.
    pub fn key_problems(&self) -> Vec<String> {
        let Some(card) = self.card() else {
            return Vec::new();
        };
        let mut problems = Vec::new();
        if self.builtin.card(&card.key).is_some() {
            problems.push(format!("the key {:?} is a built-in card's", card.key));
        }
        let same = self.pack.cards.iter().filter(|c| c.key == card.key).count();
        if same > 1 {
            problems.push(format!(
                "{same} cards in this pack have the key {:?}",
                card.key
            ));
        }
        problems
    }

    fn has_problems(&self, i: usize) -> bool {
        let key = &self.pack.cards[i].key;
        self.previews[i].is_err()
            || self.builtin.card(key).is_some()
            || self.pack.cards.iter().filter(|c| &c.key == key).count() > 1
    }

    /// Applies `change` to the selected card and refreshes its preview.
    fn edit(&mut self, change: impl FnOnce(&mut CardSpec)) {
        let Some(card) = self.pack.cards.get_mut(self.card_sel) else {
            return;
        };
        change(card);
        self.previews[self.card_sel] = build_preview(card);
        self.dirty = true;
        self.row_sel = self.row_sel.min(self.rows().len().saturating_sub(1));
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.editing.is_some() {
            self.on_editing_key(key);
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
        let deleting = self.confirm_delete;
        self.confirm_quit = false;
        self.confirm_delete = false;
        self.status = None;
        match key.code {
            KeyCode::Char('s') => return self.save(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Cards if self.card().is_some() => Focus::Form,
                    _ => Focus::Cards,
                };
                return;
            }
            _ => {}
        }
        match self.focus {
            Focus::Cards => self.on_cards_key(key, deleting),
            Focus::Form => self.on_form_key(key),
        }
    }

    fn on_cards_key(&mut self, key: KeyEvent, deleting: bool) {
        let len = self.pack.cards.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if len > 0 => {
                self.card_sel = (self.card_sel + len - 1) % len;
                self.row_sel = 0;
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 => {
                self.card_sel = (self.card_sel + 1) % len;
                self.row_sel = 0;
            }
            KeyCode::Enter | KeyCode::Right if len > 0 => self.focus = Focus::Form,
            KeyCode::Char('n') => {
                let key = self.free_key("new-card");
                self.add_card(new_card(key));
                self.focus = Focus::Form;
                self.row_sel = 0;
            }
            KeyCode::Char('d') if len > 0 => {
                let mut copy = self.pack.cards[self.card_sel].clone();
                copy.key = self.free_key(&format!("{}-copy", copy.key));
                copy.name = format!("{} (copy)", copy.name);
                self.add_card(copy);
            }
            KeyCode::Char('x') | KeyCode::Delete if len > 0 => {
                if deleting {
                    self.pack.cards.remove(self.card_sel);
                    let _ = self.previews.remove(self.card_sel);
                    self.card_sel = self.card_sel.min(self.pack.cards.len().saturating_sub(1));
                    self.dirty = true;
                } else {
                    self.confirm_delete = true;
                    let name = self.pack.cards[self.card_sel].name.clone();
                    self.status = Some(format!("Press x again to delete {name}."));
                }
            }
            _ => {}
        }
    }

    fn add_card(&mut self, card: CardSpec) {
        self.previews.push(build_preview(&card));
        self.pack.cards.push(card);
        self.card_sel = self.pack.cards.len() - 1;
        self.dirty = true;
    }

    /// `base`, or `base-2`, `base-3`… whichever no card already uses.
    fn free_key(&self, base: &str) -> String {
        let taken = |key: &str| {
            self.builtin.card(key).is_some() || self.pack.cards.iter().any(|c| c.key == key)
        };
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|key| !taken(key))
            .expect("some number is free")
    }

    fn on_form_key(&mut self, key: KeyEvent) {
        let rows = self.rows();
        let Some(&row) = rows.get(self.row_sel) else {
            self.focus = Focus::Cards;
            return;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.row_sel = (self.row_sel + rows.len() - 1) % rows.len()
            }
            KeyCode::Down | KeyCode::Char('j') => self.row_sel = (self.row_sel + 1) % rows.len(),
            KeyCode::Esc => self.focus = Focus::Cards,
            KeyCode::Left => self.adjust(row, false),
            KeyCode::Right => self.adjust(row, true),
            KeyCode::Enter => self.activate(row),
            KeyCode::Char('x') | KeyCode::Delete => self.delete(row),
            _ => {}
        }
    }

    /// ← and →: cycle a choice, or step a number.
    fn adjust(&mut self, row: Row, forward: bool) {
        let step = if forward { 1 } else { -1 };
        match row {
            Row::Kind => self.edit(|c| set_kind(c, cycle(&KINDS, &c.kind, forward))),
            Row::Mana => self.edit(|c| {
                let now = c.mana.as_deref().and_then(|m| m.chars().next());
                let colors = Color::ALL.map(|c| c.symbol());
                let next = match now {
                    Some(sym) => cycle(&colors, &sym, forward),
                    None => colors[0],
                };
                c.mana = Some(next.to_string());
            }),
            Row::Power => self.edit(|c| c.power = Some(c.power.unwrap_or(0) + step)),
            Row::Toughness => self.edit(|c| c.toughness = Some(c.toughness.unwrap_or(0) + step)),
            Row::Loyalty => self.edit(|c| c.loyalty = Some(c.loyalty.unwrap_or(0) + step)),
            Row::Legendary => self.edit(|c| c.legendary = !c.legendary),
            Row::When(a) => self.edit(|c| {
                if let AbilitySpec::Triggered { when, .. } = &mut c.abilities[a] {
                    *when = cycle(&Trigger::ALL, &*when, forward);
                }
            }),
            Row::If(a) => self.edit(|c| {
                if let AbilitySpec::Triggered { only_if, .. } = &mut c.abilities[a] {
                    *only_if = cycle(&conditions(), &*only_if, forward);
                }
            }),
            _ => {}
        }
    }

    /// Enter: edit the row (type into it, open a popup, or add something).
    fn activate(&mut self, row: Row) {
        let Some(card) = self.card().cloned() else {
            return;
        };
        let text = |s: &str| Some(s.to_string());
        let buffer = match row {
            Row::Key => text(&card.key),
            Row::Name => text(&card.name),
            Row::Cost => text(&card.cost),
            Row::Subtype => text(&card.subtype),
            Row::Flashback => card.flashback.clone().or_else(|| text("")),
            Row::Flavor => card.flavor.clone().or_else(|| text("")),
            Row::Reduction(i) => text(&card.cast_options[i].reduction),
            Row::Power => card.power.map(|p| p.to_string()).or_else(|| text("")),
            Row::Toughness => card.toughness.map(|t| t.to_string()).or_else(|| text("")),
            Row::Loyalty => card.loyalty.map(|l| l.to_string()).or_else(|| text("")),
            Row::ActivationCost(a) => match &card.abilities[a] {
                AbilitySpec::Activated { cost, .. } => text(&cost_text(cost)),
                _ => None,
            },
            _ => None,
        };
        if let Some(buffer) = buffer {
            self.editing = Some(Editing::Text { row, buffer });
            return;
        }
        let first_example = || serde_json::to_value(&Effect::examples()[0]).expect("serializes");
        self.editing = self.editor_for(row, &card, first_example);
    }

    /// What Enter on `row` opens, if anything (doing it if it's an action).
    fn editor_for(
        &mut self,
        row: Row,
        card: &CardSpec,
        first_example: impl Fn() -> Value,
    ) -> Option<Editing> {
        match row {
            Row::Kind | Row::Mana | Row::When(_) | Row::If(_) | Row::Legendary => {
                self.adjust(row, true);
                None
            }
            Row::Static(a) => Some(Editing::Effect {
                at: EffectAt::Static(a),
                effect: static_json(&card.abilities[a]),
                sel: 0,
                new: false,
            }),
            Row::AddActivated => {
                let ability = if card.kind == SpecKind::Planeswalker {
                    AbilitySpec::Activated {
                        cost: CostSpec {
                            loyalty: Some(1),
                            ..CostSpec::default()
                        },
                        effects: vec![Effect::Draw {
                            who: Who::You,
                            count: 1,
                        }],
                    }
                } else {
                    AbilitySpec::Activated {
                        cost: CostSpec {
                            tap: true,
                            ..CostSpec::default()
                        },
                        effects: vec![Effect::Damage {
                            amount: 1,
                            target: TargetKind::Any,
                            whose: Whose::Anyone,
                        }],
                    }
                };
                self.edit(|c| c.abilities.push(ability));
                None
            }
            Row::AddStatic => {
                self.edit(|c| {
                    c.abilities.push(AbilitySpec::Static {
                        whose: Whose::You,
                        other: c.kind == SpecKind::Creature,
                        power: 1,
                        toughness: 1,
                        keyword: None,
                    })
                });
                None
            }
            Row::Keywords => Some(Editing::Keywords { sel: 0 }),
            Row::Effect(i) => Some(Editing::Effect {
                at: EffectAt::Spell(i),
                effect: serde_json::to_value(&card.effects[i]).expect("serializes"),
                sel: 0,
                new: false,
            }),
            Row::AbilityEffect(a, i) => {
                let effects = ability_effects(&card.abilities[a])?;
                Some(Editing::Effect {
                    at: EffectAt::Ability(a, i),
                    effect: serde_json::to_value(&effects[i]).expect("serializes"),
                    sel: 0,
                    new: false,
                })
            }
            Row::AddEffect => Some(Editing::Effect {
                at: EffectAt::Spell(card.effects.len()),
                effect: first_example(),
                sel: 0,
                new: true,
            }),
            Row::AddAbilityEffect(a) => {
                let effects = ability_effects(&card.abilities[a])?;
                Some(Editing::Effect {
                    at: EffectAt::Ability(a, effects.len()),
                    effect: first_example(),
                    sel: 0,
                    new: true,
                })
            }
            Row::CastOption(i) => Some(Editing::Effect {
                at: EffectAt::CastOption(i),
                effect: serde_json::to_value(&card.cast_options[i].action).expect("serializes"),
                sel: 0,
                new: false,
            }),
            Row::AddCastOption => {
                self.edit(|c| {
                    c.cast_options.push(CastOptionSpec {
                        action: Effect::Search {
                            kind: Some(CardType::Land),
                            color: None,
                            named: None,
                            count: 1,
                            to: Destination::Battlefield,
                            tapped: true,
                        },
                        reduction: "1".into(),
                    })
                });
                None
            }
            Row::AddAbility => {
                self.edit(|c| {
                    c.abilities.push(AbilitySpec::Triggered {
                        when: Trigger::Enters,
                        only_if: None,
                        effects: vec![Effect::Draw {
                            who: Who::You,
                            count: 1,
                        }],
                    })
                });
                None
            }
            _ => None,
        }
    }

    /// x: remove an effect or ability, or clear an optional field.
    fn delete(&mut self, row: Row) {
        match row {
            Row::Effect(i) => self.edit(|c| {
                c.effects.remove(i);
            }),
            Row::AbilityEffect(a, i) => self.edit(|c| {
                if let Some(effects) = ability_effects_mut(&mut c.abilities[a]) {
                    effects.remove(i);
                }
            }),
            Row::When(a) | Row::If(a) | Row::ActivationCost(a) | Row::Static(a) => self.edit(|c| {
                c.abilities.remove(a);
            }),
            Row::CastOption(i) | Row::Reduction(i) => self.edit(|c| {
                c.cast_options.remove(i);
            }),
            Row::Flashback => self.edit(|c| c.flashback = None),
            Row::Flavor => self.edit(|c| c.flavor = None),
            _ => {}
        }
    }

    fn on_editing_key(&mut self, key: KeyEvent) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        self.editing = match editing {
            Editing::Text { row, mut buffer } => match key.code {
                KeyCode::Enter => {
                    self.commit_text(row, buffer);
                    None
                }
                KeyCode::Esc => None,
                KeyCode::Backspace => {
                    buffer.pop();
                    Some(Editing::Text { row, buffer })
                }
                KeyCode::Char(c) if !c.is_control() => {
                    buffer.push(c);
                    Some(Editing::Text { row, buffer })
                }
                _ => Some(Editing::Text { row, buffer }),
            },
            Editing::Keywords { mut sel } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    sel = (sel + Keyword::ALL.len() - 1) % Keyword::ALL.len();
                    Some(Editing::Keywords { sel })
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    sel = (sel + 1) % Keyword::ALL.len();
                    Some(Editing::Keywords { sel })
                }
                KeyCode::Char(' ') | KeyCode::Enter => {
                    let keyword = Keyword::ALL[sel];
                    self.edit(|c| {
                        match c.keywords.iter().position(|&k| k == keyword) {
                            Some(i) => {
                                c.keywords.remove(i);
                            }
                            None => c.keywords.push(keyword),
                        }
                        // Keep them in the usual order.
                        c.keywords
                            .sort_by_key(|k| Keyword::ALL.iter().position(|x| x == k));
                    });
                    Some(Editing::Keywords { sel })
                }
                KeyCode::Esc => None,
                _ => Some(Editing::Keywords { sel }),
            },
            Editing::Effect {
                at,
                mut effect,
                mut sel,
                new,
            } => {
                let fields = effect_fields(&effect);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        sel = (sel + fields.len() - 1) % fields.len()
                    }
                    KeyCode::Down | KeyCode::Char('j') => sel = (sel + 1) % fields.len(),
                    KeyCode::Left => {
                        effect = adjust_field(&effect, &fields[sel], false, &self.card_names())
                    }
                    KeyCode::Right => {
                        effect = adjust_field(&effect, &fields[sel], true, &self.card_names())
                    }
                    KeyCode::Enter => {
                        self.commit_effect(at, &effect, new);
                        return;
                    }
                    KeyCode::Esc => return,
                    _ => {}
                }
                // Changing the type can change how many fields there are.
                sel = sel.min(effect_fields(&effect).len() - 1);
                Some(Editing::Effect {
                    at,
                    effect,
                    sel,
                    new,
                })
            }
        };
    }

    fn commit_text(&mut self, row: Row, buffer: String) {
        let optional = |s: String| {
            let s = s.trim().to_string();
            (!s.is_empty()).then_some(s)
        };
        let number = |s: &str| s.trim().parse::<i32>();
        match row {
            Row::Power | Row::Toughness => match number(&buffer) {
                Ok(n) => self.edit(|c| match row {
                    Row::Power => c.power = Some(n),
                    _ => c.toughness = Some(n),
                }),
                Err(_) => self.status = Some(format!("{:?} isn't a whole number.", buffer.trim())),
            },
            Row::Key => self.edit(|c| c.key = buffer.trim().to_string()),
            Row::Name => self.edit(|c| c.name = buffer.trim().to_string()),
            Row::Cost => self.edit(|c| c.cost = buffer.trim().to_uppercase()),
            Row::Subtype => self.edit(|c| c.subtype = buffer.trim().to_string()),
            Row::Flashback => self.edit(|c| c.flashback = optional(buffer.to_uppercase())),
            Row::Flavor => self.edit(|c| c.flavor = optional(buffer)),
            Row::Reduction(i) => self.edit(|c| {
                c.cast_options[i].reduction = buffer.trim().to_uppercase();
            }),
            Row::Loyalty => match number(&buffer) {
                Ok(n) => self.edit(|c| c.loyalty = Some(n)),
                Err(_) => self.status = Some(format!("{:?} isn't a whole number.", buffer.trim())),
            },
            Row::ActivationCost(a) => {
                let planeswalker = self
                    .card()
                    .is_some_and(|c| c.kind == SpecKind::Planeswalker);
                match parse_cost(&buffer, planeswalker) {
                    Ok(cost) => self.edit(|c| {
                        if let AbilitySpec::Activated { cost: now, .. } = &mut c.abilities[a] {
                            *now = cost;
                        }
                    }),
                    Err(e) => self.status = Some(e),
                }
            }
            _ => {}
        }
    }

    fn commit_effect(&mut self, at: EffectAt, effect: &Value, new: bool) {
        if let EffectAt::Static(a) = at {
            match serde_json::from_value::<AbilitySpec>(effect.clone()) {
                Ok(ability) => self.edit(|c| c.abilities[a] = ability),
                Err(e) => self.status = Some(format!("That doesn't work: {e}")),
            }
            return;
        }
        let effect: Effect = match serde_json::from_value(effect.clone()) {
            Ok(effect) => effect,
            Err(e) => {
                self.status = Some(format!("That effect doesn't work: {e}"));
                return;
            }
        };
        self.edit(|c| {
            let list = match at {
                EffectAt::Spell(_) => &mut c.effects,
                EffectAt::Ability(a, _) => match ability_effects_mut(&mut c.abilities[a]) {
                    Some(effects) => effects,
                    None => return,
                },
                EffectAt::CastOption(i) => {
                    c.cast_options[i].action = effect;
                    return;
                }
                EffectAt::Static(_) => unreachable!("handled above"),
            };
            let (EffectAt::Spell(i) | EffectAt::Ability(_, i)) = at else {
                unreachable!("handled above")
            };
            if new {
                list.push(effect);
            } else {
                list[i] = effect;
            }
        });
    }

    /// Writes the pack (cards and any decks it already had), even with
    /// problems, so unfinished cards aren't lost; then reports any problems.
    pub fn save(&mut self) {
        let text = match toml::to_string(&self.pack) {
            Ok(text) => text,
            Err(e) => {
                self.status = Some(format!("Couldn't save: {e}"));
                return;
            }
        };
        if let Err(e) = std::fs::write(&self.path, text) {
            self.status = Some(format!("Couldn't save {}: {e}", self.path.display()));
            return;
        }
        self.dirty = false;
        self.status = Some(match CardPool::builtin().add_pack(self.pack.clone()) {
            Ok(()) => format!(
                "Saved {}. A server can load it with --cards.",
                self.path.display()
            ),
            Err(e) => format!(
                "Saved {}, but a server would refuse it: {}",
                self.path.display(),
                e.0.join("; ")
            ),
        });
    }
}

const KINDS: [SpecKind; 5] = [
    SpecKind::Creature,
    SpecKind::Instant,
    SpecKind::Sorcery,
    SpecKind::Land,
    SpecKind::Planeswalker,
];

fn kind_name(kind: SpecKind) -> &'static str {
    kind.name()
}

/// The effects of an ability that has them (triggered or activated).
fn ability_effects(ability: &AbilitySpec) -> Option<&Vec<Effect>> {
    match ability {
        AbilitySpec::Triggered { effects, .. } | AbilitySpec::Activated { effects, .. } => {
            Some(effects)
        }
        AbilitySpec::Static { .. } => None,
    }
}

fn ability_effects_mut(ability: &mut AbilitySpec) -> Option<&mut Vec<Effect>> {
    match ability {
        AbilitySpec::Triggered { effects, .. } | AbilitySpec::Activated { effects, .. } => {
            Some(effects)
        }
        AbilitySpec::Static { .. } => None,
    }
}

/// A static ability with every field present (the pack format leaves out
/// defaults), for the editor.
fn static_json(ability: &AbilitySpec) -> Value {
    match ability {
        AbilitySpec::Static {
            whose,
            other,
            power,
            toughness,
            keyword,
        } => serde_json::json!({
            "type": "static",
            "whose": whose,
            "other": other,
            "power": power,
            "toughness": toughness,
            "keyword": keyword,
        }),
        _ => Value::Null,
    }
}

/// How a cost is typed and shown: "+1", "−2", "{T}", "{1}{R}, {T}".
fn cost_text(cost: &CostSpec) -> String {
    let mut parts = Vec::new();
    if let Some(n) = cost.loyalty {
        parts.push(if n == 0 { "0".to_string() } else { signed(n) });
    }
    if let Some(mana) = &cost.mana {
        parts.push(
            ManaCost::try_parse(mana)
                .map(|m| m.to_string())
                .unwrap_or_else(|_| mana.clone()),
        );
    }
    if cost.tap {
        parts.push("{T}".into());
    }
    parts.join(", ")
}

/// Reads a typed cost. For a planeswalker it's a loyalty change ("+1", "-2",
/// "0"); otherwise "T" (or "{T}") to tap and/or mana such as "1R".
fn parse_cost(text: &str, planeswalker: bool) -> Result<CostSpec, String> {
    let text = text.trim().replace('\u{2212}', "-");
    if planeswalker {
        return text
            .trim_start_matches('+')
            .parse::<i32>()
            .map(|n| CostSpec {
                loyalty: Some(n),
                ..CostSpec::default()
            })
            .map_err(|_| format!("{text:?} isn't a loyalty change like +1 or -2."));
    }
    let mut cost = CostSpec::default();
    for part in text.split([',', ' ']).filter(|p| !p.is_empty()) {
        let part = part.replace(['{', '}'], "").to_uppercase();
        if part == "T" {
            cost.tap = true;
        } else {
            let mana = cost.mana.take().unwrap_or_default();
            cost.mana = Some(mana + &part);
        }
    }
    Ok(cost)
}

/// The value after (or before) `now` in `options`, wrapping around.
fn cycle<T: Clone + PartialEq>(options: &[T], now: &T, forward: bool) -> T {
    let i = options.iter().position(|o| o == now).unwrap_or(0);
    let n = options.len();
    options[if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    }]
    .clone()
}

/// Every condition an ability can have, starting with none.
fn conditions() -> Vec<Option<Condition>> {
    let mut all = vec![None];
    for zone in CastZone::ALL {
        all.push(Some(Condition::CastFrom { zone }));
        all.push(Some(Condition::NotCastFrom { zone }));
    }
    all
}

/// Changes a card's type, dropping the fields the new type can't have and
/// filling in the ones it needs.
fn set_kind(card: &mut CardSpec, kind: SpecKind) {
    card.kind = kind;
    let creature = kind == SpecKind::Creature;
    let land = kind == SpecKind::Land;
    let planeswalker = kind == SpecKind::Planeswalker;
    let spell = matches!(kind, SpecKind::Instant | SpecKind::Sorcery);
    if creature {
        card.power.get_or_insert(1);
        card.toughness.get_or_insert(1);
    } else {
        card.power = None;
        card.toughness = None;
        card.keywords.clear();
    }
    if planeswalker {
        card.loyalty.get_or_insert(3);
        let has_loyalty = card
            .abilities
            .iter()
            .any(|a| matches!(a, AbilitySpec::Activated { cost, .. } if cost.loyalty.is_some()));
        if !has_loyalty {
            card.abilities.push(AbilitySpec::Activated {
                cost: CostSpec {
                    loyalty: Some(1),
                    ..CostSpec::default()
                },
                effects: vec![Effect::Draw {
                    who: Who::You,
                    count: 1,
                }],
            });
        }
    } else {
        card.loyalty = None;
        // Loyalty abilities belong to planeswalkers only.
        card.abilities.retain(
            |a| !matches!(a, AbilitySpec::Activated { cost, .. } if cost.loyalty.is_some()),
        );
    }
    if !creature && !planeswalker {
        card.abilities.clear();
    }
    if !creature {
        card.legendary = false;
    }
    if land {
        card.cost.clear();
        card.subtype.clear();
        card.mana.get_or_insert_with(|| "W".into());
    } else {
        card.mana = None;
    }
    if !spell {
        card.effects.clear();
        card.flashback = None;
    }
    if land {
        card.cast_options.clear();
    }
}

fn new_card(key: String) -> CardSpec {
    CardSpec {
        key,
        name: "New Card".into(),
        kind: SpecKind::Creature,
        cost: "1".into(),
        power: Some(1),
        toughness: Some(1),
        mana: None,
        subtype: String::new(),
        keywords: Vec::new(),
        effects: Vec::new(),
        flashback: None,
        flavor: None,
        cast_options: Vec::new(),
        loyalty: None,
        legendary: false,
        abilities: Vec::new(),
    }
}

fn build_preview(card: &CardSpec) -> Preview {
    preview_card(card).map(|def| CardView::new(ObjectId(0), def))
}

/// The form's rows for `card`, in order.
fn rows(card: &CardSpec) -> Vec<Row> {
    let creature = card.kind == SpecKind::Creature;
    let land = card.kind == SpecKind::Land;
    let planeswalker = card.kind == SpecKind::Planeswalker;
    let spell = !creature && !land && !planeswalker;
    let mut rows = vec![Row::Key, Row::Name, Row::Kind];
    if !land {
        rows.push(Row::Cost);
    }
    if creature {
        rows.extend([Row::Power, Row::Toughness, Row::Legendary]);
    }
    if planeswalker {
        rows.push(Row::Loyalty);
    }
    if land {
        rows.push(Row::Mana);
    } else {
        rows.push(Row::Subtype);
    }
    if creature {
        rows.push(Row::Keywords);
    }
    if spell {
        rows.push(Row::Flashback);
    }
    rows.push(Row::Flavor);
    if !land {
        for i in 0..card.cast_options.len() {
            rows.extend([Row::CastOption(i), Row::Reduction(i)]);
        }
        rows.push(Row::AddCastOption);
    }
    if spell {
        rows.extend((0..card.effects.len()).map(Row::Effect));
        rows.push(Row::AddEffect);
    }
    if creature || planeswalker {
        for (a, ability) in card.abilities.iter().enumerate() {
            match ability {
                AbilitySpec::Triggered { .. } => rows.extend([Row::When(a), Row::If(a)]),
                AbilitySpec::Activated { .. } => rows.push(Row::ActivationCost(a)),
                AbilitySpec::Static { .. } => rows.push(Row::Static(a)),
            }
            if let Some(effects) = ability_effects(ability) {
                rows.extend((0..effects.len()).map(|i| Row::AbilityEffect(a, i)));
                rows.push(Row::AddAbilityEffect(a));
            }
        }
        if creature {
            rows.push(Row::AddAbility);
        }
        rows.extend([Row::AddActivated, Row::AddStatic]);
    }
    rows
}

// ----- the effect editor -----

/// The effect's fields in display order: its type, then the rest by name.
fn effect_fields(effect: &Value) -> Vec<String> {
    // A static ability is edited the same way, but has no other kinds to
    // switch to.
    let mut fields = if effect["type"] == "static" {
        Vec::new()
    } else {
        vec!["type".to_string()]
    };
    if let Value::Object(map) = effect {
        fields.extend(map.keys().filter(|k| *k != "type").cloned());
    }
    fields
}

/// How a static ability (in its editor form) reads.
fn static_reads(value: &Value) -> String {
    match serde_json::from_value::<AbilitySpec>(value.clone()) {
        Ok(AbilitySpec::Static {
            whose,
            other,
            power,
            toughness,
            keyword,
        }) => {
            let boost = Boost {
                whose,
                power,
                toughness,
                keyword,
            };
            format!("{}.", boost.describe(other))
        }
        _ => String::new(),
    }
}

/// The serialized name of each value in `values`.
fn names<T: serde::Serialize>(values: &[T]) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| serde_json::to_value(v).ok()?.as_str().map(str::to_string))
        .collect()
}

/// The values a field can take, in order; `null` means "any" or "none".
/// Empty for numbers and yes/no fields, which ← → change directly.
/// `card_names` are the choices for a search's `named`.
fn field_options(field: &str, card_names: &[String]) -> Vec<Value> {
    let strings = |v: Vec<String>| v.into_iter().map(Value::String).collect::<Vec<_>>();
    let optional = |v: Vec<String>| {
        std::iter::once(Value::Null)
            .chain(v.into_iter().map(Value::String))
            .collect()
    };
    match field {
        "type" => strings(
            Effect::examples()
                .iter()
                .filter_map(|e| Some(serde_json::to_value(e).ok()?["type"].as_str()?.to_string()))
                .collect(),
        ),
        "who" => strings(names(&Who::ALL)),
        "whose" => strings(names(&Whose::ALL)),
        // Only `damage` has a `target`, and it can only aim at these.
        "target" => strings(["any", "creature", "player"].map(String::from).to_vec()),
        "to" => strings(names(&Destination::ALL)),
        "kind" => optional(names(&CardType::ALL)),
        "color" => optional(names(&Color::ALL)),
        "named" => optional(card_names.to_vec()),
        "keyword" => optional(names(&Keyword::ALL)),
        _ => Vec::new(),
    }
}

/// `effect` with `field` moved to its next (or previous) value.
fn adjust_field(effect: &Value, field: &str, forward: bool, card_names: &[String]) -> Value {
    let mut effect = effect.clone();
    let now = effect[field].clone();
    if field == "type" {
        // A different kind of effect: start from its example.
        let next = cycle(&field_options("type", card_names), &now, forward);
        return Effect::examples()
            .into_iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .find(|e| e["type"] == next)
            .unwrap_or(effect);
    }
    effect[field] = match now {
        Value::Number(n) => {
            let n = n.as_i64().unwrap_or(0) + if forward { 1 } else { -1 };
            Value::from(n.clamp(-99, 99))
        }
        Value::Bool(b) => Value::Bool(!b),
        other => {
            let options = field_options(field, card_names);
            if options.is_empty() {
                other
            } else {
                cycle(&options, &other, forward)
            }
        }
    };
    effect
}

/// How a field's value is shown in the effect editor.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => "(any)".into(),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::String(s) => s.replace('_', " "),
        other => other.to_string(),
    }
}

// ----- drawing -----

pub fn draw(frame: &mut Frame, b: &CardBuilder) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    let [cards, form, preview] = Layout::horizontal([
        Constraint::Length(30),
        Constraint::Fill(1),
        Constraint::Length(44),
    ])
    .areas(body);

    let dirty = if b.dirty { " (unsaved)" } else { "" };
    frame.render_widget(
        Line::from(vec![
            " MAGUS ".bold().black().on_cyan(),
            Span::raw("  Card builder · "),
            format!("{}{dirty}", b.path.display()).dark_gray(),
        ]),
        header,
    );
    draw_cards(frame, cards, b);
    draw_form(frame, form, b);
    draw_preview(frame, preview, b);
    draw_footer(frame, footer, b);
    match &b.editing {
        Some(Editing::Effect { effect, sel, .. }) => draw_effect_popup(frame, body, effect, *sel),
        Some(Editing::Keywords { sel }) => draw_keyword_popup(frame, body, b, *sel),
        _ => {}
    }
}

/// The colors a card would show as, worked out from the spec so it works even
/// when the card can't be built yet.
fn spec_colors(card: &CardSpec) -> Vec<Color> {
    match card.kind {
        SpecKind::Land => card
            .mana
            .as_deref()
            .and_then(|m| m.chars().next())
            .and_then(Color::from_symbol)
            .into_iter()
            .collect(),
        _ => ManaCost::try_parse(&card.cost)
            .map(|c| c.colors())
            .unwrap_or_default(),
    }
}

fn draw_cards(frame: &mut Frame, area: Rect, b: &CardBuilder) {
    let items: Vec<ListItem> = b
        .pack
        .cards
        .iter()
        .enumerate()
        .map(|(i, card)| {
            let marker = if b.has_problems(i) {
                "✗ ".red()
            } else {
                "  ".into()
            };
            let name = Span::styled(
                card.name.clone(),
                Style::new().fg(color_of(&spec_colors(card))).bold(),
            );
            ListItem::new(Line::from(vec![marker, name]))
        })
        .collect();
    let focused = b.focus == Focus::Cards && b.editing.is_none();
    let title = format!(" Cards ({}) ", b.pack.cards.len());
    let list = if items.is_empty() {
        List::new([ListItem::new(
            Line::from("No cards yet: press n.").dark_gray(),
        )])
    } else {
        List::new(items)
    };
    let mut state = ListState::default().with_selected(Some(b.card_sel));
    frame.render_stateful_widget(
        list.block(panel(title, focused)).highlight_style(SELECTED),
        area,
        &mut state,
    );
}

/// The label and value shown for `row` of `card`.
fn row_line(card: &CardSpec, row: Row) -> Line<'static> {
    let none = || Span::raw("(none)").dark_gray();
    let plain = |s: &str| Span::raw(s.to_string());
    let (label, value): (String, Vec<Span<'static>>) = match row {
        Row::Key => ("Key".into(), vec![plain(&card.key)]),
        Row::Name => ("Name".into(), vec![plain(&card.name)]),
        Row::Kind => (
            "Type".into(),
            vec![plain(kind_name(card.kind)), "  ◂ ▸".dark_gray()],
        ),
        Row::Cost => ("Cost".into(), cost_spans(&card.cost)),
        Row::Power => ("Power".into(), vec![plain(&opt_num(card.power))]),
        Row::Toughness => ("Toughness".into(), vec![plain(&opt_num(card.toughness))]),
        Row::Mana => (
            "Taps for".into(),
            match &card.mana {
                Some(m) => {
                    let mut spans = mana_text(&format!("{{{m}}}"), Style::new());
                    spans.push("  ◂ ▸".dark_gray());
                    spans
                }
                None => vec![none()],
            },
        ),
        Row::Subtype => (
            "Subtype".into(),
            if card.subtype.is_empty() {
                vec![none()]
            } else {
                vec![plain(&card.subtype)]
            },
        ),
        Row::Keywords => (
            "Keywords".into(),
            if card.keywords.is_empty() {
                vec![none()]
            } else {
                let names: Vec<_> = card.keywords.iter().map(|k| k.name()).collect();
                vec![plain(&names.join(", "))]
            },
        ),
        Row::Flashback => (
            "Flashback".into(),
            match &card.flashback {
                Some(cost) => cost_spans(cost),
                None => vec![none()],
            },
        ),
        Row::Flavor => (
            "Flavor".into(),
            match &card.flavor {
                Some(f) => vec![Span::raw(f.clone()).italic()],
                None => vec![none()],
            },
        ),
        Row::CastOption(i) => (
            "As you cast".into(),
            vec![plain(&format!(
                "you may {}",
                card.cast_options[i].action.describe()
            ))],
        ),
        Row::Reduction(i) => (
            "  costs less".into(),
            cost_spans(&card.cast_options[i].reduction),
        ),
        Row::AddCastOption => (String::new(), vec!["+ add cast option".green()]),
        Row::Effect(i) => (
            format!("Effect {}", i + 1),
            vec![plain(&card.effects[i].describe())],
        ),
        Row::AddEffect => (String::new(), vec!["+ add effect".green()]),
        Row::Loyalty => ("Loyalty".into(), vec![plain(&opt_num(card.loyalty))]),
        Row::Legendary => (
            "Legendary".into(),
            vec![
                plain(if card.legendary { "yes" } else { "no" }),
                "  ◂ ▸".dark_gray(),
            ],
        ),
        Row::When(a) => {
            let text = match &card.abilities[a] {
                AbilitySpec::Triggered { when, .. } => when.describe(),
                _ => "",
            };
            (
                format!("Ability {}", a + 1),
                vec![plain(text), "  ◂ ▸".dark_gray()],
            )
        }
        Row::If(a) => {
            let text = match &card.abilities[a] {
                AbilitySpec::Triggered {
                    only_if: Some(c), ..
                } => c.describe(),
                _ => "(always)".to_string(),
            };
            ("  if".into(), vec![plain(&text), "  ◂ ▸".dark_gray()])
        }
        Row::ActivationCost(a) => {
            let cost = match &card.abilities[a] {
                AbilitySpec::Activated { cost, .. } => cost_text(cost),
                _ => String::new(),
            };
            (
                format!("Ability {}", a + 1),
                mana_text(&format!("{cost}:"), Style::new()),
            )
        }
        Row::Static(a) => {
            let text = static_json(&card.abilities[a]);
            let reads = static_reads(&text);
            (format!("Ability {}", a + 1), vec![plain(&reads)])
        }
        Row::AbilityEffect(a, i) => {
            let text = ability_effects(&card.abilities[a])
                .map(|effects| effects[i].describe())
                .unwrap_or_default();
            ("  then".into(), vec![plain(&text)])
        }
        Row::AddAbilityEffect(_) => (String::new(), vec!["  + add effect".green()]),
        Row::AddAbility => (String::new(), vec!["+ add triggered ability".green()]),
        Row::AddActivated => (String::new(), vec!["+ add activated ability".green()]),
        Row::AddStatic => (String::new(), vec!["+ add static ability".green()]),
    };
    let mut spans = vec![format!("{label:<11}").dark_gray()];
    spans.extend(value);
    Line::from(spans)
}

/// What a text field is called, while typing into it.
fn text_label(row: Row) -> &'static str {
    match row {
        Row::Key => "Key",
        Row::Name => "Name",
        Row::Cost => "Cost (e.g. 2RR)",
        Row::Power => "Power",
        Row::Toughness => "Toughness",
        Row::Subtype => "Subtype",
        Row::Flashback => "Flashback cost (empty for none)",
        Row::Flavor => "Flavor text (empty for none)",
        Row::Reduction(_) => "Costs less by (e.g. B or 1)",
        Row::Loyalty => "Starting loyalty",
        Row::ActivationCost(_) => "Cost (planeswalker: +1, -2…; others: T and/or mana like 1R)",
        _ => "Value",
    }
}

fn opt_num(n: Option<i32>) -> String {
    n.map_or("(none)".into(), |n| n.to_string())
}

/// A cost as typed, with its symbols colored if it's valid.
fn cost_spans(cost: &str) -> Vec<Span<'static>> {
    match ManaCost::try_parse(cost) {
        Ok(parsed) if !cost.is_empty() => {
            let mut spans = mana_text(&parsed.to_string(), Style::new());
            spans.push(format!("  ({cost})").dark_gray());
            spans
        }
        Ok(_) => vec![Span::raw("(free)").dark_gray()],
        Err(_) => vec![Span::raw(cost.to_string()).red()],
    }
}

fn draw_form(frame: &mut Frame, area: Rect, b: &CardBuilder) {
    let focused = b.focus == Focus::Form && b.editing.is_none();
    let Some(card) = b.card() else {
        frame.render_widget(
            Paragraph::new("Select a card, or press n for a new one.")
                .dark_gray()
                .block(panel(" Card ", focused)),
            area,
        );
        return;
    };
    let items: Vec<ListItem> = rows(card)
        .into_iter()
        .map(|row| ListItem::new(row_line(card, row)))
        .collect();
    let mut state = ListState::default().with_selected(focused.then_some(b.row_sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(format!(" {} ", card.name), focused))
            .highlight_style(SELECTED),
        area,
        &mut state,
    );
}

fn draw_preview(frame: &mut Frame, area: Rect, b: &CardBuilder) {
    let mut lines = Vec::new();
    let mut problems = b.key_problems();
    match b.preview() {
        Some(Ok(card)) => {
            lines = card_lines(card);
            if let (Some(p), Some(t)) = (card.power, card.toughness) {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("{p}/{t}")).bold());
            }
        }
        Some(Err(card_problems)) => {
            problems = card_problems.iter().cloned().chain(problems).collect();
        }
        None => lines.push(Line::from("No card selected.").dark_gray()),
    }
    if b.card().is_some() {
        lines.push(Line::from(""));
        if problems.is_empty() {
            lines.push(Line::from("Ready to use.").green().bold());
        } else {
            lines.push(Line::from("Problems").red().bold());
            lines.extend(
                problems
                    .into_iter()
                    .map(|p| Line::from(format!("• {p}")).red()),
            );
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(panel(" Preview ", false)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, b: &CardBuilder) {
    let keys = match (&b.editing, b.focus) {
        (Some(Editing::Text { row, buffer }), _) => Line::from(vec![
            Span::raw(format!("{}: ", text_label(*row))),
            Span::styled(format!("{buffer}▏"), Style::new().bold()),
            "   Enter: done · Esc: cancel".dark_gray(),
        ]),
        (Some(Editing::Effect { .. }), _) => {
            Line::from("↑↓: field · ←→: change · Enter: done · Esc: cancel").dark_gray()
        }
        (Some(Editing::Keywords { .. }), _) => {
            Line::from("↑↓: select · Space: toggle · Esc: done").dark_gray()
        }
        (None, Focus::Cards) => Line::from(
            "↑↓: select · Enter/Tab: edit · n: new · d: duplicate · x: delete · s: save · q: quit",
        )
        .dark_gray(),
        (None, Focus::Form) => Line::from(
            "↑↓: field · Enter: edit · ←→: change · x: remove · Tab/Esc: cards · s: save · q: quit",
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

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn draw_effect_popup(frame: &mut Frame, area: Rect, effect: &Value, sel: usize) {
    let fields = effect_fields(effect);
    let is_static = effect["type"] == "static";
    let mut items: Vec<ListItem> = fields
        .iter()
        .map(|f| {
            let value = match (&effect[f.as_str()], f.as_str()) {
                (Value::Null, "keyword") => "(none)".to_string(),
                (v, _) => shown(v),
            };
            ListItem::new(Line::from(vec![
                format!("{f:<8}").dark_gray(),
                Span::raw(value),
                "  ◂ ▸".dark_gray(),
            ]))
        })
        .collect();
    let reads = if is_static {
        static_reads(effect)
    } else {
        serde_json::from_value::<Effect>(effect.clone())
            .map(|e| e.describe())
            .unwrap_or_default()
    };
    items.push(ListItem::new(""));
    items.push(ListItem::new(Line::from(format!("“{reads}”")).italic()));
    let popup = centered(area, 64, fields.len() as u16 + 4);
    frame.render_widget(Clear, popup);
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(
                if is_static {
                    " Static ability "
                } else {
                    " Effect "
                },
                true,
            ))
            .highlight_style(SELECTED),
        popup,
        &mut state,
    );
}

fn draw_keyword_popup(frame: &mut Frame, area: Rect, b: &CardBuilder, sel: usize) {
    let have: &[Keyword] = b.card().map_or(&[], |c| &c.keywords);
    let items: Vec<ListItem> = Keyword::ALL
        .iter()
        .map(|k| {
            let mark = if have.contains(k) { "[x] " } else { "[ ] " };
            ListItem::new(format!("{mark}{}", k.name()))
        })
        .collect();
    let popup = centered(area, 30, Keyword::ALL.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Keywords ", true))
            .highlight_style(SELECTED),
        popup,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn press(b: &mut CardBuilder, code: KeyCode) {
        b.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typing(b: &mut CardBuilder, text: &str) {
        for c in text.chars() {
            press(b, KeyCode::Char(c));
        }
    }

    /// Moves the form's cursor to `row`.
    fn select(b: &mut CardBuilder, row: Row) {
        b.focus = Focus::Form;
        b.row_sel = b
            .rows()
            .iter()
            .position(|&r| r == row)
            .expect("row is shown");
    }

    fn screen(b: &CardBuilder) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 36)).unwrap();
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

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("magus-cardbuilder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Builds "Ember Lance", a sorcery that deals 3 damage, entirely by keys.
    fn make_ember_lance(b: &mut CardBuilder) {
        press(b, KeyCode::Char('n'));
        select(b, Row::Key);
        press(b, KeyCode::Enter);
        for _ in 0..20 {
            press(b, KeyCode::Backspace);
        }
        typing(b, "ember-lance");
        press(b, KeyCode::Enter);
        select(b, Row::Name);
        press(b, KeyCode::Enter);
        for _ in 0..20 {
            press(b, KeyCode::Backspace);
        }
        typing(b, "Ember Lance");
        press(b, KeyCode::Enter);
        select(b, Row::Kind);
        press(b, KeyCode::Right); // instant
        press(b, KeyCode::Right); // sorcery
        select(b, Row::Cost);
        press(b, KeyCode::Enter);
        press(b, KeyCode::Backspace);
        typing(b, "1r");
        press(b, KeyCode::Enter);
        select(b, Row::AddEffect);
        press(b, KeyCode::Enter); // the effect editor opens on `damage`
        press(b, KeyCode::Down); // amount
        press(b, KeyCode::Right); // 2 → 3
        press(b, KeyCode::Enter);
    }

    #[test]
    fn builds_a_spell_with_the_keyboard() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        make_ember_lance(&mut b);
        let card = b.card().unwrap();
        assert_eq!(card.kind, SpecKind::Sorcery);
        assert_eq!(card.cost, "1R");
        assert_eq!(card.power, None, "a sorcery has no power");
        let Some(Ok(view)) = b.preview() else {
            panic!("expected a working card, got {:?}", b.preview());
        };
        assert_eq!(view.text, "Deal 3 damage to any target.");
        assert!(b.key_problems().is_empty());
        let shown = screen(&b);
        println!("{shown}");
        assert!(shown.contains("Ready to use."));
    }

    #[test]
    fn creature_abilities_and_keywords() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        press(&mut b, KeyCode::Char('n'));
        select(&mut b, Row::Keywords);
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Char(' ')); // flying
        press(&mut b, KeyCode::Esc);
        select(&mut b, Row::AddAbility);
        press(&mut b, KeyCode::Enter);
        select(&mut b, Row::If(0));
        press(&mut b, KeyCode::Right); // cast from hand
        press(&mut b, KeyCode::Right); // not cast from hand
        let Some(Ok(view)) = b.preview() else {
            panic!("{:?}", b.preview());
        };
        assert_eq!(
            view.text,
            "Flying\nWhen this creature enters, if you didn't cast it from your hand, draw a card."
        );
        // Effects in abilities can't target: the preview says so.
        select(&mut b, Row::AbilityEffect(0, 0));
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Left); // `draw` back to `counter`, which targets
        press(&mut b, KeyCode::Enter);
        let Some(Err(problems)) = b.preview() else {
            panic!("{:?}", b.preview());
        };
        assert!(
            problems[0].contains("triggered abilities can't target"),
            "{problems:?}"
        );
    }

    #[test]
    fn effect_editor_offers_every_kind_of_effect() {
        let types = field_options("type", &[]);
        assert_eq!(types.len(), Effect::examples().len());
        // Cycling through every type and back lands where it started, and
        // every stop is a real effect.
        let start = serde_json::to_value(&Effect::examples()[0]).unwrap();
        let mut effect = start.clone();
        for _ in 0..types.len() {
            effect = adjust_field(&effect, "type", true, &[]);
            serde_json::from_value::<Effect>(effect.clone()).expect("a real effect");
        }
        assert_eq!(effect, start);
    }

    #[test]
    fn clashing_keys_are_problems() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        press(&mut b, KeyCode::Char('n'));
        select(&mut b, Row::Key);
        press(&mut b, KeyCode::Enter);
        for _ in 0..20 {
            press(&mut b, KeyCode::Backspace);
        }
        typing(&mut b, "blaze");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.key_problems(), ["the key \"blaze\" is a built-in card's"]);
        // New cards get a free key.
        press(&mut b, KeyCode::Tab);
        press(&mut b, KeyCode::Char('n'));
        press(&mut b, KeyCode::Tab);
        press(&mut b, KeyCode::Char('n'));
        let keys: Vec<_> = b.pack.cards.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["blaze", "new-card", "new-card-2"]);
    }

    #[test]
    fn saves_a_pack_a_server_accepts_and_keeps_its_decks() {
        let path = temp_path("lances.toml");
        let deck = magus_core::pool::DeckSpec {
            key: "lances".into(),
            name: "Lances".into(),
            description: String::new(),
            cards: [("ember-lance".to_string(), 4), ("crag".to_string(), 56)]
                .into_iter()
                .collect(),
        };
        let pack = Pack {
            cards: Vec::new(),
            decks: vec![deck.clone()],
        };
        let mut b = CardBuilder::new(pack, path.clone());
        make_ember_lance(&mut b);
        press(&mut b, KeyCode::Char('s'));
        assert!(!b.dirty);
        assert!(
            b.status
                .as_deref()
                .unwrap()
                .contains("A server can load it"),
            "{:?}",
            b.status
        );

        let pool = magus_server::load_pool(&[&path]).unwrap();
        assert_eq!(pool.card("ember-lance").unwrap().name, "Ember Lance");
        assert_eq!(pool.deck("lances").unwrap().size(), 60, "the deck survived");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn flavor_text_shows_in_the_preview() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        press(&mut b, KeyCode::Char('n'));
        select(&mut b, Row::Flavor);
        press(&mut b, KeyCode::Enter);
        typing(&mut b, "I knew I forgot something...");
        press(&mut b, KeyCode::Enter);
        let shown = screen(&b);
        assert!(shown.contains("I knew I forgot something..."), "{shown}");
        select(&mut b, Row::Flavor);
        press(&mut b, KeyCode::Char('x'));
        assert_eq!(b.card().unwrap().flavor, None);
    }

    #[test]
    fn cast_options_and_search_fields() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        press(&mut b, KeyCode::Char('n'));
        select(&mut b, Row::Cost);
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Backspace);
        typing(&mut b, "2bb");
        press(&mut b, KeyCode::Enter);
        select(&mut b, Row::AddCastOption);
        press(&mut b, KeyCode::Enter);
        select(&mut b, Row::Reduction(0));
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Backspace);
        typing(&mut b, "b");
        press(&mut b, KeyCode::Enter);

        // Fields, in order: type, color, count, kind, named, tapped, to.
        select(&mut b, Row::CastOption(0));
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Down); // color
        for _ in 0..3 {
            press(&mut b, KeyCode::Right); // (any) → white → blue → black
        }
        let Some(Editing::Effect { effect, .. }) = &b.editing else {
            panic!("the effect editor is open");
        };
        assert_eq!(effect["color"], "black");
        for _ in 0..4 {
            press(&mut b, KeyCode::Down); // count, kind, named, tapped
        }
        press(&mut b, KeyCode::Right); // yes → no
        press(&mut b, KeyCode::Enter);
        let Some(Ok(view)) = b.preview() else {
            panic!("{:?}", b.preview());
        };
        assert_eq!(
            view.text,
            "As you cast this spell, you may search your library for a land card that taps for \
             {B}, put it onto the battlefield, then shuffle. If you do, this spell costs {B} \
             less to cast."
        );

        // A search for a named card cycles through real card names.
        select(&mut b, Row::CastOption(0));
        press(&mut b, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut b, KeyCode::Down); // named
        }
        press(&mut b, KeyCode::Right);
        let Some(Editing::Effect { effect, .. }) = &b.editing else {
            panic!("the effect editor is open");
        };
        assert_eq!(effect["named"], b.card_names()[0].as_str());
    }

    #[test]
    fn building_a_planeswalker() {
        let mut b = CardBuilder::new(Pack::default(), temp_path("unused.toml"));
        press(&mut b, KeyCode::Char('n'));
        select(&mut b, Row::Kind);
        for _ in 0..4 {
            press(&mut b, KeyCode::Right); // creature → … → planeswalker
        }
        let card = b.card().unwrap();
        assert_eq!(card.kind, SpecKind::Planeswalker);
        assert_eq!(card.loyalty, Some(3));
        assert!(b.rows().contains(&Row::Loyalty));
        assert!(
            !b.rows().contains(&Row::AddAbility),
            "no triggered abilities offered"
        );

        // It got a +1 ability; make it −3 and deal damage instead.
        select(&mut b, Row::ActivationCost(0));
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Backspace);
        press(&mut b, KeyCode::Backspace);
        typing(&mut b, "-3");
        press(&mut b, KeyCode::Enter);
        select(&mut b, Row::AbilityEffect(0, 0));
        press(&mut b, KeyCode::Enter);
        for _ in 0..5 {
            press(&mut b, KeyCode::Left); // draw → … → damage
        }
        press(&mut b, KeyCode::Enter);
        // Add a static ability (ability 2) and give it a keyword.
        select(&mut b, Row::AddStatic);
        press(&mut b, KeyCode::Enter);
        select(&mut b, Row::Static(1));
        press(&mut b, KeyCode::Enter);
        // Fields: keyword, other, power, toughness, whose.
        press(&mut b, KeyCode::Right); // keyword: (none) → flying
        press(&mut b, KeyCode::Enter);

        let Some(Ok(view)) = b.preview() else {
            panic!("{:?}", b.preview());
        };
        assert_eq!(
            view.text,
            "\u{2212}3: Deal 2 damage to any target.\n\
             Creatures you control get +1/+1 and have flying."
        );
        assert_eq!(view.type_line, "Legendary Planeswalker");
        assert_eq!(view.loyalty, Some(3));
    }
}
