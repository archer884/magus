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
}

/// Which player(s) an untargeted effect applies to, from the point of view of
/// whoever controls the spell or ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    You,
    EachOpponent,
}

impl Who {
    /// The subject of a sentence, e.g. "you" or "each opponent".
    fn noun(self) -> &'static str {
        match self {
            Who::You => "you",
            Who::EachOpponent => "each opponent",
        }
    }

    /// `verb` conjugated to agree with [`Who::noun`]: "you gain" but "each
    /// opponent gains".
    fn conjugate(self, verb: &str) -> String {
        match self {
            Who::You => verb.to_string(),
            Who::EachOpponent => format!("{verb}s"),
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
    },
    Destroy,
    Bounce,
    Pump {
        power: i32,
        toughness: i32,
    },
    Counter,
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
}

impl Effect {
    pub fn target(&self) -> Option<TargetKind> {
        match self {
            Effect::Damage { target, .. } => Some(*target),
            Effect::Destroy | Effect::Bounce | Effect::Pump { .. } => Some(TargetKind::Creature),
            Effect::Counter => Some(TargetKind::Spell),
            Effect::Draw { .. }
            | Effect::GainLife { .. }
            | Effect::LoseLife { .. }
            | Effect::DamagePlayers { .. } => None,
        }
    }

    pub fn describe(&self) -> String {
        match *self {
            Effect::Damage { amount, target } => {
                let what = match target {
                    TargetKind::Any => "any target",
                    TargetKind::Creature => "target creature",
                    TargetKind::Player => "target player",
                    TargetKind::Spell => "target spell",
                };
                format!("deal {amount} damage to {what}")
            }
            Effect::Destroy => "destroy target creature".into(),
            Effect::Bounce => "return target creature to its owner's hand".into(),
            Effect::Pump { power, toughness } => {
                format!("target creature gets +{power}/+{toughness} until end of turn")
            }
            Effect::Counter => "counter target spell".into(),
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
        }
    }
}

/// The event that makes a triggered ability go on the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// This permanent enters the battlefield.
    Enters,
}

impl Trigger {
    /// The start of the ability's sentence, e.g. "When this creature enters".
    fn describe(self) -> &'static str {
        match self {
            Trigger::Enters => "When this creature enters",
        }
    }
}

/// Something a permanent does beyond its keywords.
///
/// Only serializable for now: deserializing needs owned card data rather than
/// `&'static` slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ability {
    /// "When `when`, do `effects`." Goes on the stack when its event happens.
    Triggered {
        when: Trigger,
        effects: &'static [Effect],
    },
}

impl Ability {
    pub fn describe(&self) -> String {
        match self {
            Ability::Triggered { when, effects } => {
                format!("{}, {}.", when.describe(), describe_effects(effects))
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
    pub fn spell_target(&self) -> Option<TargetKind> {
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
        for (i, ability) in self.abilities.iter().enumerate() {
            let Ability::Triggered { effects, .. } = ability;
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
            effects: &[
                Effect::DamagePlayers {
                    who: Who::EachOpponent,
                    amount: 2,
                },
                Effect::Destroy,
            ],
        };
        assert_eq!(
            serde_json::to_value(ability).unwrap(),
            json!({
                "type": "triggered",
                "when": "enters",
                "effects": [
                    { "type": "damage_players", "who": "each_opponent", "amount": 2 },
                    { "type": "destroy" },
                ],
            })
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
            },
            Effect::Draw {
                who: Who::You,
                count: 2,
            },
            Effect::Counter,
        ];
        let text = serde_json::to_string(&effects).unwrap();
        let back: Vec<Effect> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, effects);
    }
}
