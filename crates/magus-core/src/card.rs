use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::mana::{Color, ManaCost};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keyword {
    /// Can't be blocked except by creatures with flying or reach.
    Flying,
    /// Can block creatures with flying.
    Reach,
    /// Can attack the turn it enters.
    Haste,
    /// Doesn't tap to attack.
    Vigilance,
    /// Damage it deals also gains its controller that much life.
    Lifelink,
    /// Any damage it deals to a creature is lethal.
    Deathtouch,
    /// Can't attack.
    Defender,
}

impl Keyword {
    pub const ALL: [Keyword; 7] = [
        Keyword::Flying,
        Keyword::Reach,
        Keyword::Haste,
        Keyword::Vigilance,
        Keyword::Lifelink,
        Keyword::Deathtouch,
        Keyword::Defender,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Keyword::Flying => "Flying",
            Keyword::Reach => "Reach",
            Keyword::Haste => "Haste",
            Keyword::Vigilance => "Vigilance",
            Keyword::Lifelink => "Lifelink",
            Keyword::Deathtouch => "Deathtouch",
            Keyword::Defender => "Defender",
        }
    }
}

/// What a spell may target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// A creature or a player.
    Any,
    Creature,
    Player,
    Spell,
    /// A creature card in a graveyard.
    CreatureCardInGraveyard,
    Planeswalker,
}

/// Whose things a spell may target, from its controller's point of view:
/// who controls the creature or spell, whose graveyard the card is in, or
/// which player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Whose {
    #[default]
    Anyone,
    You,
    Opponent,
}

impl Whose {
    pub const ALL: [Whose; 3] = [Whose::Anyone, Whose::You, Whose::Opponent];

    pub(crate) fn you() -> Whose {
        Whose::You
    }
}

/// What a spell targets: a kind of thing, and whose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetSpec {
    pub kind: TargetKind,
    pub whose: Whose,
}

impl TargetSpec {
    /// The target as a noun phrase, e.g. "target creature an opponent controls".
    pub fn describe(self) -> String {
        let suffix = |yours: &str, theirs: &str| match self.whose {
            Whose::Anyone => String::new(),
            Whose::You => format!(" {yours}"),
            Whose::Opponent => format!(" {theirs}"),
        };
        match self.kind {
            TargetKind::Any => match self.whose {
                Whose::Opponent => "target opponent or creature an opponent controls".into(),
                _ => "any target".into(),
            },
            TargetKind::Player => match self.whose {
                Whose::Opponent => "target opponent".into(),
                _ => "target player".into(),
            },
            TargetKind::Creature => {
                format!(
                    "target creature{}",
                    suffix("you control", "an opponent controls")
                )
            }
            TargetKind::Spell => {
                format!(
                    "target spell{}",
                    suffix("you control", "an opponent controls")
                )
            }
            TargetKind::Planeswalker => {
                format!(
                    "target planeswalker{}",
                    suffix("you control", "an opponent controls")
                )
            }
            TargetKind::CreatureCardInGraveyard => format!(
                "target creature card from {}",
                match self.whose {
                    Whose::Anyone => "a graveyard",
                    Whose::You => "your graveyard",
                    Whose::Opponent => "an opponent's graveyard",
                }
            ),
        }
    }
}

/// Which player(s) an untargeted effect applies to, from the point of view of
/// whoever controls the spell or ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    You,
    EachOpponent,
    /// The player in the event that triggered the ability, e.g. the player
    /// who was dealt combat damage. Only valid for triggers that involve one.
    ThatPlayer,
}

impl Who {
    pub const ALL: [Who; 3] = [Who::You, Who::EachOpponent, Who::ThatPlayer];

    /// The subject of a sentence, e.g. "you" or "each opponent".
    fn noun(self) -> &'static str {
        match self {
            Who::You => "you",
            Who::EachOpponent => "each opponent",
            Who::ThatPlayer => "that player",
        }
    }

    /// `verb` conjugated to agree with [`Who::noun`]: "you gain" but "each
    /// opponent gains".
    fn conjugate(self, verb: &str) -> String {
        match self {
            Who::You => verb.to_string(),
            Who::EachOpponent | Who::ThatPlayer => format!("{verb}s"),
        }
    }
}

/// A kind of card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    Creature,
    Instant,
    Sorcery,
    Land,
    Planeswalker,
}

impl CardType {
    pub const ALL: [CardType; 5] = [
        CardType::Creature,
        CardType::Instant,
        CardType::Sorcery,
        CardType::Land,
        CardType::Planeswalker,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CardType::Creature => "creature",
            CardType::Instant => "instant",
            CardType::Sorcery => "sorcery",
            CardType::Land => "land",
            CardType::Planeswalker => "planeswalker",
        }
    }
}

/// "+2" or "−1": how real cards write a change to a number. (The minus sign
/// is the real one, not a hyphen.)
pub fn signed(n: i32) -> String {
    if n < 0 {
        format!("\u{2212}{}", -n)
    } else {
        format!("+{n}")
    }
}

/// Upper-cases the first letter, for the start of a sentence.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// "Creatures `whose` control get +`power`/+`toughness` and have `keyword`":
/// what a static ability or an emblem does to creatures, all the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boost {
    pub whose: Whose,
    pub power: i32,
    pub toughness: i32,
    pub keyword: Option<Keyword>,
}

impl Boost {
    /// Whether it does anything at all.
    pub fn is_empty(&self) -> bool {
        self.power == 0 && self.toughness == 0 && self.keyword.is_none()
    }

    /// The sentence, without a final period: "Other creatures you control
    /// get +1/+1 and have flying". With `other`, not the source itself.
    pub fn describe(&self, other: bool) -> String {
        let other = if other { "other " } else { "" };
        let subject = match self.whose {
            Whose::You => format!("{other}creatures you control"),
            Whose::Opponent => format!("{other}creatures your opponents control"),
            Whose::Anyone => format!("all {other}creatures"),
        };
        let mut parts = Vec::new();
        if self.power != 0 || self.toughness != 0 {
            parts.push(format!(
                "get {}/{}",
                signed(self.power),
                signed(self.toughness)
            ));
        }
        if let Some(k) = self.keyword {
            parts.push(format!("have {}", k.name().to_lowercase()));
        }
        capitalize(&format!("{subject} {}", parts.join(" and ")))
    }
}

/// What it takes to activate an ability: any of a loyalty change (for
/// planeswalkers), tapping the permanent ({T}) and mana.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ActivationCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loyalty: Option<i32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tap: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mana: Option<&'static str>,
}

impl ActivationCost {
    /// "+1", "−2", "{T}", "{1}{R}, {T}".
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(n) = self.loyalty {
            parts.push(if n == 0 { "0".to_string() } else { signed(n) });
        }
        if let Some(mana) = self.mana {
            parts.push(ManaCost::parse(mana).to_string());
        }
        if self.tap {
            parts.push("{T}".into());
        }
        parts.join(", ")
    }
}

/// Where cards found by a search go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    #[default]
    Hand,
    Battlefield,
    /// On top of the library, after the rest of it is shuffled.
    TopOfLibrary,
}

impl Destination {
    pub const ALL: [Destination; 3] = [
        Destination::Hand,
        Destination::Battlefield,
        Destination::TopOfLibrary,
    ];
}

/// Which cards a search can find. Each part that's set must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CardFilter<'a> {
    pub kind: Option<CardType>,
    /// A card's color; for lands, the color of mana they make.
    pub color: Option<Color>,
    /// A specific card, by name.
    pub named: Option<&'a str>,
}

impl CardFilter<'_> {
    pub fn matches(&self, def: &CardDef) -> bool {
        let color_ok = |color| match def.kind {
            CardKind::Land(made) => made == color,
            _ => def.colors().contains(&color),
        };
        self.kind.is_none_or(|k| def.card_type() == k)
            && self.color.is_none_or(color_ok)
            && self.named.is_none_or(|n| def.name == n)
    }

    /// Whether anything narrows it down: a search for one of these reveals
    /// the card it finds, as real cards say.
    pub fn is_restricted(&self) -> bool {
        self.kind.is_some() || self.color.is_some() || self.named.is_some()
    }

    /// Up to `count` of these, in words: "a card named Bog", "an instant
    /// card", "up to 2 land cards that tap for {B}".
    pub fn describe(&self, count: u32) -> String {
        let plural = count != 1;
        let cards = if plural { "cards" } else { "card" };
        let words = if let Some(name) = self.named {
            format!("{cards} named {name}")
        } else {
            match (self.kind, self.color) {
                (Some(CardType::Land), Some(color)) => {
                    let tap = if plural { "tap" } else { "taps" };
                    format!("land {cards} that {tap} for {{{}}}", color.symbol())
                }
                (kind, color) => {
                    let color = color.map(|c| format!("{} ", c.name())).unwrap_or_default();
                    let kind = kind.map(|k| format!("{} ", k.name())).unwrap_or_default();
                    format!("{color}{kind}{cards}")
                }
            }
        };
        if plural {
            format!("up to {count} {words}")
        } else if words.starts_with(['a', 'e', 'i', 'o', 'u']) {
            format!("an {words}")
        } else {
            format!("a {words}")
        }
    }
}

/// One thing a spell or ability does.
///
/// A card has at most one targeted effect; every targeted effect on a card
/// shares the card's single target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Damage {
        amount: i32,
        target: TargetKind,
        #[serde(default)]
        whose: Whose,
    },
    Destroy {
        #[serde(default)]
        whose: Whose,
    },
    Bounce {
        #[serde(default)]
        whose: Whose,
    },
    Pump {
        power: i32,
        toughness: i32,
        #[serde(default)]
        whose: Whose,
    },
    Counter {
        #[serde(default)]
        whose: Whose,
    },
    Draw {
        who: Who,
        count: u32,
    },
    GainLife {
        who: Who,
        amount: i32,
    },
    LoseLife {
        who: Who,
        amount: i32,
    },
    /// Damage to players without targeting them.
    DamagePlayers {
        who: Who,
        amount: i32,
    },
    LoseGame {
        who: Who,
    },
    /// Puts the targeted card from a graveyard onto the battlefield under the
    /// spell's controller's control. It enters, but isn't cast.
    ReturnToBattlefield {
        #[serde(default = "Whose::you")]
        whose: Whose,
    },
    /// Returns the targeted card from a graveyard to its owner's hand.
    ReturnToHand {
        #[serde(default = "Whose::you")]
        whose: Whose,
    },
    /// Exiles the targeted creature and immediately returns it under its
    /// owner's control, as a new object: spells and abilities aimed at it lose
    /// track of it, and it enters again.
    Blink {
        #[serde(default)]
        whose: Whose,
    },
    /// "You may put a creature card from your hand onto the battlefield." The
    /// player chooses while the spell or ability resolves.
    PutFromHand,
    /// "You get an emblem with …": a boost that lasts the rest of the game and
    /// can't be removed. Usually a planeswalker's last ability.
    Emblem {
        #[serde(default = "Whose::you")]
        whose: Whose,
        #[serde(default)]
        power: i32,
        #[serde(default)]
        toughness: i32,
        #[serde(default)]
        keyword: Option<Keyword>,
    },
    /// "Search your library for up to `count` cards matching the filter, put
    /// them `to` somewhere, then shuffle." The player chooses while it
    /// resolves, and may find fewer (even none).
    Search {
        #[serde(default)]
        kind: Option<CardType>,
        #[serde(default)]
        color: Option<Color>,
        /// A specific card, by name.
        #[serde(default)]
        named: Option<Cow<'static, str>>,
        #[serde(default = "one")]
        count: u32,
        #[serde(default)]
        to: Destination,
        /// For `battlefield`: the cards enter tapped.
        #[serde(default)]
        tapped: bool,
    },
}

fn one() -> u32 {
    1
}

impl Effect {
    pub fn target(&self) -> Option<TargetSpec> {
        let (kind, whose) = match *self {
            Effect::Damage { target, whose, .. } => (target, whose),
            Effect::Destroy { whose }
            | Effect::Bounce { whose }
            | Effect::Pump { whose, .. }
            | Effect::Blink { whose } => (TargetKind::Creature, whose),
            Effect::ReturnToBattlefield { whose } | Effect::ReturnToHand { whose } => {
                (TargetKind::CreatureCardInGraveyard, whose)
            }
            Effect::Counter { whose } => (TargetKind::Spell, whose),
            Effect::Draw { .. }
            | Effect::GainLife { .. }
            | Effect::LoseLife { .. }
            | Effect::DamagePlayers { .. }
            | Effect::LoseGame { .. }
            | Effect::PutFromHand
            | Effect::Search { .. }
            | Effect::Emblem { .. } => return None,
        };
        Some(TargetSpec { kind, whose })
    }

    /// One effect of each kind, with typical values: the vocabulary a card
    /// editor offers. A test fails if a new kind of effect is missing here.
    pub fn examples() -> Vec<Effect> {
        let anyone = Whose::Anyone;
        vec![
            Effect::Damage {
                amount: 2,
                target: TargetKind::Any,
                whose: anyone,
            },
            Effect::Destroy { whose: anyone },
            Effect::Bounce { whose: anyone },
            Effect::Pump {
                power: 2,
                toughness: 2,
                whose: anyone,
            },
            Effect::Counter { whose: anyone },
            Effect::Draw {
                who: Who::You,
                count: 1,
            },
            Effect::GainLife {
                who: Who::You,
                amount: 2,
            },
            Effect::LoseLife {
                who: Who::EachOpponent,
                amount: 2,
            },
            Effect::DamagePlayers {
                who: Who::EachOpponent,
                amount: 1,
            },
            Effect::LoseGame {
                who: Who::ThatPlayer,
            },
            Effect::ReturnToBattlefield { whose: Whose::You },
            Effect::ReturnToHand { whose: Whose::You },
            Effect::Blink { whose: Whose::You },
            Effect::PutFromHand,
            Effect::Search {
                kind: Some(CardType::Land),
                color: None,
                named: None,
                count: 1,
                to: Destination::Hand,
                tapped: false,
            },
            Effect::Emblem {
                whose: Whose::You,
                power: 1,
                toughness: 1,
                keyword: None,
            },
        ]
    }

    /// What an [`Effect::Emblem`] does.
    pub fn emblem(&self) -> Option<Boost> {
        match *self {
            Effect::Emblem {
                whose,
                power,
                toughness,
                keyword,
            } => Some(Boost {
                whose,
                power,
                toughness,
                keyword,
            }),
            _ => None,
        }
    }

    /// What a [`Effect::Search`] looks for.
    pub fn search_filter(&self) -> Option<CardFilter<'_>> {
        match self {
            Effect::Search {
                kind, color, named, ..
            } => Some(CardFilter {
                kind: *kind,
                color: *color,
                named: named.as_deref(),
            }),
            _ => None,
        }
    }

    /// The players this effect applies to, if it names them with a [`Who`].
    pub fn who(&self) -> Option<Who> {
        match *self {
            Effect::Draw { who, .. }
            | Effect::GainLife { who, .. }
            | Effect::LoseLife { who, .. }
            | Effect::DamagePlayers { who, .. }
            | Effect::LoseGame { who } => Some(who),
            Effect::Damage { .. }
            | Effect::Destroy { .. }
            | Effect::Bounce { .. }
            | Effect::Pump { .. }
            | Effect::Counter { .. }
            | Effect::ReturnToBattlefield { .. }
            | Effect::ReturnToHand { .. }
            | Effect::Blink { .. }
            | Effect::PutFromHand
            | Effect::Search { .. }
            | Effect::Emblem { .. } => None,
        }
    }

    pub fn describe(&self) -> String {
        let target = self.target().map(TargetSpec::describe).unwrap_or_default();
        match *self {
            Effect::Damage { amount, .. } => format!("deal {amount} damage to {target}"),
            Effect::Destroy { .. } => format!("destroy {target}"),
            Effect::Bounce { .. } => format!("return {target} to its owner's hand"),
            Effect::Pump {
                power, toughness, ..
            } => format!("{target} gets +{power}/+{toughness} until end of turn"),
            Effect::Counter { .. } => format!("counter {target}"),
            // "You draw a card" reads as "draw a card" on real cards.
            Effect::Draw { who, count } => {
                let cards = match count {
                    1 => "a card".to_string(),
                    n => format!("{n} cards"),
                };
                match who {
                    Who::You => format!("draw {cards}"),
                    _ => format!("{} {} {cards}", who.noun(), who.conjugate("draw")),
                }
            }
            Effect::GainLife { who, amount } => {
                format!("{} {} {amount} life", who.noun(), who.conjugate("gain"))
            }
            Effect::LoseLife { who, amount } => {
                format!("{} {} {amount} life", who.noun(), who.conjugate("lose"))
            }
            Effect::DamagePlayers { who, amount } => {
                format!("deal {amount} damage to {}", who.noun())
            }
            Effect::LoseGame { who } => {
                format!("{} {} the game", who.noun(), who.conjugate("lose"))
            }
            Effect::ReturnToBattlefield { whose } => match whose {
                Whose::You => format!("return {target} to the battlefield"),
                _ => format!("put {target} onto the battlefield under your control"),
            },
            Effect::ReturnToHand { whose } => match whose {
                Whose::You => format!("return {target} to your hand"),
                _ => format!("return {target} to its owner's hand"),
            },
            Effect::Blink { .. } => format!(
                "exile {target}, then return it to the battlefield under its owner's control"
            ),
            Effect::PutFromHand => {
                "you may put a creature card from your hand onto the battlefield".into()
            }
            Effect::Emblem { .. } => format!(
                "you get an emblem with \"{}.\"",
                self.emblem().expect("an emblem").describe(false)
            ),
            Effect::Search {
                count, to, tapped, ..
            } => {
                let what = self.search_filter().expect("a search").describe(count);
                let them = if count == 1 { "it" } else { "them" };
                match to {
                    Destination::Hand => {
                        format!(
                            "search your library for {what}, put {them} into your hand, then shuffle"
                        )
                    }
                    Destination::Battlefield => format!(
                        "search your library for {what}, put {them} onto the battlefield{}, then shuffle",
                        if tapped { " tapped" } else { "" }
                    ),
                    Destination::TopOfLibrary => format!(
                        "search your library for {what}, then shuffle and put {them} on top"
                    ),
                }
            }
        }
    }
}

/// The event that makes a triggered ability go on the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// This permanent enters the battlefield.
    Enters,
    /// This creature deals combat damage to a player, who becomes
    /// [`Who::ThatPlayer`].
    DealsCombatDamageToPlayer,
}

impl Trigger {
    pub const ALL: [Trigger; 2] = [Trigger::Enters, Trigger::DealsCombatDamageToPlayer];

    /// The start of the ability's sentence, e.g. "When this creature enters".
    pub fn describe(self) -> &'static str {
        match self {
            Trigger::Enters => "When this creature enters",
            Trigger::DealsCombatDamageToPlayer => {
                "Whenever this creature deals combat damage to a player"
            }
        }
    }

    /// Whether the triggering event involves a player, so that effects can
    /// refer to [`Who::ThatPlayer`].
    pub fn has_player(self) -> bool {
        match self {
            Trigger::Enters => false,
            Trigger::DealsCombatDamageToPlayer => true,
        }
    }
}

/// Where a spell was cast from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CastZone {
    Hand,
    Graveyard,
}

impl CastZone {
    pub const ALL: [CastZone; 2] = [CastZone::Hand, CastZone::Graveyard];

    pub fn describe(self) -> &'static str {
        match self {
            CastZone::Hand => "your hand",
            CastZone::Graveyard => "your graveyard",
        }
    }
}

/// An "if" on a triggered ability. It's checked when the ability triggers,
/// and again when it resolves: if it's false either time, nothing happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    /// This permanent was cast from `zone`.
    CastFrom { zone: CastZone },
    /// This permanent wasn't cast from `zone`, including when it wasn't cast at
    /// all (put onto the battlefield by an effect).
    NotCastFrom { zone: CastZone },
}

impl Condition {
    pub fn describe(self) -> String {
        match self {
            Condition::CastFrom { zone } => format!("if you cast it from {}", zone.describe()),
            Condition::NotCastFrom { zone } => {
                format!("if you didn't cast it from {}", zone.describe())
            }
        }
    }
}

/// "As you cast this spell, you may `action`. If you do, this spell costs
/// `reduction` less to cast." The action is a search for one card; when
/// casting, each different card it could find is offered as its own way to
/// cast, so nothing has to be chosen partway through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CastOption {
    pub action: Effect,
    pub reduction: &'static str,
}

impl CastOption {
    pub fn describe(&self) -> String {
        format!(
            "As you cast this spell, you may {}. If you do, this spell costs {} less to cast.",
            self.action.describe(),
            ManaCost::parse(self.reduction)
        )
    }
}

/// Something a permanent does beyond its keywords.
///
/// Only serializable: card packs deserialize into the owned `AbilitySpec`
/// (see `pool.rs`) because these slices are `&'static`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ability {
    /// "When `when`, if `only_if`, do `effects`." Goes on the stack when its
    /// event happens.
    Triggered {
        when: Trigger,
        #[serde(skip_serializing_if = "Option::is_none")]
        only_if: Option<Condition>,
        effects: &'static [Effect],
    },
    /// "`cost`: `effects`." Its controller may pay the cost to put it on the
    /// stack, like casting a spell. It may have one targeted effect.
    /// Loyalty abilities (with a loyalty cost) belong to planeswalkers: one
    /// per planeswalker per turn, when a sorcery could be cast.
    Activated {
        cost: ActivationCost,
        effects: &'static [Effect],
    },
    /// Applies all the time while this permanent is on the battlefield:
    /// creatures `whose` control get +`power`/+`toughness` and `keyword`;
    /// with `other`, not this permanent itself.
    Static {
        whose: Whose,
        other: bool,
        power: i32,
        toughness: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        keyword: Option<Keyword>,
    },
}

impl Ability {
    pub fn describe(&self) -> String {
        match self {
            Ability::Triggered {
                when,
                only_if,
                effects,
            } => {
                let condition = only_if
                    .map(|c| format!(", {}", c.describe()))
                    .unwrap_or_default();
                format!(
                    "{}{condition}, {}.",
                    when.describe(),
                    describe_effects(effects)
                )
            }
            Ability::Activated { cost, effects } => format!(
                "{}: {}.",
                cost.describe(),
                capitalize(&describe_effects(effects))
            ),
            Ability::Static { other, .. } => {
                format!("{}.", self.boost().expect("static").describe(*other))
            }
        }
    }

    /// What the ability does when it resolves (none for static abilities).
    pub fn effects(&self) -> &'static [Effect] {
        match *self {
            Ability::Triggered { effects, .. } | Ability::Activated { effects, .. } => effects,
            Ability::Static { .. } => &[],
        }
    }

    /// What a static ability does to creatures.
    pub fn boost(&self) -> Option<Boost> {
        match *self {
            Ability::Static {
                whose,
                power,
                toughness,
                keyword,
                ..
            } => Some(Boost {
                whose,
                power,
                toughness,
                keyword,
            }),
            _ => None,
        }
    }

    /// What activating it targets, if anything.
    pub fn target(&self) -> Option<TargetSpec> {
        match self {
            Ability::Activated { effects, .. } => effects.iter().find_map(Effect::target),
            _ => None,
        }
    }
}

/// Joins effects into one clause: "deal 1 damage to each opponent, then draw a card".
fn describe_effects(effects: &[Effect]) -> String {
    let parts: Vec<_> = effects.iter().map(Effect::describe).collect();
    parts.join(", then ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Land(Color),
    Creature {
        power: i32,
        toughness: i32,
    },
    Instant,
    Sorcery,
    /// Enters with this many loyalty counters.
    Planeswalker {
        loyalty: i32,
    },
}

/// The printed card: everything that's the same about every copy of it.
#[derive(Debug)]
pub struct CardDef {
    pub key: &'static str,
    pub name: &'static str,
    /// Compact mana notation, e.g. `"2RR"`.
    pub cost: &'static str,
    pub kind: CardKind,
    pub subtype: &'static str,
    pub keywords: &'static [Keyword],
    /// What an instant or sorcery does when it resolves. Empty for permanents.
    pub effects: &'static [Effect],
    /// Triggered abilities of a permanent. Empty for instants and sorceries.
    pub abilities: &'static [Ability],
    /// For instants and sorceries: a cost for casting this card from its
    /// owner's graveyard, after which it's exiled instead of going back.
    pub flashback: Option<&'static str>,
    /// Flavor text: shown in italics under the rules text. No effect on play.
    pub flavor: Option<&'static str>,
    /// Optional things to do while casting this card that make it cheaper.
    pub cast_options: &'static [CastOption],
    /// Legendary: a player can't control two with the same name (the legend
    /// rule). Planeswalkers always are.
    pub legendary: bool,
}

impl CardDef {
    pub fn mana_cost(&self) -> ManaCost {
        ManaCost::parse(self.cost)
    }

    pub fn colors(&self) -> Vec<Color> {
        self.mana_cost().colors()
    }

    pub fn card_type(&self) -> CardType {
        match self.kind {
            CardKind::Land(_) => CardType::Land,
            CardKind::Creature { .. } => CardType::Creature,
            CardKind::Instant => CardType::Instant,
            CardKind::Sorcery => CardType::Sorcery,
            CardKind::Planeswalker { .. } => CardType::Planeswalker,
        }
    }

    pub fn is_planeswalker(&self) -> bool {
        matches!(self.kind, CardKind::Planeswalker { .. })
    }

    /// Whether it stays on the battlefield (everything but instants and
    /// sorceries).
    pub fn is_permanent(&self) -> bool {
        !matches!(self.kind, CardKind::Instant | CardKind::Sorcery)
    }

    pub fn is_legendary(&self) -> bool {
        self.legendary || self.is_planeswalker()
    }

    /// A planeswalker's starting loyalty.
    pub fn loyalty(&self) -> Option<i32> {
        match self.kind {
            CardKind::Planeswalker { loyalty } => Some(loyalty),
            _ => None,
        }
    }

    pub fn is_land(&self) -> bool {
        matches!(self.kind, CardKind::Land(_))
    }

    pub fn is_creature(&self) -> bool {
        matches!(self.kind, CardKind::Creature { .. })
    }

    pub fn is_instant(&self) -> bool {
        matches!(self.kind, CardKind::Instant)
    }

    pub fn power(&self) -> Option<i32> {
        match self.kind {
            CardKind::Creature { power, .. } => Some(power),
            _ => None,
        }
    }

    pub fn toughness(&self) -> Option<i32> {
        match self.kind {
            CardKind::Creature { toughness, .. } => Some(toughness),
            _ => None,
        }
    }

    pub fn has(&self, keyword: Keyword) -> bool {
        self.keywords.contains(&keyword)
    }

    /// What casting this card targets, if anything. (Permanents don't target
    /// when cast; their abilities may.)
    pub fn spell_target(&self) -> Option<TargetSpec> {
        match self.kind {
            CardKind::Instant | CardKind::Sorcery => self.effects.iter().find_map(Effect::target),
            CardKind::Land(_) | CardKind::Creature { .. } | CardKind::Planeswalker { .. } => None,
        }
    }

    /// Everything wrong with this definition, as readable sentences; empty if
    /// it's well-formed. Built-in cards and cards loaded from packs are held
    /// to the same rules. (Duplicate keys are the pool's job to catch.)
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.key.is_empty() || self.key.chars().any(char::is_whitespace) {
            problems.push("the key must be non-empty and contain no spaces".into());
        }
        if self.name.trim().is_empty() {
            problems.push("the name is empty".into());
        }
        if let Err(e) = ManaCost::try_parse(self.cost) {
            problems.push(e);
        }
        if let Some(cost) = self.flashback {
            if let Err(e) = ManaCost::try_parse(cost) {
                problems.push(format!("flashback: {e}"));
            }
            if !matches!(self.kind, CardKind::Instant | CardKind::Sorcery) {
                problems.push("only instants and sorceries can have flashback".into());
            }
        }
        let targeted = self.effects.iter().filter(|e| e.target().is_some()).count();
        match self.kind {
            CardKind::Land(_) => {
                if !self.cost.is_empty() {
                    problems.push("lands have no mana cost".into());
                }
                if !(self.keywords.is_empty()
                    && self.effects.is_empty()
                    && self.abilities.is_empty())
                {
                    problems.push("lands can't have keywords, effects or abilities".into());
                }
            }
            CardKind::Creature { power, toughness } => {
                if power < 0 || toughness < 1 {
                    problems.push(format!(
                        "a creature can't be {power}/{toughness}: power must be at least 0 \
                         and toughness at least 1"
                    ));
                }
                if !self.effects.is_empty() {
                    problems.push(
                        "creatures use abilities (e.g. an \"enters\" trigger), not effects".into(),
                    );
                }
            }
            CardKind::Planeswalker { loyalty } => {
                if loyalty < 1 {
                    problems.push("a planeswalker needs a starting loyalty of at least 1".into());
                }
                if !self.effects.is_empty() || !self.keywords.is_empty() {
                    problems.push(
                        "planeswalkers use loyalty abilities, not effects or keywords".into(),
                    );
                }
                let loyalty_abilities = self
                    .abilities
                    .iter()
                    .filter(
                        |a| matches!(a, Ability::Activated { cost, .. } if cost.loyalty.is_some()),
                    )
                    .count();
                if loyalty_abilities == 0 {
                    problems.push("a planeswalker needs at least one loyalty ability".into());
                }
            }
            CardKind::Instant | CardKind::Sorcery => {
                if self.effects.is_empty() {
                    problems.push("a spell needs at least one effect".into());
                }
                if targeted > 1 {
                    problems.push("a spell can have at most one targeted effect".into());
                }
                if !self.keywords.is_empty() || !self.abilities.is_empty() {
                    problems.push("only permanents have keywords and abilities".into());
                }
            }
        }
        let damages_non_creature = self.effects.iter().any(|e| {
            matches!(
                e,
                Effect::Damage {
                    target: TargetKind::Spell | TargetKind::CreatureCardInGraveyard,
                    ..
                }
            )
        });
        if damages_non_creature {
            problems.push("damage can only target `any`, `creature` or `player`".into());
        }
        let targets_yourself = self.effects.iter().filter_map(Effect::target).any(|t| {
            t.whose == Whose::You && matches!(t.kind, TargetKind::Any | TargetKind::Player)
        });
        if targets_yourself {
            problems.push(
                "`whose = \"you\"` can't apply to a player target (use an untargeted effect)"
                    .into(),
            );
        }
        if self
            .effects
            .iter()
            .any(|e| e.who() == Some(Who::ThatPlayer))
        {
            problems.push("\"that_player\" only works in an ability triggered by a player".into());
        }
        for (i, option) in self.cast_options.iter().enumerate() {
            let n = i + 1;
            match &option.action {
                Effect::Search { count: 1, .. } => {}
                Effect::Search { .. } => {
                    problems.push(format!("cast option {n} must search for exactly one card"))
                }
                _ => problems.push(format!("cast option {n}'s action must be a search")),
            }
            match ManaCost::try_parse(option.reduction) {
                Ok(cost) if cost.mana_value() > 0 => {}
                Ok(_) => problems.push(format!("cast option {n} doesn't reduce the cost")),
                Err(e) => problems.push(format!("cast option {n}: {e}")),
            }
            if self.is_land() {
                problems.push("lands aren't cast, so they can't have cast options".into());
            }
        }
        if self
            .effects
            .iter()
            .any(|e| e.emblem().is_some_and(|b| b.is_empty()))
        {
            problems.push("an emblem needs a boost or a keyword".into());
        }
        for (i, ability) in self.abilities.iter().enumerate() {
            let n = i + 1;
            let effects = ability.effects();
            if effects.iter().any(|e| e.who() == Some(Who::ThatPlayer))
                && !matches!(ability, Ability::Triggered { when, .. } if when.has_player())
            {
                problems.push(format!(
                    "ability {n} uses \"that_player\", but nothing it reacts to involves a player"
                ));
            }
            if effects
                .iter()
                .any(|e| e.emblem().is_some_and(|b| b.is_empty()))
            {
                problems.push(format!("ability {n}: an emblem needs a boost or a keyword"));
            }
            match ability {
                Ability::Triggered { effects, .. } => {
                    if effects.is_empty() {
                        problems.push(format!("ability {n} has no effects"));
                    }
                    if effects.iter().any(|e| e.target().is_some()) {
                        problems.push(format!(
                            "ability {n} has a targeted effect, but triggered abilities can't \
                             target yet"
                        ));
                    }
                }
                Ability::Activated { cost, effects } => {
                    if effects.is_empty() {
                        problems.push(format!("ability {n} has no effects"));
                    }
                    if effects.iter().filter(|e| e.target().is_some()).count() > 1 {
                        problems.push(format!("ability {n} has more than one targeted effect"));
                    }
                    if let Some(Err(e)) = cost.mana.map(ManaCost::try_parse) {
                        problems.push(format!("ability {n}: {e}"));
                    }
                    match (self.is_planeswalker(), cost.loyalty.is_some()) {
                        (true, true) if cost.tap || cost.mana.is_some() => problems
                            .push(format!("ability {n}: a loyalty ability costs only loyalty")),
                        (false, true) => problems.push(format!(
                            "ability {n}: only planeswalkers have loyalty abilities"
                        )),
                        (_, false) if !cost.tap && cost.mana.is_none() => problems.push(format!(
                            "ability {n} has no cost: give it loyalty, {{T}} or mana"
                        )),
                        _ => {}
                    }
                }
                Ability::Static { .. } => {
                    if ability.boost().is_some_and(|b| b.is_empty()) {
                        problems.push(format!("ability {n} doesn't change anything"));
                    }
                }
            }
        }
        if self.legendary && !self.is_permanent() {
            problems.push("only permanents can be legendary".into());
        }
        problems
    }

    pub fn type_line(&self) -> String {
        let base = match self.kind {
            CardKind::Land(_) => "Basic Land",
            CardKind::Creature { .. } => "Creature",
            CardKind::Instant => "Instant",
            CardKind::Sorcery => "Sorcery",
            CardKind::Planeswalker { .. } => "Planeswalker",
        };
        let base = if self.is_legendary() {
            format!("Legendary {base}")
        } else {
            base.to_string()
        };
        if self.subtype.is_empty() {
            base.to_string()
        } else {
            format!("{base} — {}", self.subtype)
        }
    }

    pub fn rules_text(&self) -> String {
        let mut lines = Vec::new();
        if let CardKind::Land(color) = self.kind {
            lines.push(format!("Tap: Add {{{}}}.", color.symbol()));
        }
        if !self.keywords.is_empty() {
            let names: Vec<_> = self.keywords.iter().map(|k| k.name()).collect();
            lines.push(names.join(", "));
        }
        if !self.effects.is_empty() {
            let body = describe_effects(self.effects);
            let mut chars = body.chars();
            let first = chars
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            lines.push(format!("{first}{}.", chars.as_str()));
        }
        lines.extend(self.cast_options.iter().map(CastOption::describe));
        lines.extend(self.abilities.iter().map(Ability::describe));
        if let Some(cost) = self.flashback {
            lines.push(format!(
                "Flashback {} (You may cast this card from your graveyard for its flashback cost. \
                 Then exile it.)",
                ManaCost::parse(cost)
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Pins the data format cards will be loaded from, so changing it is a
    /// deliberate decision rather than a side effect of renaming something.
    #[test]
    fn serialized_shape() {
        let ability = Ability::Triggered {
            when: Trigger::Enters,
            only_if: Some(Condition::NotCastFrom {
                zone: CastZone::Hand,
            }),
            effects: &[
                Effect::DamagePlayers {
                    who: Who::EachOpponent,
                    amount: 2,
                },
                Effect::Destroy {
                    whose: Whose::Opponent,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(ability).unwrap(),
            json!({
                "type": "triggered",
                "when": "enters",
                "only_if": { "type": "not_cast_from", "zone": "hand" },
                "effects": [
                    { "type": "damage_players", "who": "each_opponent", "amount": 2 },
                    { "type": "destroy", "whose": "opponent" },
                ],
            })
        );
        // `whose` may be left out. Returning from a graveyard defaults to yours.
        let effects: Vec<Effect> = serde_json::from_value(json!([
            { "type": "destroy" },
            { "type": "return_to_hand" },
        ]))
        .unwrap();
        assert_eq!(
            effects,
            [
                Effect::Destroy {
                    whose: Whose::Anyone
                },
                Effect::ReturnToHand { whose: Whose::You },
            ]
        );
        let kind: CardKind =
            serde_json::from_value(json!({ "creature": { "power": 2, "toughness": 3 } })).unwrap();
        assert_eq!(
            kind,
            CardKind::Creature {
                power: 2,
                toughness: 3
            }
        );
    }

    /// Every kind of effect has an example, so card editors offer all of them.
    #[test]
    fn examples_cover_every_effect() {
        // Adding an `Effect` variant makes this match fail to compile; add
        // the variant here and an example to `Effect::examples`.
        fn kind(e: &Effect) -> usize {
            match e {
                Effect::Damage { .. } => 0,
                Effect::Destroy { .. } => 1,
                Effect::Bounce { .. } => 2,
                Effect::Pump { .. } => 3,
                Effect::Counter { .. } => 4,
                Effect::Draw { .. } => 5,
                Effect::GainLife { .. } => 6,
                Effect::LoseLife { .. } => 7,
                Effect::DamagePlayers { .. } => 8,
                Effect::LoseGame { .. } => 9,
                Effect::ReturnToBattlefield { .. } => 10,
                Effect::ReturnToHand { .. } => 11,
                Effect::Blink { .. } => 12,
                Effect::PutFromHand => 13,
                Effect::Search { .. } => 14,
                Effect::Emblem { .. } => 15,
            }
        }
        let mut kinds: Vec<usize> = Effect::examples().iter().map(kind).collect();
        kinds.sort();
        assert_eq!(kinds, (0..16).collect::<Vec<_>>());
    }

    /// The `ALL` lists name every value, for the same reason.
    #[test]
    fn all_lists_are_complete() {
        for k in Keyword::ALL {
            match k {
                Keyword::Flying
                | Keyword::Reach
                | Keyword::Haste
                | Keyword::Vigilance
                | Keyword::Lifelink
                | Keyword::Deathtouch
                | Keyword::Defender => {}
            }
        }
        let unique = |n: usize, names: Vec<String>| {
            let set: std::collections::BTreeSet<_> = names.iter().collect();
            assert_eq!(set.len(), n, "{names:?}");
        };
        let name = |v: serde_json::Value| v.to_string();
        unique(7, Keyword::ALL.map(|k| name(json!(k))).to_vec());
        unique(3, Who::ALL.map(|w| name(json!(w))).to_vec());
        unique(3, Whose::ALL.map(|w| name(json!(w))).to_vec());
        unique(2, Trigger::ALL.map(|t| name(json!(t))).to_vec());
        unique(2, CastZone::ALL.map(|z| name(json!(z))).to_vec());
        // Matching on each value fails to compile when a variant is added.
        for w in Who::ALL {
            let (Who::You | Who::EachOpponent | Who::ThatPlayer) = w;
        }
        for w in Whose::ALL {
            let (Whose::Anyone | Whose::You | Whose::Opponent) = w;
        }
        for t in Trigger::ALL {
            let (Trigger::Enters | Trigger::DealsCombatDamageToPlayer) = t;
        }
        for z in CastZone::ALL {
            let (CastZone::Hand | CastZone::Graveyard) = z;
        }
    }

    #[test]
    fn card_filters_read_naturally() {
        let filter = |kind, color, named| CardFilter { kind, color, named };
        let land = Some(CardType::Land);
        assert_eq!(filter(None, None, None).describe(1), "a card");
        assert_eq!(
            filter(Some(CardType::Instant), None, None).describe(1),
            "an instant card"
        );
        assert_eq!(
            filter(Some(CardType::Creature), Some(Color::Green), None).describe(2),
            "up to 2 green creature cards"
        );
        assert_eq!(
            filter(land, Some(Color::Black), None).describe(1),
            "a land card that taps for {B}"
        );
        assert_eq!(
            filter(land, Some(Color::Blue), None).describe(3),
            "up to 3 land cards that tap for {U}"
        );
        assert_eq!(
            filter(None, None, Some("Bog")).describe(1),
            "a card named Bog"
        );
    }

    #[test]
    fn effects_round_trip() {
        let effects = [
            Effect::Damage {
                amount: 3,
                target: TargetKind::Any,
                whose: Whose::Anyone,
            },
            Effect::Draw {
                who: Who::You,
                count: 2,
            },
            Effect::Counter {
                whose: Whose::Opponent,
            },
        ];
        let text = serde_json::to_string(&effects).unwrap();
        let back: Vec<Effect> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, effects);
    }
}
