//! What one player is allowed to see. Views are self-contained: a client can
//! render the game and offer every legal choice without knowing any rules.

use serde::{Deserialize, Serialize};

use crate::card::{CardDef, Keyword};
use crate::game::{ObjectId, PlayerId, Step, Target};
use crate::mana::Color;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardView {
    pub id: ObjectId,
    pub key: String,
    pub name: String,
    pub cost: String,
    pub mana_value: u32,
    pub type_line: String,
    pub text: String,
    pub colors: Vec<Color>,
    pub keywords: Vec<Keyword>,
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    pub is_land: bool,
    pub is_creature: bool,
}

impl CardView {
    pub fn new(id: ObjectId, def: &CardDef) -> CardView {
        let cost = def.mana_cost();
        CardView {
            id,
            key: def.key.into(),
            name: def.name.into(),
            cost: cost.to_string(),
            mana_value: cost.mana_value(),
            type_line: def.type_line(),
            text: def.rules_text(),
            colors: def.colors(),
            keywords: def.keywords.to_vec(),
            power: def.power(),
            toughness: def.toughness(),
            is_land: def.is_land(),
            is_creature: def.is_creature(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermanentView {
    pub card: CardView,
    pub controller: PlayerId,
    pub owner: PlayerId,
    pub tapped: bool,
    /// Entered this turn and can't attack yet.
    pub summoning_sick: bool,
    pub damage: i32,
    /// Current power and toughness, including temporary boosts.
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    pub attacking: Option<PlayerId>,
    pub blocking: Option<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackItemView {
    pub id: ObjectId,
    /// The spell itself, or the creature whose ability this is.
    pub card: CardView,
    pub controller: PlayerId,
    pub is_spell: bool,
    pub target: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerView {
    pub id: PlayerId,
    pub name: String,
    pub life: i32,
    pub hand_size: usize,
    pub library_size: usize,
    pub graveyard: Vec<CardView>,
    pub lost: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayKind {
    Land,
    Spell,
}

/// A card you could play right now. Spells that target list every legal target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayOption {
    pub card: ObjectId,
    pub kind: PlayKind,
    pub targets: Option<Vec<Target>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackOption {
    pub attacker: ObjectId,
    pub defenders: Vec<PlayerId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockOption {
    pub blocker: ObjectId,
    pub attackers: Vec<ObjectId>,
}

/// The decision the game needs from this player, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Prompt {
    /// Someone else is deciding.
    Waiting {
        on: PlayerId,
    },
    /// You may play one of these, or pass.
    Priority {
        plays: Vec<PlayOption>,
    },
    /// Choose any subset of these creatures to attack (possibly none).
    DeclareAttackers {
        options: Vec<AttackOption>,
    },
    /// Choose blocks; each blocker blocks at most one attacker.
    DeclareBlockers {
        options: Vec<BlockOption>,
    },
    /// You're over the hand size limit; discard exactly this many cards.
    Discard {
        count: usize,
    },
    GameOver {
        winner: Option<PlayerId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameView {
    /// Increments with every accepted action; actions must quote it.
    pub version: u64,
    pub you: PlayerId,
    pub players: Vec<PlayerView>,
    pub hand: Vec<CardView>,
    pub battlefield: Vec<PermanentView>,
    /// Bottom first; the last item resolves next.
    pub stack: Vec<StackItemView>,
    pub turn: u32,
    pub active: PlayerId,
    pub step: Step,
    pub priority: Option<PlayerId>,
    pub prompt: Prompt,
    pub log: Vec<String>,
}

impl GameView {
    pub fn player_name(&self, p: PlayerId) -> &str {
        self.players.get(p).map_or("?", |pl| pl.name.as_str())
    }

    pub fn permanent(&self, id: ObjectId) -> Option<&PermanentView> {
        self.battlefield.iter().find(|p| p.card.id == id)
    }

    pub fn stack_item(&self, id: ObjectId) -> Option<&StackItemView> {
        self.stack.iter().find(|s| s.id == id)
    }

    pub fn hand_card(&self, id: ObjectId) -> Option<&CardView> {
        self.hand.iter().find(|c| c.id == id)
    }

    pub fn opponents(&self) -> impl Iterator<Item = &PlayerView> {
        self.players.iter().filter(move |p| p.id != self.you)
    }

    pub fn describe_target(&self, target: &Target) -> String {
        match *target {
            Target::Player(p) if p == self.you => "you".into(),
            Target::Player(p) => self.player_name(p).into(),
            Target::Permanent(id) => match self.permanent(id) {
                Some(perm) => format!("{} ({})", perm.card.name, self.player_name(perm.controller)),
                None => "a creature".into(),
            },
            Target::Spell(id) => match self.stack_item(id) {
                Some(item) => format!("{} ({})", item.card.name, self.player_name(item.controller)),
                None => "a spell".into(),
            },
        }
    }
}
