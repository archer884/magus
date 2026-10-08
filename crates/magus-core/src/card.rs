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

/// One thing a spell (or a creature's enters-the-battlefield ability) does.
///
/// A card has at most one targeted effect; every targeted effect on a card
/// shares the card's single target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Damage { amount: i32, target: TargetKind },
    Destroy,
    Bounce,
    Pump { power: i32, toughness: i32 },
    Counter,
    Draw(u32),
    GainLife(i32),
    LoseLife(i32),
    DamageEachOpponent(i32),
}

impl Effect {
    pub fn target(&self) -> Option<TargetKind> {
        match self {
            Effect::Damage { target, .. } => Some(*target),
            Effect::Destroy | Effect::Bounce | Effect::Pump { .. } => Some(TargetKind::Creature),
            Effect::Counter => Some(TargetKind::Spell),
            Effect::Draw(_)
            | Effect::GainLife(_)
            | Effect::LoseLife(_)
            | Effect::DamageEachOpponent(_) => None,
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
            Effect::Draw(1) => "draw a card".into(),
            Effect::Draw(n) => format!("draw {n} cards"),
            Effect::GainLife(n) => format!("you gain {n} life"),
            Effect::LoseLife(n) => format!("you lose {n} life"),
            Effect::DamageEachOpponent(n) => format!("deal {n} damage to each opponent"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// For instants and sorceries, what the spell does. For creatures, what
    /// happens when the creature enters the battlefield.
    pub effects: &'static [Effect],
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

    /// What casting this card targets, if anything. Creature abilities never target.
    pub fn spell_target(&self) -> Option<TargetKind> {
        match self.kind {
            CardKind::Instant | CardKind::Sorcery => self.effects.iter().find_map(Effect::target),
            CardKind::Land(_) | CardKind::Creature { .. } => None,
        }
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
            let body: Vec<_> = self.effects.iter().map(Effect::describe).collect();
            let body = body.join(", then ");
            if self.is_creature() {
                lines.push(format!("When this creature enters, {body}."));
            } else {
                let mut chars = body.chars();
                let first = chars
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                lines.push(format!("{first}{}.", chars.as_str()));
            }
        }
        lines.join("\n")
    }
}
