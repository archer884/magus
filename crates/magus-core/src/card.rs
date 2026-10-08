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
    fn you() -> Whose {
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

/// One thing a spell or ability does.
///
/// A card has at most one targeted effect; every targeted effect on a card
/// shares the card's single target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
            | Effect::PutFromHand => return None,
        };
        Some(TargetSpec { kind, whose })
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
            | Effect::PutFromHand => None,
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
    /// The start of the ability's sentence, e.g. "When this creature enters".
    fn describe(self) -> &'static str {
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
    fn describe(self) -> &'static str {
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
    fn describe(self) -> String {
        match self {
            Condition::CastFrom { zone } => format!("if you cast it from {}", zone.describe()),
            Condition::NotCastFrom { zone } => {
                format!("if you didn't cast it from {}", zone.describe())
            }
        }
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
    Creature { power: i32, toughness: i32 },
    Instant,
    Sorcery,
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
}

impl CardDef {
    pub fn mana_cost(&self) -> ManaCost {
        ManaCost::parse(self.cost)
    }

    pub fn colors(&self) -> Vec<Color> {
        self.mana_cost().colors()
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

    /// What casting this card targets, if anything. Abilities never target.
    pub fn spell_target(&self) -> Option<TargetSpec> {
        match self.kind {
            CardKind::Instant | CardKind::Sorcery => self.effects.iter().find_map(Effect::target),
            CardKind::Land(_) | CardKind::Creature { .. } => None,
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
        for (i, ability) in self.abilities.iter().enumerate() {
            let Ability::Triggered { when, effects, .. } = ability;
            if !when.has_player() && effects.iter().any(|e| e.who() == Some(Who::ThatPlayer)) {
                problems.push(format!(
                    "ability {} uses \"that_player\", but its trigger doesn't involve a player",
                    i + 1
                ));
            }
            if effects.is_empty() {
                problems.push(format!("ability {} has no effects", i + 1));
            }
            if effects.iter().any(|e| e.target().is_some()) {
                problems.push(format!(
                    "ability {} has a targeted effect, but triggered abilities can't target yet",
                    i + 1
                ));
            }
        }
        problems
    }

    pub fn type_line(&self) -> String {
        let base = match self.kind {
            CardKind::Land(_) => "Basic Land",
            CardKind::Creature { .. } => "Creature",
            CardKind::Instant => "Instant",
            CardKind::Sorcery => "Sorcery",
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
