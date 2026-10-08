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
    /// Flavor text, if any: shown in italics, with no effect on play.
    pub flavor: Option<String>,
    pub colors: Vec<Color>,
    pub keywords: Vec<Keyword>,
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    /// A planeswalker's starting loyalty.
    pub loyalty: Option<i32>,
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
            flavor: def.flavor.map(str::to_string),
            colors: def.colors(),
            keywords: def.keywords.to_vec(),
            power: def.power(),
            toughness: def.toughness(),
            loyalty: def.loyalty(),
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
    /// Current power and toughness, including temporary boosts and static
    /// abilities.
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    /// Current keywords, including ones granted by static abilities.
    pub keywords: Vec<Keyword>,
    /// A planeswalker's current loyalty.
    pub loyalty: Option<i32>,
    /// The player it's attacking (the planeswalker's controller, if it's
    /// attacking a planeswalker).
    pub attacking: Option<PlayerId>,
    pub attacking_planeswalker: Option<ObjectId>,
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
    pub exile: Vec<CardView>,
    /// What each of their emblems says.
    pub emblems: Vec<String>,
    pub lost: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayKind {
    Land,
    Spell,
    /// Activate the card's activated ability with this index (`card` is
    /// then a permanent on the battlefield).
    Ability(usize),
}

/// A card you could play right now. Spells that target list every legal target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayOption {
    pub card: ObjectId,
    pub kind: PlayKind,
    pub targets: Option<Vec<Target>>,
    /// For a spell, what this way of casting it costs, e.g. "{2}{B}".
    pub cost: String,
    /// Set when this is one of a card's optional ways to cast it.
    pub way: Option<CastWay>,
    /// For an ability, its text, e.g. "−2: Deal 3 damage to target creature."
    pub ability: Option<String>,
}

/// One of a card's optional ways to cast it (a cast option), and the card
/// it would fetch. Sent back in [`crate::Action::Cast`] to choose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastWay {
    /// Which of the card's cast options.
    pub option: usize,
    /// The key of the card it fetches.
    pub fetch: String,
    /// That card's name, for showing to the player.
    pub fetch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackOption {
    pub attacker: ObjectId,
    pub defenders: Vec<PlayerId>,
    /// Opponents' planeswalkers it may attack instead.
    pub planeswalkers: Vec<ObjectId>,
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
    /// Choose between `min` and `max` of these cards: for a spell or ability
    /// you control, or for the legend rule. They may be in a hidden zone, such as your
    /// library during a search, so the cards are shown here in full.
    ChooseCards {
        /// What the choice is for, e.g. "Beckon the Wild: you may put a
        /// creature card from your hand onto the battlefield".
        reason: String,
        options: Vec<CardView>,
        /// At least this many must be chosen (usually 0).
        min: usize,
        max: usize,
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

    /// A card in anyone's graveyard, and whose graveyard it's in.
    pub fn graveyard_card(&self, id: ObjectId) -> Option<(PlayerId, &CardView)> {
        self.players
            .iter()
            .find_map(|p| Some((p.id, p.graveyard.iter().find(|c| c.id == id)?)))
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
            Target::GraveyardCard(id) => match self.graveyard_card(id) {
                Some((owner, card)) => {
                    format!("{} (in {}'s graveyard)", card.name, self.player_name(owner))
                }
                None => "a card in a graveyard".into(),
            },
        }
    }
}
