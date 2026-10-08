//! The set of cards and decks a server offers: the built-in ones plus any
//! loaded from card packs.
//!
//! A pack is plain data (see [`Pack`]); this module validates it and adds it to
//! a [`CardPool`]. Parsing a pack from a file format lives outside the engine,
//! so the engine itself only needs `serde`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::Deserialize;

use crate::card::{Ability, CardDef, CardKind, Effect, Keyword, Trigger};
use crate::cards::{self, DeckList};
use crate::mana::Color;

/// Every deck must have exactly this many cards.
pub const DECK_SIZE: u32 = 60;

/// Cards and decks from outside the engine, as read from a pack file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    #[serde(default, rename = "card")]
    pub cards: Vec<CardSpec>,
    #[serde(default, rename = "deck")]
    pub decks: Vec<DeckSpec>,
}

/// One card in a pack. Which fields apply depends on `kind`: creatures need
/// `power` and `toughness`, lands need `mana`, and spells need `effects`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardSpec {
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: SpecKind,
    #[serde(default)]
    pub cost: String,
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    /// For lands, the mana symbol it taps for, e.g. `"R"`.
    pub mana: Option<String>,
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub keywords: Vec<Keyword>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default, rename = "ability")]
    pub abilities: Vec<AbilitySpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecKind {
    Creature,
    Instant,
    Sorcery,
    Land,
}

/// An owned [`Ability`].
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AbilitySpec {
    Triggered { when: Trigger, effects: Vec<Effect> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeckSpec {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Card key → number of copies.
    pub cards: BTreeMap<String, u32>,
}

/// Why a pack was rejected: one readable line per problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackError(pub Vec<String>);

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join("\n"))
    }
}

impl std::error::Error for PackError {}

/// The cards and decks available to games.
///
/// Card definitions are `&'static`: pack data is leaked when it's added, which
/// suits a pool that's built once at startup and lives as long as the process.
#[derive(Debug, Clone)]
pub struct CardPool {
    cards: HashMap<&'static str, &'static CardDef>,
    decks: Vec<&'static DeckList>,
}

impl CardPool {
    /// Just the cards and decks that ship with the engine.
    pub fn builtin() -> CardPool {
        CardPool {
            cards: cards::CARDS.iter().map(|c| (c.key, c)).collect(),
            decks: cards::DECKS.iter().collect(),
        }
    }

    pub fn card(&self, key: &str) -> Option<&'static CardDef> {
        self.cards.get(key).copied()
    }

    pub fn deck(&self, key: &str) -> Option<&'static DeckList> {
        self.decks.iter().find(|d| d.key == key).copied()
    }

    /// Every deck, built-in ones first, then packs in the order they were added.
    pub fn decks(&self) -> &[&'static DeckList] {
        &self.decks
    }

    /// Checks the pool's own consistency: every card well-formed and every
    /// deck the right size and made of cards that exist.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut cards: Vec<_> = self.cards.values().collect();
        cards.sort_by_key(|c| c.key);
        for def in cards {
            problems.extend(
                def.problems()
                    .into_iter()
                    .map(|p| format!("card {:?}: {p}", def.key)),
            );
        }
        for deck in &self.decks {
            problems.extend(self.deck_problems(deck));
        }
        problems
    }

    /// Validates `pack` against this pool and, if it's sound, adds its cards
    /// and decks. On error, the pool is unchanged.
    pub fn add_pack(&mut self, pack: Pack) -> Result<(), PackError> {
        let mut problems = Vec::new();
        let mut new_cards = HashMap::new();
        for spec in pack.cards {
            let key = spec.key.clone();
            if self.cards.contains_key(key.as_str()) || new_cards.contains_key(&key) {
                problems.push(format!("card {key:?}: that key is already taken"));
                continue;
            }
            match build_card(spec) {
                Ok(def) => {
                    new_cards.insert(key, def);
                }
                Err(errs) => {
                    problems.extend(errs.into_iter().map(|p| format!("card {key:?}: {p}")));
                }
            }
        }

        // Decks may use the pack's own cards, so check them against the
        // combined pool. Cards that failed validation count as missing.
        let mut combined = self.clone();
        combined
            .cards
            .extend(new_cards.values().map(|def| (def.key, *def)));
        let mut deck_keys: HashSet<String> = self.decks.iter().map(|d| d.key.to_string()).collect();
        let mut new_decks = Vec::new();
        for spec in pack.decks {
            if !deck_keys.insert(spec.key.clone()) {
                problems.push(format!("deck {:?}: that key is already taken", spec.key));
                continue;
            }
            let deck = build_deck(spec);
            let deck_problems = combined.deck_problems(deck);
            if deck_problems.is_empty() {
                new_decks.push(deck);
            }
            problems.extend(deck_problems);
        }

        if !problems.is_empty() {
            return Err(PackError(problems));
        }
        self.cards = combined.cards;
        self.decks.extend(new_decks);
        Ok(())
    }

    fn deck_problems(&self, deck: &DeckList) -> Vec<String> {
        let mut problems = Vec::new();
        if deck.size() != DECK_SIZE {
            problems.push(format!(
                "deck {:?}: has {} cards, but decks must have exactly {DECK_SIZE}",
                deck.key,
                deck.size()
            ));
        }
        for (key, _) in deck.cards {
            if self.card(key).is_none() {
                problems.push(format!("deck {:?}: unknown card {key:?}", deck.key));
            }
        }
        problems
    }
}

/// Turns a spec into a validated, leaked definition.
fn build_card(spec: CardSpec) -> Result<&'static CardDef, Vec<String>> {
    let mut problems = Vec::new();
    let kind = match spec.kind {
        SpecKind::Creature => match (spec.power, spec.toughness) {
            (Some(power), Some(toughness)) => Some(CardKind::Creature { power, toughness }),
            _ => {
                problems.push("a creature needs both power and toughness".into());
                None
            }
        },
        SpecKind::Instant => Some(CardKind::Instant),
        SpecKind::Sorcery => Some(CardKind::Sorcery),
        SpecKind::Land => {
            let mut symbols = spec.mana.as_deref().unwrap_or("").chars();
            match (symbols.next().and_then(Color::from_symbol), symbols.next()) {
                (Some(color), None) => Some(CardKind::Land(color)),
                _ => {
                    problems.push(
                        "a land needs `mana`, one of \"W\", \"U\", \"B\", \"R\" or \"G\"".into(),
                    );
                    None
                }
            }
        }
    };
    if spec.kind != SpecKind::Creature && (spec.power.is_some() || spec.toughness.is_some()) {
        problems.push("only creatures have power and toughness".into());
    }
    if spec.kind != SpecKind::Land && spec.mana.is_some() {
        problems.push("only lands have `mana`".into());
    }
    let Some(kind) = kind else {
        return Err(problems);
    };

    let def = CardDef {
        key: leak(spec.key),
        name: leak(spec.name),
        cost: leak(spec.cost),
        kind,
        subtype: leak(spec.subtype),
        keywords: spec.keywords.leak(),
        effects: spec.effects.leak(),
        abilities: spec
            .abilities
            .into_iter()
            .map(|a| match a {
                AbilitySpec::Triggered { when, effects } => Ability::Triggered {
                    when,
                    effects: effects.leak(),
                },
            })
            .collect::<Vec<_>>()
            .leak(),
    };
    problems.extend(def.problems());
    if problems.is_empty() {
        Ok(Box::leak(Box::new(def)))
    } else {
        Err(problems)
    }
}

fn build_deck(spec: DeckSpec) -> &'static DeckList {
    let cards: Vec<(&'static str, u32)> =
        spec.cards.into_iter().map(|(k, n)| (leak(k), n)).collect();
    Box::leak(Box::new(DeckList {
        key: leak(spec.key),
        name: leak(spec.name),
        description: leak(spec.description),
        cards: cards.leak(),
    }))
}

/// A rejected pack leaks the strings it got as far as building. Packs are
/// loaded once at startup, so that's a few bytes, once.
fn leak(s: String) -> &'static str {
    s.leak()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{TargetKind, Who};

    fn creature(key: &str) -> CardSpec {
        CardSpec {
            key: key.into(),
            name: "Test Creature".into(),
            kind: SpecKind::Creature,
            cost: "1U".into(),
            power: Some(2),
            toughness: Some(1),
            mana: None,
            subtype: String::new(),
            keywords: vec![Keyword::Flying],
            effects: vec![],
            abilities: vec![AbilitySpec::Triggered {
                when: Trigger::Enters,
                effects: vec![Effect::Draw {
                    who: Who::You,
                    count: 1,
                }],
            }],
        }
    }

    fn deck(key: &str, cards: &[(&str, u32)]) -> DeckSpec {
        DeckSpec {
            key: key.into(),
            name: "Test Deck".into(),
            description: String::new(),
            cards: cards.iter().map(|&(k, n)| (k.to_string(), n)).collect(),
        }
    }

    #[test]
    fn builtin_pool_is_valid() {
        assert_eq!(CardPool::builtin().problems(), Vec::<String>::new());
    }

    #[test]
    fn pack_cards_and_decks_join_the_pool() {
        let mut pool = CardPool::builtin();
        let pack = Pack {
            cards: vec![creature("test-courier")],
            decks: vec![deck("test-deck", &[("test-courier", 36), ("lagoon", 24)])],
        };
        pool.add_pack(pack).unwrap();
        let def = pool.card("test-courier").unwrap();
        assert_eq!(
            def.rules_text(),
            "Flying\nWhen this creature enters, draw a card."
        );
        assert_eq!(pool.deck("test-deck").unwrap().size(), 60);
        assert_eq!(pool.decks().last().unwrap().key, "test-deck");
        assert_eq!(pool.problems(), Vec::<String>::new());
    }

    #[test]
    fn bad_packs_are_rejected_whole() {
        let mut pool = CardPool::builtin();
        let mut targeted = creature("test-sniper");
        targeted.abilities = vec![AbilitySpec::Triggered {
            when: Trigger::Enters,
            effects: vec![Effect::Damage {
                amount: 1,
                target: TargetKind::Any,
            }],
        }];
        let mut no_toughness = creature("test-wisp");
        no_toughness.toughness = None;
        let mut bad_cost = creature("test-gremlin");
        bad_cost.cost = "1X".into();
        let pack = Pack {
            cards: vec![
                creature("test-fine"),
                creature("lagoon"),
                targeted,
                no_toughness,
                bad_cost,
            ],
            decks: vec![
                deck("tide-ash", &[("lagoon", 60)]),
                deck("test-short", &[("test-fine", 10)]),
                deck("test-missing", &[("test-wisp", 30), ("lagoon", 30)]),
            ],
        };
        let PackError(problems) = pool.add_pack(pack).unwrap_err();
        assert_eq!(
            problems,
            [
                "card \"lagoon\": that key is already taken",
                "card \"test-sniper\": ability 1 has a targeted effect, but triggered abilities \
                 can't target yet",
                "card \"test-wisp\": a creature needs both power and toughness",
                "card \"test-gremlin\": bad mana symbol 'X' in cost \"1X\"",
                "deck \"tide-ash\": that key is already taken",
                "deck \"test-short\": has 10 cards, but decks must have exactly 60",
                "deck \"test-missing\": unknown card \"test-wisp\"",
            ]
        );
        assert!(
            pool.card("test-fine").is_none(),
            "nothing from a bad pack is added"
        );
    }
}
