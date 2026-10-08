//! The card pool and the preconstructed decks.

use crate::card::CardKind::{Instant, Sorcery};
use crate::card::Effect::*;
use crate::card::Keyword::*;
use crate::card::Who::{EachOpponent, You};
use crate::card::{
    Ability, CardDef, CardKind, CastZone, Condition, Effect, Keyword, TargetKind, Trigger, Whose,
};
use crate::mana::Color;

const fn land(key: &'static str, name: &'static str, color: Color) -> CardDef {
    CardDef {
        key,
        name,
        cost: "",
        kind: CardKind::Land(color),
        subtype: "",
        keywords: &[],
        effects: &[],
        abilities: &[],
        flashback: None,
    }
}

#[allow(clippy::too_many_arguments)]
const fn creature(
    key: &'static str,
    name: &'static str,
    cost: &'static str,
    subtype: &'static str,
    power: i32,
    toughness: i32,
    keywords: &'static [Keyword],
    abilities: &'static [Ability],
) -> CardDef {
    CardDef {
        key,
        name,
        cost,
        kind: CardKind::Creature { power, toughness },
        subtype,
        keywords,
        effects: &[],
        abilities,
        flashback: None,
    }
}

const fn spell(
    key: &'static str,
    name: &'static str,
    cost: &'static str,
    kind: CardKind,
    effects: &'static [Effect],
) -> CardDef {
    CardDef {
        key,
        name,
        cost,
        kind,
        subtype: "",
        keywords: &[],
        effects,
        abilities: &[],
        flashback: None,
    }
}

/// "When this creature enters, do `effects`."
const fn enters(effects: &'static [Effect]) -> Ability {
    Ability::Triggered {
        when: Trigger::Enters,
        only_if: None,
        effects,
    }
}

/// `card`, which can also be cast from the graveyard for `cost`.
const fn flashback(card: CardDef, cost: &'static str) -> CardDef {
    CardDef {
        flashback: Some(cost),
        ..card
    }
}

const ANY: TargetKind = TargetKind::Any;

pub static CARDS: &[CardDef] = &[
    land("meadow", "Meadow", Color::White),
    land("lagoon", "Lagoon", Color::Blue),
    land("bog", "Bog", Color::Black),
    land("crag", "Crag", Color::Red),
    land("thicket", "Thicket", Color::Green),
    // White: small soldiers, fliers, life
    creature(
        "hearthguard-recruit",
        "Hearthguard Recruit",
        "W",
        "Soldier",
        1,
        2,
        &[Vigilance],
        &[],
    ),
    creature(
        "lantern-pilgrim",
        "Lantern Pilgrim",
        "1W",
        "Cleric",
        2,
        2,
        &[],
        &[enters(&[GainLife {
            who: You,
            amount: 2,
        }])],
    ),
    creature(
        "skyreach-griffin",
        "Skyreach Griffin",
        "2W",
        "Griffin",
        2,
        2,
        &[Flying],
        &[],
    ),
    creature(
        "bastion-warden",
        "Bastion Warden",
        "3W",
        "Soldier",
        3,
        4,
        &[Vigilance],
        &[],
    ),
    creature(
        "sunlit-seraph",
        "Sunlit Seraph",
        "3WW",
        "Angel",
        3,
        3,
        &[Flying, Lifelink],
        &[],
    ),
    spell(
        "mending-light",
        "Mending Light",
        "W",
        Instant,
        &[GainLife {
            who: You,
            amount: 4,
        }],
    ),
    spell(
        "rally-cry",
        "Rally Cry",
        "W",
        Instant,
        &[Pump {
            power: 2,
            toughness: 2,
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "judgment-ray",
        "Judgment Ray",
        "3W",
        Sorcery,
        &[Destroy {
            whose: Whose::Anyone,
        }],
    ),
    // Saves a creature from removal (the spell loses track of it) or reuses
    // its "enters" ability.
    spell(
        "veilstep",
        "Veilstep",
        "1W",
        Instant,
        &[Blink { whose: Whose::You }],
    ),
    // Blue: fliers, card draw, tricks
    creature(
        "skyward-kestrel",
        "Skyward Kestrel",
        "U",
        "Bird",
        1,
        1,
        &[Flying],
        &[],
    ),
    creature(
        "reef-sentinel",
        "Reef Sentinel",
        "1U",
        "Wall",
        1,
        4,
        &[Defender],
        &[],
    ),
    creature(
        "cove-scholar",
        "Cove Scholar",
        "2U",
        "Wizard",
        1,
        3,
        &[],
        &[enters(&[Draw { who: You, count: 1 }])],
    ),
    creature(
        "mistwing-drake",
        "Mistwing Drake",
        "2U",
        "Drake",
        2,
        2,
        &[Flying],
        &[],
    ),
    creature(
        "stormcrest-leviathan",
        "Stormcrest Leviathan",
        "5UU",
        "Leviathan",
        5,
        5,
        &[Flying],
        &[],
    ),
    spell(
        "dissolve-thought",
        "Dissolve Thought",
        "UU",
        Instant,
        &[Counter {
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "undertow",
        "Undertow",
        "1U",
        Instant,
        &[Bounce {
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "glimpse-beyond",
        "Glimpse Beyond",
        "2U",
        Instant,
        &[Draw { who: You, count: 2 }],
    ),
    // Black: removal, drain, costs paid in life
    creature(
        "fen-stalker",
        "Fen Stalker",
        "B",
        "Rat",
        1,
        1,
        &[Deathtouch],
        &[],
    ),
    creature(
        "night-leech",
        "Night Leech",
        "1B",
        "Leech",
        2,
        2,
        &[Lifelink],
        &[],
    ),
    creature(
        "wailing-shade",
        "Wailing Shade",
        "3B",
        "Spirit",
        3,
        3,
        &[],
        &[enters(&[DamagePlayers {
            who: EachOpponent,
            amount: 2,
        }])],
    ),
    creature(
        "bone-colossus",
        "Bone Colossus",
        "4BB",
        "Skeleton",
        6,
        5,
        &[],
        &[],
    ),
    spell(
        "grasp-of-ruin",
        "Grasp of Ruin",
        "1BB",
        Instant,
        &[Destroy {
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "call-from-the-mire",
        "Call from the Mire",
        "3B",
        Sorcery,
        &[ReturnToBattlefield { whose: Whose::You }],
    ),
    // Weak to cast, strong to bring back with Call from the Mire.
    creature(
        "gravecall-wraith",
        "Gravecall Wraith",
        "3B",
        "Spirit",
        3,
        2,
        &[],
        &[Ability::Triggered {
            when: Trigger::Enters,
            only_if: Some(Condition::NotCastFrom {
                zone: CastZone::Hand,
            }),
            effects: &[Draw { who: You, count: 2 }],
        }],
    ),
    spell(
        "siphon-essence",
        "Siphon Essence",
        "2B",
        Sorcery,
        &[
            Damage {
                amount: 2,
                target: ANY,
                whose: Whose::Anyone,
            },
            GainLife {
                who: You,
                amount: 2,
            },
        ],
    ),
    spell(
        "dark-bargain",
        "Dark Bargain",
        "1B",
        Sorcery,
        &[
            Draw { who: You, count: 2 },
            LoseLife {
                who: You,
                amount: 2,
            },
        ],
    ),
    // Red: haste, burn
    creature(
        "emberkin-scout",
        "Emberkin Scout",
        "R",
        "Goblin",
        1,
        1,
        &[Haste],
        &[],
    ),
    creature(
        "cinderhound",
        "Cinderhound",
        "1R",
        "Hound",
        2,
        1,
        &[Haste],
        &[],
    ),
    creature(
        "forgeheart-brute",
        "Forgeheart Brute",
        "2R",
        "Warrior",
        3,
        2,
        &[],
        &[],
    ),
    creature(
        "volcanic-ogre",
        "Volcanic Ogre",
        "3R",
        "Ogre",
        4,
        3,
        &[],
        &[enters(&[DamagePlayers {
            who: EachOpponent,
            amount: 1,
        }])],
    ),
    creature(
        "ashen-wyrm",
        "Ashen Wyrm",
        "4RR",
        "Dragon",
        5,
        4,
        &[Flying, Haste],
        &[],
    ),
    // A second burn spell later in the game, from the graveyard.
    flashback(
        spell(
            "sparkfall",
            "Sparkfall",
            "1R",
            Instant,
            &[Damage {
                amount: 2,
                target: ANY,
                whose: Whose::Anyone,
            }],
        ),
        "2R",
    ),
    spell(
        "flicker-bolt",
        "Flicker Bolt",
        "R",
        Instant,
        &[Damage {
            amount: 2,
            target: ANY,
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "blaze",
        "Blaze",
        "1R",
        Instant,
        &[Damage {
            amount: 3,
            target: ANY,
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "rage-surge",
        "Rage Surge",
        "R",
        Instant,
        &[Pump {
            power: 3,
            toughness: 0,
            whose: Whose::Anyone,
        }],
    ),
    // Green: big bodies
    creature("mossling", "Mossling", "G", "Elemental", 1, 2, &[], &[]),
    creature("grovekin", "Grovekin", "1G", "Bear", 2, 2, &[], &[]),
    creature(
        "thornback-boar",
        "Thornback Boar",
        "2G",
        "Boar",
        3,
        3,
        &[],
        &[],
    ),
    creature(
        "canopy-spider",
        "Canopy Spider",
        "2G",
        "Spider",
        2,
        4,
        &[Reach],
        &[],
    ),
    creature(
        "ironbark-behemoth",
        "Ironbark Behemoth",
        "3GG",
        "Beast",
        5,
        5,
        &[],
        &[],
    ),
    creature(
        "elderwood-titan",
        "Elderwood Titan",
        "4GG",
        "Treefolk",
        6,
        6,
        &[Vigilance],
        &[],
    ),
    spell(
        "wild-growth",
        "Wild Growth",
        "G",
        Instant,
        &[Pump {
            power: 3,
            toughness: 3,
            whose: Whose::Anyone,
        }],
    ),
    spell(
        "verdant-bounty",
        "Verdant Bounty",
        "1G",
        Sorcery,
        &[
            GainLife {
                who: You,
                amount: 3,
            },
            Draw { who: You, count: 1 },
        ],
    ),
    // Cheats a big creature in early, or brings in one that rewards not
    // being cast (Gravecall Wraith).
    spell(
        "beckon-the-wild",
        "Beckon the Wild",
        "2G",
        Sorcery,
        &[PutFromHand],
    ),
    spell(
        "mossgrave-recovery",
        "Mossgrave Recovery",
        "1G",
        Sorcery,
        &[ReturnToHand { whose: Whose::You }],
    ),
];

pub fn card(key: &str) -> Option<&'static CardDef> {
    CARDS.iter().find(|c| c.key == key)
}

#[derive(Debug)]
pub struct DeckList {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub cards: &'static [(&'static str, u32)],
}

impl DeckList {
    pub fn size(&self) -> u32 {
        self.cards
            .iter()
            .fold(0, |sum, (_, n)| sum.saturating_add(*n))
    }
}

pub static DECKS: &[DeckList] = &[
    DeckList {
        key: "ember-thorn",
        name: "Ember & Thorn",
        description: "Red/green aggro: hasty attackers, big beasts, burn to finish.",
        cards: &[
            ("crag", 12),
            ("thicket", 12),
            ("emberkin-scout", 4),
            ("cinderhound", 4),
            ("grovekin", 2),
            ("mossgrave-recovery", 2),
            ("forgeheart-brute", 2),
            ("beckon-the-wild", 2),
            ("thornback-boar", 4),
            ("canopy-spider", 4),
            ("ironbark-behemoth", 4),
            ("flicker-bolt", 4),
            ("blaze", 4),
        ],
    },
    DeckList {
        key: "tide-ash",
        name: "Tide & Ash",
        description: "Blue/black control: counter, kill, and fly over the top.",
        cards: &[
            ("lagoon", 12),
            ("bog", 12),
            ("fen-stalker", 4),
            ("night-leech", 4),
            ("cove-scholar", 4),
            ("mistwing-drake", 4),
            ("wailing-shade", 4),
            ("dissolve-thought", 4),
            ("undertow", 4),
            ("grasp-of-ruin", 4),
            ("gravecall-wraith", 2),
            ("call-from-the-mire", 2),
        ],
    },
    DeckList {
        key: "dawn-grove",
        name: "Dawn & Grove",
        description: "White/green midrange: sturdy creatures and combat tricks.",
        cards: &[
            ("meadow", 12),
            ("thicket", 12),
            ("hearthguard-recruit", 4),
            ("lantern-pilgrim", 4),
            ("grovekin", 4),
            ("skyreach-griffin", 4),
            ("thornback-boar", 4),
            ("bastion-warden", 4),
            ("sunlit-seraph", 4),
            ("rally-cry", 2),
            ("veilstep", 2),
            ("judgment-ray", 4),
        ],
    },
    DeckList {
        key: "storm-cinder",
        name: "Storm & Cinder",
        description: "Blue/red tempo: cheap fliers, burn, and card draw.",
        cards: &[
            ("lagoon", 12),
            ("crag", 12),
            ("skyward-kestrel", 4),
            ("emberkin-scout", 4),
            ("cinderhound", 4),
            ("mistwing-drake", 4),
            ("volcanic-ogre", 4),
            ("ashen-wyrm", 4),
            ("flicker-bolt", 2),
            ("sparkfall", 2),
            ("glimpse-beyond", 4),
            ("dissolve-thought", 4),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The rest of the pool's invariants are checked by `CardPool::problems`
    /// (see `pool.rs`), which can't see duplicates once keys are in a map.
    #[test]
    fn card_and_deck_keys_are_unique() {
        for (i, def) in CARDS.iter().enumerate() {
            assert!(
                CARDS[..i].iter().all(|c| c.key != def.key),
                "duplicate card key {}",
                def.key
            );
        }
        for (i, deck) in DECKS.iter().enumerate() {
            assert!(
                DECKS[..i].iter().all(|d| d.key != deck.key),
                "duplicate deck key {}",
                deck.key
            );
        }
    }
}
