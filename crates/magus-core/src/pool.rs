//! The set of cards and decks a server offers: the built-in ones plus any
//! loaded from card packs.
//!
//! A pack is plain data (see [`Pack`]); this module validates it and adds it to
//! a [`CardPool`]. Parsing a pack from a file format lives outside the engine,
//! so the engine itself only needs `serde`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::card::{
    Ability, ActivationCost, CardDef, CardKind, CastOption, Condition, Effect, Keyword, Trigger,
    Whose,
};
use crate::cards::{self, DeckList};
use crate::mana::Color;

/// Every deck must have exactly this many cards.
pub const DECK_SIZE: u32 = 60;

/// A deck may have at most this many copies of any card except basic lands.
pub const MAX_COPIES: u32 = 4;

/// Cards and decks from outside the engine, as read from (or written to) a
/// pack file. A deck saved by the deck builder is a pack with just one deck.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    #[serde(default, rename = "card", skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<CardSpec>,
    #[serde(default, rename = "deck", skip_serializing_if = "Vec::is_empty")]
    pub decks: Vec<DeckSpec>,
}

/// One card in a pack. Which fields apply depends on `kind`: creatures need
/// `power` and `toughness`, lands need `mana`, and spells need `effects`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CardSpec {
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: SpecKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cost: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toughness: Option<i32>,
    /// For planeswalkers, the loyalty it enters with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loyalty: Option<i32>,
    /// Legendary (planeswalkers always are): see the legend rule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legendary: bool,
    /// For lands, the mana symbol it taps for, e.g. `"R"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mana: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtype: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<Keyword>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// For instants and sorceries, a cost for casting it from the graveyard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flashback: Option<String>,
    /// Flavor text, shown in italics under the rules text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    #[serde(default, rename = "cast_option", skip_serializing_if = "Vec::is_empty")]
    pub cast_options: Vec<CastOptionSpec>,
    // Last, because TOML writes arrays of tables after plain fields.
    #[serde(default, rename = "ability", skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<AbilitySpec>,
}

/// A card's type in a pack file.
pub use crate::card::CardType as SpecKind;

/// An owned [`CastOption`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CastOptionSpec {
    pub action: Effect,
    pub reduction: String,
}

/// An owned [`Ability`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AbilitySpec {
    Triggered {
        when: Trigger,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        only_if: Option<Condition>,
        effects: Vec<Effect>,
    },
    Activated {
        cost: CostSpec,
        effects: Vec<Effect>,
    },
    Static {
        #[serde(default = "Whose::you")]
        whose: Whose,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        other: bool,
        #[serde(default, skip_serializing_if = "is_zero")]
        power: i32,
        #[serde(default, skip_serializing_if = "is_zero")]
        toughness: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keyword: Option<Keyword>,
    },
}

/// An owned [`ActivationCost`]: e.g. `{ loyalty = -2 }`, `{ tap = true }`,
/// `{ mana = "1R", tap = true }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CostSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loyalty: Option<i32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana: Option<String>,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

    /// Every card, in no particular order.
    pub fn cards(&self) -> impl Iterator<Item = &'static CardDef> + '_ {
        self.cards.values().copied()
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
            problems.extend(self.search_problems(def));
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
        let mut new_defs: Vec<_> = new_cards.values().collect();
        new_defs.sort_by_key(|def| def.key);
        for def in new_defs {
            problems.extend(combined.search_problems(def));
        }
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

    /// Searches in `def` for a card named something no card in the pool is
    /// called: almost certainly a typo, since they could never find anything.
    fn search_problems(&self, def: &CardDef) -> Vec<String> {
        let ability_effects = def.abilities.iter().flat_map(|a| a.effects().iter());
        let cast_actions = def.cast_options.iter().map(|o| &o.action);
        def.effects
            .iter()
            .chain(ability_effects)
            .chain(cast_actions)
            .filter_map(|e| e.search_filter()?.named)
            .filter(|name| self.cards.values().all(|c| c.name != *name))
            .map(|name| {
                format!(
                    "card {:?}: searches for a card named {name:?}, but no card has that name",
                    def.key
                )
            })
            .collect()
    }

    fn deck_problems(&self, deck: &DeckList) -> Vec<String> {
        self.decklist_problems(deck.cards.iter().copied())
            .into_iter()
            .map(|p| format!("deck {:?}: {p}", deck.key))
            .collect()
    }

    /// Everything wrong with a decklist of (card key, copies) as a deck for
    /// this pool: its size, cards the pool doesn't have, and too many copies.
    /// Pack decks, the deck builder and players' own decks all use this.
    pub fn decklist_problems<'a>(
        &self,
        cards: impl IntoIterator<Item = (&'a str, u32)>,
    ) -> Vec<String> {
        let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
        for (key, n) in cards {
            *counts.entry(key).or_default() += u64::from(n);
        }
        let mut problems = Vec::new();
        let size: u64 = counts.values().sum();
        if size != u64::from(DECK_SIZE) {
            problems.push(format!(
                "has {size} cards, but decks must have exactly {DECK_SIZE}"
            ));
        }
        for (key, n) in counts {
            match self.card(key) {
                None => problems.push(format!("unknown card {key:?}")),
                Some(def) if !def.is_land() && n > u64::from(MAX_COPIES) => problems.push(format!(
                    "{n} copies of {}, but at most {MAX_COPIES} are allowed (basic lands \
                         are exempt)",
                    def.name
                )),
                Some(_) => {}
            }
        }
        problems
    }
}

/// Builds `spec` on its own, for a card editor's live preview: the definition
/// it makes, or why it can't be built. Doesn't check that its key is free.
///
/// The definition is leaked, like every pack card; an editor should only call
/// this when the card has changed.
pub fn preview_card(spec: &CardSpec) -> Result<&'static CardDef, Vec<String>> {
    build_card(spec.clone())
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
        SpecKind::Planeswalker => match spec.loyalty {
            Some(loyalty) => Some(CardKind::Planeswalker { loyalty }),
            None => {
                problems.push("a planeswalker needs a starting `loyalty`".into());
                None
            }
        },
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
    if spec.kind != SpecKind::Planeswalker && spec.loyalty.is_some() {
        problems.push("only planeswalkers have `loyalty`".into());
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
                AbilitySpec::Triggered {
                    when,
                    only_if,
                    effects,
                } => Ability::Triggered {
                    when,
                    only_if,
                    effects: effects.leak(),
                },
                AbilitySpec::Activated { cost, effects } => Ability::Activated {
                    cost: ActivationCost {
                        loyalty: cost.loyalty,
                        tap: cost.tap,
                        mana: cost.mana.map(leak),
                    },
                    effects: effects.leak(),
                },
                AbilitySpec::Static {
                    whose,
                    other,
                    power,
                    toughness,
                    keyword,
                } => Ability::Static {
                    whose,
                    other,
                    power,
                    toughness,
                    keyword,
                },
            })
            .collect::<Vec<_>>()
            .leak(),
        flashback: spec.flashback.map(leak),
        flavor: spec.flavor.map(leak),
        cast_options: spec
            .cast_options
            .into_iter()
            .map(|o| CastOption {
                action: o.action,
                reduction: leak(o.reduction),
            })
            .collect::<Vec<_>>()
            .leak(),
        legendary: spec.legendary,
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
            flashback: None,
            flavor: None,
            cast_options: vec![],
            loyalty: None,
            legendary: false,
            abilities: vec![AbilitySpec::Triggered {
                when: Trigger::Enters,
                only_if: None,
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
            decks: vec![deck("test-deck", &[("test-courier", 4), ("lagoon", 56)])],
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
            only_if: None,
            effects: vec![Effect::Damage {
                amount: 1,
                target: TargetKind::Any,
                whose: Default::default(),
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
                deck("test-short", &[("test-fine", 4)]),
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
                "deck \"test-short\": has 4 cards, but decks must have exactly 60",
                "deck \"test-missing\": unknown card \"test-wisp\"",
            ]
        );
        assert!(
            pool.card("test-fine").is_none(),
            "nothing from a bad pack is added"
        );
    }

    #[test]
    fn that_player_needs_a_trigger_with_a_player() {
        let lose = |who| Effect::LoseGame { who };
        let mut on_enter = creature("test-curse");
        on_enter.abilities = vec![AbilitySpec::Triggered {
            when: Trigger::Enters,
            only_if: None,
            effects: vec![lose(Who::ThatPlayer)],
        }];
        let mut on_hit = creature("test-reaver");
        on_hit.abilities = vec![AbilitySpec::Triggered {
            when: Trigger::DealsCombatDamageToPlayer,
            only_if: None,
            effects: vec![lose(Who::ThatPlayer)],
        }];
        let spell = CardSpec {
            kind: SpecKind::Sorcery,
            power: None,
            toughness: None,
            keywords: vec![],
            effects: vec![lose(Who::ThatPlayer)],
            abilities: vec![],
            ..creature("test-hex")
        };
        let pack = Pack {
            cards: vec![on_enter, on_hit, spell],
            decks: vec![],
        };
        let PackError(problems) = CardPool::builtin().add_pack(pack).unwrap_err();
        assert_eq!(
            problems,
            [
                "card \"test-curse\": ability 1 uses \"that_player\", but nothing it reacts to \
                 involves a player",
                "card \"test-hex\": \"that_player\" only works in an ability triggered by a player",
            ]
        );
    }

    #[test]
    fn at_most_four_copies_except_basic_lands() {
        let pool = CardPool::builtin();
        let ok = [("blaze", 4), ("crag", 56)];
        assert_eq!(pool.decklist_problems(ok), Vec::<String>::new());
        let too_many = [("blaze", 5), ("crag", 55)];
        assert_eq!(
            pool.decklist_problems(too_many),
            ["5 copies of Blaze, but at most 4 are allowed (basic lands are exempt)"]
        );
        // Counts for the same card add up, and huge counts don't overflow.
        let split = [("blaze", 3), ("crag", 54), ("blaze", 3)];
        assert_eq!(pool.decklist_problems(split).len(), 1);
        let huge = [("crag", u32::MAX), ("bog", u32::MAX)];
        assert_eq!(
            pool.decklist_problems(huge),
            ["has 8589934590 cards, but decks must have exactly 60"]
        );
    }

    #[test]
    fn packs_survive_being_written_and_read_back() {
        let text = include_str!("../tests/fixtures/sample-pack.toml");
        let pack: Pack = toml::from_str(text).unwrap();
        let written = toml::to_string(&pack).unwrap();
        let again: Pack = toml::from_str(&written).unwrap();
        assert_eq!(again, pack, "written as:\n{written}");
    }

    #[test]
    fn searches_must_name_a_real_card() {
        let mut seeker = creature("test-seeker");
        seeker.abilities = vec![AbilitySpec::Triggered {
            when: Trigger::Enters,
            only_if: None,
            effects: vec![Effect::Search {
                kind: None,
                color: None,
                named: Some("Bgo".into()),
                count: 1,
                to: crate::card::Destination::Hand,
                tapped: false,
            }],
        }];
        let pack = Pack {
            cards: vec![seeker],
            decks: vec![],
        };
        let PackError(problems) = CardPool::builtin().add_pack(pack).unwrap_err();
        assert_eq!(
            problems,
            ["card \"test-seeker\": searches for a card named \"Bgo\", but no card has that name"]
        );
    }

    #[test]
    fn loyalty_abilities_belong_to_planeswalkers() {
        let mut creature_with_loyalty = creature("test-confused");
        creature_with_loyalty.abilities = vec![AbilitySpec::Activated {
            cost: CostSpec {
                loyalty: Some(1),
                ..CostSpec::default()
            },
            effects: vec![Effect::Draw {
                who: Who::You,
                count: 1,
            }],
        }];
        let mut walker = creature("test-walker");
        walker.kind = SpecKind::Planeswalker;
        walker.power = None;
        walker.toughness = None;
        walker.keywords = vec![];
        walker.loyalty = Some(3);
        walker.abilities = vec![AbilitySpec::Static {
            whose: Whose::You,
            other: false,
            power: 1,
            toughness: 0,
            keyword: None,
        }];
        let pack = Pack {
            cards: vec![creature_with_loyalty, walker],
            decks: vec![],
        };
        let PackError(problems) = CardPool::builtin().add_pack(pack).unwrap_err();
        assert_eq!(
            problems,
            [
                "card \"test-confused\": ability 1: only planeswalkers have loyalty abilities",
                "card \"test-walker\": a planeswalker needs at least one loyalty ability",
            ]
        );
    }
}
