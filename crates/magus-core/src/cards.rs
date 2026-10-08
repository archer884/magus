//! The card pool and the preconstructed decks.

use crate::card::CardKind::{Instant, Sorcery};
use crate::card::Effect::*;
use crate::card::Keyword::*;
use crate::card::Who::{EachOpponent, You};
use crate::card::{Ability, CardDef, CardKind, Effect, Keyword, TargetKind, Trigger};
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
    }
}

/// "When this creature enters, do `effects`."
const fn enters(effects: &'static [Effect]) -> Ability {
    Ability::Triggered {
        when: Trigger::Enters,
        effects,
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
        }],
    ),
    spell("judgment-ray", "Judgment Ray", "3W", Sorcery, &[Destroy]),
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
        &[Counter],
    ),
    spell("undertow", "Undertow", "1U", Instant, &[Bounce]),
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
    spell("grasp-of-ruin", "Grasp of Ruin", "1BB", Instant, &[Destroy]),
    spell(
        "siphon-essence",
        "Siphon Essence",
        "2B",
        Sorcery,
        &[
            Damage {
                amount: 2,
                target: ANY,
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
    spell(
        "flicker-bolt",
        "Flicker Bolt",
        "R",
        Instant,
        &[Damage {
            amount: 2,
            target: ANY,
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
        self.cards.iter().map(|(_, n)| n).sum()
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
            ("grovekin", 4),
            ("forgeheart-brute", 4),
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
            ("skyward-kestrel", 4),
            ("fen-stalker", 4),
            ("night-leech", 4),
            ("cove-scholar", 4),
            ("mistwing-drake", 4),
            ("wailing-shade", 4),
            ("dissolve-thought", 4),
            ("undertow", 4),
            ("grasp-of-ruin", 4),
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
            ("rally-cry", 4),
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
            ("flicker-bolt", 4),
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
