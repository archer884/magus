//! The rules engine. A `Game` is driven entirely by `apply`; after every
//! accepted action the engine advances on its own until some player has a real
//! decision to make, automatically passing priority for anyone with nothing to do.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::card::{
    Ability, CardDef, CardKind, CastZone, Condition, Effect, Keyword, TargetKind, TargetSpec,
    Trigger, Who, Whose,
};
use crate::mana::{Color, ManaCost, plan_payment};
use crate::pool::CardPool;
use crate::view::{
    AttackOption, BlockOption, CardView, GameView, PermanentView, PlayKind, PlayOption, PlayerView,
    Prompt, StackItemView,
};

pub type PlayerId = usize;

pub const STARTING_LIFE: i32 = 20;
pub const STARTING_HAND: usize = 7;
pub const MAX_HAND_SIZE: usize = 7;
const VIEW_LOG_LINES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    Player(PlayerId),
    Permanent(ObjectId),
    Spell(ObjectId),
    /// A card in a graveyard.
    GraveyardCard(ObjectId),
}

impl Target {
    /// The object targeted, if it isn't a player.
    pub fn object(self) -> Option<ObjectId> {
        match self {
            Target::Player(_) => None,
            Target::Permanent(id) | Target::Spell(id) | Target::GraveyardCard(id) => Some(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attack {
    pub attacker: ObjectId,
    pub defender: PlayerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub blocker: ObjectId,
    pub attacker: ObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Pass,
    PlayLand {
        card: ObjectId,
    },
    Cast {
        card: ObjectId,
        target: Option<Target>,
    },
    DeclareAttackers {
        attacks: Vec<Attack>,
    },
    DeclareBlockers {
        blocks: Vec<Block>,
    },
    Discard {
        cards: Vec<ObjectId>,
    },
    /// Answers [`Prompt::ChooseCard`]: one of its options, or `None` to choose
    /// nothing when that's allowed.
    ChooseCard {
        card: Option<ObjectId>,
    },
    Concede,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Untap,
    Upkeep,
    Draw,
    Main1,
    DeclareAttackers,
    DeclareBlockers,
    CombatDamage,
    Main2,
    End,
    Cleanup,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Untap => "Untap",
            Step::Upkeep => "Upkeep",
            Step::Draw => "Draw",
            Step::Main1 => "Main 1",
            Step::DeclareAttackers => "Declare attackers",
            Step::DeclareBlockers => "Declare blockers",
            Step::CombatDamage => "Combat damage",
            Step::Main2 => "Main 2",
            Step::End => "End",
            Step::Cleanup => "Cleanup",
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionError(pub String);

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ActionError {}

fn reject<T>(msg: impl Into<String>) -> Result<T, ActionError> {
    Err(ActionError(msg.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Library,
    Hand,
    Battlefield,
    Graveyard,
    Stack,
    /// Removed from the game. Public, like the graveyard.
    Exile,
}

/// One physical card, wherever it currently is.
#[derive(Debug, Clone)]
struct Object {
    def: &'static CardDef,
    owner: PlayerId,
    controller: PlayerId,
    zone: Zone,
    tapped: bool,
    /// Came under its controller's control this turn.
    sick: bool,
    damage: i32,
    deathtouched: bool,
    power_mod: i32,
    toughness_mod: i32,
    /// Where this permanent was cast from, if it was cast. Cleared when it
    /// moves anywhere but from the stack to the battlefield.
    cast_from: Option<CastZone>,
    /// How many times this card has changed zones. In the real rules a card
    /// that changes zones becomes a new object; we keep its id, so a spell
    /// remembers this count for its target and loses track of the target if
    /// it changes (see `StackItem::target_moves`).
    moves: u32,
}

impl Object {
    fn is_creature(&self) -> bool {
        self.def.is_creature()
    }

    fn has(&self, keyword: Keyword) -> bool {
        self.def.has(keyword)
    }

    fn power(&self) -> i32 {
        self.def.power().unwrap_or(0) + self.power_mod
    }

    fn toughness(&self) -> i32 {
        self.def.toughness().unwrap_or(0) + self.toughness_mod
    }
}

#[derive(Debug, Clone)]
struct Player {
    name: String,
    life: i32,
    /// The top of the library is the end of the vector.
    library: Vec<ObjectId>,
    hand: Vec<ObjectId>,
    graveyard: Vec<ObjectId>,
    exile: Vec<ObjectId>,
    lost: bool,
    drew_from_empty: bool,
}

#[derive(Debug, Clone)]
struct StackItem {
    /// For spells, the card's own id; abilities get a fresh id.
    id: ObjectId,
    source: ObjectId,
    controller: PlayerId,
    kind: StackKind,
    target: Option<Target>,
    /// For a triggered ability, the player in its event ([`Who::ThatPlayer`]).
    that_player: Option<PlayerId>,
    /// If the target is an object, its `moves` when it was targeted.
    target_moves: Option<u32>,
}

impl StackItem {
    fn is_spell(&self) -> bool {
        self.kind == StackKind::Spell
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackKind {
    Spell,
    /// A triggered ability: an index into the source card's `abilities`.
    Ability(usize),
}

/// A triggered ability waiting to go on the stack.
#[derive(Debug, Clone, Copy)]
struct Triggered {
    source: ObjectId,
    controller: PlayerId,
    ability: usize,
    that_player: Option<PlayerId>,
}

/// Something that happened which abilities can trigger on.
#[derive(Debug, Clone, Copy)]
enum Event {
    Entered(ObjectId),
    CombatDamageToPlayer { source: ObjectId, player: PlayerId },
}

#[derive(Debug, Clone, Default)]
struct Combat {
    attacks: Vec<Attack>,
    blocks: Vec<Block>,
    /// Attackers that were blocked, even if their blockers have since left.
    blocked: HashSet<ObjectId>,
    /// Defending players who still need to declare blockers.
    to_declare: VecDeque<PlayerId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Priority(PlayerId),
    Attackers(PlayerId),
    Blockers(PlayerId),
    Discard(PlayerId, usize),
    /// Choosing a card partway through resolving something (see `Choosing`).
    Choose(PlayerId),
    GameOver(Option<PlayerId>),
}

/// A spell or ability paused in the middle of resolving, waiting for its
/// controller to choose a card. It stays on the stack until it finishes.
#[derive(Debug, Clone)]
struct Choosing {
    item: StackItem,
    /// The effect that asked, and so where to carry on after the choice.
    effect: usize,
    options: Vec<ObjectId>,
}

#[derive(Debug, Clone)]
pub struct Game {
    players: Vec<Player>,
    objects: BTreeMap<ObjectId, Object>,
    battlefield: Vec<ObjectId>,
    stack: Vec<StackItem>,
    turn: u32,
    active: PlayerId,
    step: Step,
    pending: Pending,
    /// Consecutive passes with no action in between.
    passes: usize,
    land_played: bool,
    combat: Combat,
    /// Abilities that have triggered but aren't on the stack yet. They go on
    /// the next time a player would get priority (see `settle`).
    triggered: Vec<Triggered>,
    choosing: Option<Choosing>,
    next_id: u32,
    version: u64,
    rng: StdRng,
    log: Vec<String>,
}

impl Game {
    /// Starts a game. Each seat is a player name and a decklist of (card key,
    /// copies) whose cards are in `pool`; the first player is chosen at random
    /// and skips their first draw.
    pub fn new(pool: &CardPool, seats: &[(String, &[(&str, u32)])], seed: u64) -> Game {
        assert!(seats.len() >= 2, "a game needs at least two players");
        let mut game = Game {
            players: Vec::new(),
            objects: BTreeMap::new(),
            battlefield: Vec::new(),
            stack: Vec::new(),
            turn: 0,
            active: 0,
            step: Step::Untap,
            pending: Pending::Priority(0),
            passes: 0,
            land_played: false,
            combat: Combat::default(),
            triggered: Vec::new(),
            choosing: None,
            next_id: 1,
            version: 0,
            rng: StdRng::seed_from_u64(seed),
            log: Vec::new(),
        };
        for (pid, (name, deck)) in seats.iter().enumerate() {
            let mut defs: Vec<&'static CardDef> = deck
                .iter()
                .flat_map(|&(key, n)| {
                    let def = pool
                        .card(key)
                        .unwrap_or_else(|| panic!("unknown card {key}"));
                    std::iter::repeat_n(def, n as usize)
                })
                .collect();
            // Shuffle before assigning ids so an id reveals nothing about the card.
            defs.shuffle(&mut game.rng);
            let library = defs
                .into_iter()
                .map(|def| game.create(def, pid, Zone::Library))
                .collect();
            game.players.push(Player {
                name: name.clone(),
                life: STARTING_LIFE,
                library,
                hand: Vec::new(),
                graveyard: Vec::new(),
                exile: Vec::new(),
                lost: false,
                drew_from_empty: false,
            });
        }
        for pid in 0..game.players.len() {
            for _ in 0..STARTING_HAND {
                game.draw(pid);
            }
        }
        let first = game.rng.gen_range(0..game.players.len());
        game.log(format!("{} goes first.", game.players[first].name));
        game.start_turn(first);
        game.settle();
        game
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn turn(&self) -> u32 {
        self.turn
    }

    pub fn is_over(&self) -> bool {
        matches!(self.pending, Pending::GameOver(_))
    }

    /// `None` while the game is running; `Some(None)` for a draw.
    pub fn result(&self) -> Option<Option<PlayerId>> {
        match self.pending {
            Pending::GameOver(winner) => Some(winner),
            _ => None,
        }
    }

    /// The player the game is waiting on.
    pub fn waiting_on(&self) -> Option<PlayerId> {
        match self.pending {
            Pending::Priority(p)
            | Pending::Attackers(p)
            | Pending::Blockers(p)
            | Pending::Discard(p, _)
            | Pending::Choose(p) => Some(p),
            Pending::GameOver(_) => None,
        }
    }

    pub fn apply(&mut self, p: PlayerId, action: Action) -> Result<(), ActionError> {
        if p >= self.players.len() {
            return reject("no such player");
        }
        if self.is_over() {
            return reject("the game is over");
        }
        if self.players[p].lost {
            return reject("you have already lost");
        }
        match action {
            Action::Concede => self.lose(p, "concedes"),
            Action::Pass => {
                self.expect_priority(p)?;
                self.pass_priority();
            }
            Action::PlayLand { card } => {
                self.expect_priority(p)?;
                self.play_land(p, card)?;
            }
            Action::Cast { card, target } => {
                self.expect_priority(p)?;
                self.cast(p, card, target)?;
            }
            Action::DeclareAttackers { attacks } => {
                if self.pending != Pending::Attackers(p) {
                    return reject("you aren't declaring attackers");
                }
                self.declare_attackers(attacks)?;
            }
            Action::DeclareBlockers { blocks } => {
                if self.pending != Pending::Blockers(p) {
                    return reject("you aren't declaring blockers");
                }
                self.declare_blockers(p, blocks)?;
            }
            Action::Discard { cards } => {
                let Pending::Discard(q, count) = self.pending else {
                    return reject("you don't need to discard");
                };
                if q != p {
                    return reject("you don't need to discard");
                }
                self.discard(p, count, cards)?;
            }
            Action::ChooseCard { card } => {
                if self.pending != Pending::Choose(p) {
                    return reject("you aren't choosing a card");
                }
                self.choose(p, card)?;
            }
        }
        self.version += 1;
        self.settle();
        Ok(())
    }

    pub fn view(&self, p: PlayerId) -> GameView {
        GameView {
            version: self.version,
            you: p,
            players: self
                .players
                .iter()
                .enumerate()
                .map(|(id, pl)| PlayerView {
                    id,
                    name: pl.name.clone(),
                    life: pl.life,
                    hand_size: pl.hand.len(),
                    library_size: pl.library.len(),
                    graveyard: pl.graveyard.iter().map(|&c| self.card_view(c)).collect(),
                    exile: pl.exile.iter().map(|&c| self.card_view(c)).collect(),
                    lost: pl.lost,
                })
                .collect(),
            hand: self.players[p]
                .hand
                .iter()
                .map(|&c| self.card_view(c))
                .collect(),
            battlefield: self
                .battlefield
                .iter()
                .map(|&id| self.permanent_view(id))
                .collect(),
            stack: self
                .stack
                .iter()
                .map(|item| StackItemView {
                    id: item.id,
                    card: self.card_view(item.source),
                    controller: item.controller,
                    is_spell: item.is_spell(),
                    target: item.target,
                })
                .collect(),
            turn: self.turn,
            active: self.active,
            step: self.step,
            priority: match self.pending {
                Pending::Priority(q) => Some(q),
                _ => None,
            },
            prompt: self.prompt(p),
            log: self.log[self.log.len().saturating_sub(VIEW_LOG_LINES)..].to_vec(),
        }
    }

    fn prompt(&self, p: PlayerId) -> Prompt {
        match self.pending {
            Pending::GameOver(winner) => Prompt::GameOver { winner },
            Pending::Priority(q) if q == p => Prompt::Priority {
                plays: self.legal_plays(p),
            },
            Pending::Attackers(q) if q == p => Prompt::DeclareAttackers {
                options: self.attack_options(),
            },
            Pending::Blockers(q) if q == p => Prompt::DeclareBlockers {
                options: self.block_options(p),
            },
            Pending::Discard(q, count) if q == p => Prompt::Discard { count },
            Pending::Choose(q) if q == p => {
                let choosing = self.choosing.as_ref().expect("a choice is pending");
                let def = self.objects[&choosing.item.source].def;
                let what = self.item_effects(&choosing.item)[choosing.effect].describe();
                Prompt::ChooseCard {
                    reason: format!("{}: {what}", def.name),
                    options: choosing.options.clone(),
                    optional: true,
                }
            }
            _ => Prompt::Waiting {
                on: self.waiting_on().unwrap_or(self.active),
            },
        }
    }

    fn card_view(&self, id: ObjectId) -> CardView {
        CardView::new(id, self.objects[&id].def)
    }

    fn permanent_view(&self, id: ObjectId) -> PermanentView {
        let o = &self.objects[&id];
        PermanentView {
            card: self.card_view(id),
            controller: o.controller,
            owner: o.owner,
            tapped: o.tapped,
            summoning_sick: o.sick && o.is_creature() && !o.has(Keyword::Haste),
            damage: o.damage,
            power: o.is_creature().then(|| o.power()),
            toughness: o.is_creature().then(|| o.toughness()),
            attacking: self
                .combat
                .attacks
                .iter()
                .find(|a| a.attacker == id)
                .map(|a| a.defender),
            blocking: self
                .combat
                .blocks
                .iter()
                .find(|b| b.blocker == id)
                .map(|b| b.attacker),
        }
    }

    // ----- bookkeeping -----

    fn log(&mut self, line: String) {
        self.log.push(line);
    }

    fn name(&self, p: PlayerId) -> &str {
        &self.players[p].name
    }

    fn card_name(&self, id: ObjectId) -> &'static str {
        self.objects[&id].def.name
    }

    fn create(&mut self, def: &'static CardDef, owner: PlayerId, zone: Zone) -> ObjectId {
        let id = self.fresh_id();
        self.objects.insert(
            id,
            Object {
                def,
                owner,
                controller: owner,
                zone,
                tapped: false,
                sick: false,
                damage: 0,
                deathtouched: false,
                power_mod: 0,
                toughness_mod: 0,
                cast_from: None,
                moves: 0,
            },
        );
        id
    }

    fn fresh_id(&mut self) -> ObjectId {
        let id = ObjectId(self.next_id);
        self.next_id += 1;
        id
    }

    fn on_battlefield(&self, id: ObjectId) -> bool {
        self.objects
            .get(&id)
            .is_some_and(|o| o.zone == Zone::Battlefield)
    }

    fn living(&self) -> impl Iterator<Item = PlayerId> + '_ {
        (0..self.players.len()).filter(|&p| !self.players[p].lost)
    }

    fn next_living(&self, p: PlayerId) -> PlayerId {
        let n = self.players.len();
        (1..=n)
            .map(|i| (p + i) % n)
            .find(|&q| !self.players[q].lost)
            .unwrap_or(p)
    }

    fn move_to(&mut self, id: ObjectId, zone: Zone) {
        let (owner, from) = {
            let o = &self.objects[&id];
            (o.owner, o.zone)
        };
        let player = &mut self.players[owner];
        match from {
            Zone::Library => player.library.retain(|&x| x != id),
            Zone::Hand => player.hand.retain(|&x| x != id),
            Zone::Graveyard => player.graveyard.retain(|&x| x != id),
            Zone::Exile => player.exile.retain(|&x| x != id),
            Zone::Stack => self.stack.retain(|s| s.id != id),
            Zone::Battlefield => {
                self.battlefield.retain(|&x| x != id);
                self.combat.attacks.retain(|a| a.attacker != id);
                self.combat.blocks.retain(|b| b.blocker != id);
            }
        }
        let player = &mut self.players[owner];
        match zone {
            Zone::Library => player.library.push(id),
            Zone::Hand => player.hand.push(id),
            Zone::Graveyard => player.graveyard.push(id),
            Zone::Exile => player.exile.push(id),
            Zone::Battlefield => self.battlefield.push(id),
            Zone::Stack => {}
        }
        let o = self.objects.get_mut(&id).expect("object exists");
        o.zone = zone;
        o.tapped = false;
        o.sick = zone == Zone::Battlefield;
        o.damage = 0;
        o.deathtouched = false;
        o.power_mod = 0;
        o.toughness_mod = 0;
        if zone != Zone::Battlefield {
            o.controller = o.owner;
        }
        if !(from == Zone::Stack && zone == Zone::Battlefield) {
            o.cast_from = None;
        }
        o.moves += 1;
    }

    fn draw(&mut self, p: PlayerId) {
        match self.players[p].library.pop() {
            Some(id) => {
                self.players[p].hand.push(id);
                let o = self.objects.get_mut(&id).expect("object exists");
                o.zone = Zone::Hand;
                o.moves += 1;
            }
            None => self.players[p].drew_from_empty = true,
        }
    }

    fn untapped_lands(&self, p: PlayerId) -> Vec<(ObjectId, Color)> {
        self.battlefield
            .iter()
            .filter_map(|&id| {
                let o = &self.objects[&id];
                match o.def.kind {
                    CardKind::Land(color) if o.controller == p && !o.tapped => Some((id, color)),
                    _ => None,
                }
            })
            .collect()
    }

    // ----- turn structure -----

    fn start_turn(&mut self, p: PlayerId) {
        self.turn += 1;
        self.active = p;
        self.land_played = false;
        self.combat = Combat::default();
        self.step = Step::Untap;
        self.log(format!("── Turn {}: {} ──", self.turn, self.name(p)));
        for id in self.battlefield.clone() {
            let o = self.objects.get_mut(&id).expect("object exists");
            if o.controller == p {
                o.tapped = false;
                o.sick = false;
            }
        }
        self.enter_step(Step::Upkeep);
    }

    fn enter_step(&mut self, step: Step) {
        self.step = step;
        self.passes = 0;
        match step {
            Step::Untap => unreachable!("untap happens in start_turn"),
            Step::Upkeep | Step::Main1 | Step::End => self.give_priority(self.active),
            Step::Draw => {
                // The player who goes first skips their first draw.
                if self.turn > 1 {
                    self.draw(self.active);
                    self.log(format!("{} draws a card.", self.name(self.active)));
                }
                self.give_priority(self.active);
            }
            Step::DeclareAttackers => {
                if self.attack_options().is_empty() {
                    self.enter_step(Step::Main2);
                } else {
                    self.pending = Pending::Attackers(self.active);
                }
            }
            Step::DeclareBlockers => {
                let mut defenders = VecDeque::new();
                let mut p = self.next_living(self.active);
                while p != self.active {
                    if self.combat.attacks.iter().any(|a| a.defender == p) {
                        defenders.push_back(p);
                    }
                    p = self.next_living(p);
                }
                self.combat.to_declare = defenders;
                self.next_blocker();
            }
            Step::CombatDamage => {
                self.combat_damage();
                self.give_priority(self.active);
            }
            Step::Main2 => {
                self.combat = Combat::default();
                self.give_priority(self.active);
            }
            Step::Cleanup => {
                let excess = self.players[self.active]
                    .hand
                    .len()
                    .saturating_sub(MAX_HAND_SIZE);
                if excess > 0 {
                    self.pending = Pending::Discard(self.active, excess);
                } else {
                    self.finish_turn();
                }
            }
        }
    }

    fn advance_step(&mut self) {
        let next = match self.step {
            Step::Upkeep => Step::Draw,
            Step::Draw => Step::Main1,
            Step::Main1 => Step::DeclareAttackers,
            Step::DeclareAttackers => Step::DeclareBlockers,
            Step::DeclareBlockers => Step::CombatDamage,
            Step::CombatDamage => Step::Main2,
            Step::Main2 => Step::End,
            Step::End => Step::Cleanup,
            Step::Untap | Step::Cleanup => unreachable!("no priority during {}", self.step),
        };
        self.enter_step(next);
    }

    fn finish_turn(&mut self) {
        for id in self.battlefield.clone() {
            let o = self.objects.get_mut(&id).expect("object exists");
            o.damage = 0;
            o.deathtouched = false;
            o.power_mod = 0;
            o.toughness_mod = 0;
        }
        let next = self.next_living(self.active);
        self.start_turn(next);
    }

    fn give_priority(&mut self, p: PlayerId) {
        self.pending = Pending::Priority(p);
    }

    fn expect_priority(&self, p: PlayerId) -> Result<(), ActionError> {
        if self.pending == Pending::Priority(p) {
            Ok(())
        } else {
            reject("you don't have priority")
        }
    }

    fn pass_priority(&mut self) {
        let Pending::Priority(p) = self.pending else {
            return;
        };
        self.passes += 1;
        if self.passes >= self.living().count() {
            self.passes = 0;
            // The item stays on the stack while it resolves, in case it
            // stops to ask for a choice.
            match self.stack.last().cloned() {
                Some(item) => {
                    let id = item.id;
                    self.resolve(item);
                    if self.choosing.is_none() {
                        self.finish_resolving(id);
                    }
                }
                None => self.advance_step(),
            }
        } else {
            self.give_priority(self.next_living(p));
        }
    }

    /// Applies state-based actions and skips past decisions nobody needs to make.
    fn settle(&mut self) {
        for _ in 0..100_000 {
            self.check_state();
            if self.is_over() {
                return;
            }
            if self.players[self.active].lost {
                let next = self.next_living(self.active);
                self.stack.clear();
                self.triggered.clear();
                self.start_turn(next);
                continue;
            }
            // Abilities that trigger while something is resolving wait for it
            // to finish.
            if self.choosing.is_none() && self.stack_triggers() {
                continue;
            }
            match self.pending {
                Pending::Priority(p) if self.players[p].lost || self.legal_plays(p).is_empty() => {
                    self.pass_priority()
                }
                Pending::Blockers(p) if self.players[p].lost => self.next_blocker(),
                Pending::Choose(p) if self.players[p].lost => {
                    // Its controller left the game, and their spell with them.
                    let choosing = self.choosing.take().expect("a choice is pending");
                    self.finish_resolving(choosing.item.id);
                }
                _ => return,
            }
        }
        panic!("game failed to settle");
    }

    fn check_state(&mut self) {
        if self.is_over() {
            return;
        }
        loop {
            let dying: Vec<ObjectId> = self
                .battlefield
                .iter()
                .copied()
                .filter(|id| {
                    let o = &self.objects[id];
                    o.is_creature()
                        && (o.toughness() <= 0 || o.damage >= o.toughness() || o.deathtouched)
                })
                .collect();
            let losing: Vec<(PlayerId, &str)> = self
                .living()
                .filter_map(|p| {
                    let pl = &self.players[p];
                    if pl.drew_from_empty {
                        Some((p, "loses (drew from an empty library)"))
                    } else if pl.life <= 0 {
                        Some((p, "loses (life reached 0)"))
                    } else {
                        None
                    }
                })
                .collect();
            if dying.is_empty() && losing.is_empty() {
                break;
            }
            for id in dying {
                self.log(format!("{} dies.", self.card_name(id)));
                self.move_to(id, Zone::Graveyard);
            }
            for (p, why) in losing {
                self.lose(p, why);
            }
        }
        let living: Vec<PlayerId> = self.living().collect();
        if living.len() <= 1 {
            let winner = living.first().copied();
            match winner {
                Some(w) => self.log(format!("{} wins the game!", self.name(w))),
                None => self.log("The game is a draw.".into()),
            }
            self.pending = Pending::GameOver(winner);
        }
    }

    fn lose(&mut self, p: PlayerId, why: &str) {
        self.players[p].lost = true;
        self.log(format!("{} {why}.", self.name(p)));
        if self.living().count() > 1 {
            // In a multiplayer game everything the player owned leaves with them.
            for id in self.battlefield.clone() {
                let o = &self.objects[&id];
                if o.owner == p || o.controller == p {
                    self.move_to(id, Zone::Graveyard);
                }
            }
            self.stack.retain(|s| s.controller != p);
            self.triggered.retain(|s| s.controller != p);
            self.combat.attacks.retain(|a| a.defender != p);
            self.combat.to_declare.retain(|&d| d != p);
        }
    }

    // ----- playing cards -----

    fn legal_plays(&self, p: PlayerId) -> Vec<PlayOption> {
        if self.pending != Pending::Priority(p) {
            return Vec::new();
        }
        let sorcery_speed = p == self.active
            && matches!(self.step, Step::Main1 | Step::Main2)
            && self.stack.is_empty();
        let lands = self.untapped_lands(p);
        let mut plays = Vec::new();
        let with_flashback = self.players[p]
            .graveyard
            .iter()
            .filter(|id| self.objects[id].def.flashback.is_some());
        for &id in self.players[p].hand.iter().chain(with_flashback) {
            let def = self.objects[&id].def;
            if def.is_land() {
                if sorcery_speed && !self.land_played {
                    plays.push(PlayOption {
                        card: id,
                        kind: PlayKind::Land,
                        targets: None,
                    });
                }
                continue;
            }
            if !def.is_instant() && !sorcery_speed {
                continue;
            }
            if plan_payment(&self.cast_cost(id), &lands).is_none() {
                continue;
            }
            let targets = match def.spell_target() {
                None => None,
                Some(kind) => {
                    let targets = self.valid_targets(kind, p);
                    if targets.is_empty() {
                        continue;
                    }
                    Some(targets)
                }
            };
            plays.push(PlayOption {
                card: id,
                kind: PlayKind::Spell,
                targets,
            });
        }
        plays
    }

    /// Everything a spell controlled by `caster` that targets `spec` could target.
    fn valid_targets(&self, spec: TargetSpec, caster: PlayerId) -> Vec<Target> {
        // "Whose" means the player itself, the controller of a permanent or
        // spell, or the owner of a graveyard.
        let allowed = |p: PlayerId| match spec.whose {
            Whose::Anyone => true,
            Whose::You => p == caster,
            Whose::Opponent => p != caster,
        };
        let kind = spec.kind;
        let mut targets = Vec::new();
        if matches!(kind, TargetKind::Any | TargetKind::Player) {
            targets.extend(self.living().filter(|&p| allowed(p)).map(Target::Player));
        }
        if matches!(kind, TargetKind::Any | TargetKind::Creature) {
            targets.extend(
                self.battlefield
                    .iter()
                    .filter(|id| self.objects[id].is_creature())
                    .filter(|id| allowed(self.objects[id].controller))
                    .map(|&id| Target::Permanent(id)),
            );
        }
        if kind == TargetKind::Spell {
            targets.extend(
                self.stack
                    .iter()
                    .filter(|s| s.is_spell() && allowed(s.controller))
                    .map(|s| Target::Spell(s.id)),
            );
        }
        if kind == TargetKind::CreatureCardInGraveyard {
            targets.extend(
                self.living()
                    .filter(|&p| allowed(p))
                    .flat_map(|p| &self.players[p].graveyard)
                    .filter(|id| self.objects[id].is_creature())
                    .map(|&id| Target::GraveyardCard(id)),
            );
        }
        targets
    }

    /// Whether `target`, chosen when `item` was put on the stack, is still
    /// legal: still a valid target, and (for objects) not moved since.
    fn target_still_legal(&self, item: &StackItem, spec: TargetSpec, target: Target) -> bool {
        self.valid_targets(spec, item.controller).contains(&target)
            && target.object().map(|id| self.objects[&id].moves) == item.target_moves
    }

    fn describe_target(&self, target: Target) -> String {
        match target {
            Target::Player(p) => self.name(p).to_string(),
            Target::Permanent(id) | Target::Spell(id) | Target::GraveyardCard(id) => {
                self.card_name(id).to_string()
            }
        }
    }

    /// What it costs to cast `card` from where it is now: its flashback cost
    /// from the graveyard, its mana cost otherwise.
    fn cast_cost(&self, card: ObjectId) -> ManaCost {
        let o = &self.objects[&card];
        match (o.zone, o.def.flashback) {
            (Zone::Graveyard, Some(cost)) => ManaCost::parse(cost),
            _ => o.def.mana_cost(),
        }
    }

    /// Moves a spell that's leaving the stack (resolved, fizzled or countered)
    /// to its owner's graveyard, or to exile if it was cast with flashback.
    fn spell_done(&mut self, id: ObjectId) {
        if self.objects[&id].cast_from == Some(CastZone::Graveyard) {
            self.log(format!("{} is exiled.", self.card_name(id)));
            self.move_to(id, Zone::Exile);
        } else {
            self.move_to(id, Zone::Graveyard);
        }
    }

    fn play_land(&mut self, p: PlayerId, card: ObjectId) -> Result<(), ActionError> {
        if !self
            .legal_plays(p)
            .iter()
            .any(|o| o.card == card && o.kind == PlayKind::Land)
        {
            return reject("you can't play that land now");
        }
        self.move_to(card, Zone::Battlefield);
        self.land_played = true;
        self.passes = 0;
        self.log(format!("{} plays {}.", self.name(p), self.card_name(card)));
        self.fire(Event::Entered(card));
        Ok(())
    }

    fn cast(
        &mut self,
        p: PlayerId,
        card: ObjectId,
        target: Option<Target>,
    ) -> Result<(), ActionError> {
        let plays = self.legal_plays(p);
        let Some(option) = plays
            .iter()
            .find(|o| o.card == card && o.kind == PlayKind::Spell)
        else {
            return reject("you can't cast that now");
        };
        match (&option.targets, target) {
            (None, None) => {}
            (Some(valid), Some(t)) if valid.contains(&t) => {}
            (Some(_), None) => return reject("that spell needs a target"),
            (None, Some(_)) => return reject("that spell doesn't take a target"),
            (Some(_), Some(_)) => return reject("that isn't a legal target"),
        }
        let def = self.objects[&card].def;
        let from = match self.objects[&card].zone {
            Zone::Graveyard => CastZone::Graveyard,
            _ => CastZone::Hand,
        };
        let lands = plan_payment(&self.cast_cost(card), &self.untapped_lands(p))
            .expect("affordability was checked");
        for land in lands {
            self.objects.get_mut(&land).expect("land exists").tapped = true;
        }
        self.move_to(card, Zone::Stack);
        self.objects.get_mut(&card).expect("card exists").cast_from = Some(from);
        self.stack.push(StackItem {
            id: card,
            source: card,
            controller: p,
            kind: StackKind::Spell,
            target,
            that_player: None,
            target_moves: target
                .and_then(Target::object)
                .map(|id| self.objects[&id].moves),
        });
        let aim = target
            .map(|t| format!(" targeting {}", self.describe_target(t)))
            .unwrap_or_default();
        self.log(format!("{} casts {}{aim}.", self.name(p), def.name));
        self.passes = 0;
        self.give_priority(p);
        Ok(())
    }

    fn resolve(&mut self, item: StackItem) {
        let def = self.objects[&item.source].def;
        if let (Some(target), Some(kind)) = (item.target, def.spell_target())
            && !self.target_still_legal(&item, kind, target)
        {
            self.log(format!("{} fizzles: its target is gone.", def.name));
            if item.is_spell() {
                self.spell_done(item.id);
            }
            return;
        }
        if item.is_spell() && def.is_creature() {
            self.move_to(item.id, Zone::Battlefield);
            self.objects
                .get_mut(&item.id)
                .expect("object exists")
                .controller = item.controller;
            self.log(format!("{} enters the battlefield.", def.name));
            self.fire(Event::Entered(item.id));
            return;
        }
        match item.kind {
            StackKind::Spell => {}
            StackKind::Ability(i) => {
                let Ability::Triggered { only_if, .. } = def.abilities[i];
                // Checked again on resolution. If the source has left the
                // battlefield, go by how it was when it triggered, as the
                // real rules do ("last known information").
                if let Some(condition) = only_if
                    && self.on_battlefield(item.source)
                    && !self.holds(condition, item.source)
                {
                    self.log(format!(
                        "{}'s ability does nothing: its condition no longer holds.",
                        def.name
                    ));
                    return;
                }
                self.log(format!("{}'s ability resolves.", def.name));
            }
        }
        self.apply_effects(item, 0);
    }

    /// What `item` does when it resolves.
    fn item_effects(&self, item: &StackItem) -> &'static [Effect] {
        let def = self.objects[&item.source].def;
        match item.kind {
            StackKind::Spell => def.effects,
            StackKind::Ability(i) => {
                let Ability::Triggered { effects, .. } = def.abilities[i];
                effects
            }
        }
    }

    /// Applies `item`'s effects from the `from`th on. If one needs a choice,
    /// stops and records where it got to in `self.choosing`; `choose` carries
    /// on from there.
    fn apply_effects(&mut self, item: StackItem, from: usize) {
        let effects = self.item_effects(&item);
        for (i, effect) in effects.iter().enumerate().skip(from) {
            if *effect == Effect::PutFromHand {
                let options: Vec<ObjectId> = self.players[item.controller]
                    .hand
                    .iter()
                    .copied()
                    .filter(|id| self.objects[id].is_creature())
                    .collect();
                if !options.is_empty() {
                    self.pending = Pending::Choose(item.controller);
                    self.choosing = Some(Choosing {
                        item,
                        effect: i,
                        options,
                    });
                    return;
                }
                let name = self.name(item.controller).to_string();
                self.log(format!(
                    "{name} has no creature card to put onto the battlefield."
                ));
                continue;
            }
            self.apply_effect(*effect, &item);
        }
        if item.is_spell() {
            self.spell_done(item.id);
        }
    }

    /// Takes a finished item off the stack and gives the active player priority.
    fn finish_resolving(&mut self, id: ObjectId) {
        self.stack.retain(|s| s.id != id);
        self.passes = 0;
        self.give_priority(self.active);
    }

    fn choose(&mut self, p: PlayerId, card: Option<ObjectId>) -> Result<(), ActionError> {
        let choosing = self.choosing.as_ref().expect("a choice is pending");
        if let Some(card) = card
            && !choosing.options.contains(&card)
        {
            return reject("you can't choose that card");
        }
        let Choosing { item, effect, .. } = self.choosing.take().expect("a choice is pending");
        match card {
            Some(card) => {
                self.move_to(card, Zone::Battlefield);
                self.objects.get_mut(&card).expect("card exists").controller = p;
                self.log(format!(
                    "{} puts {} onto the battlefield.",
                    self.name(p),
                    self.card_name(card)
                ));
                self.fire(Event::Entered(card));
            }
            None => self.log(format!("{} chooses nothing.", self.name(p))),
        }
        let id = item.id;
        self.apply_effects(item, effect + 1);
        if self.choosing.is_none() {
            self.finish_resolving(id);
        }
        Ok(())
    }

    /// Notes every ability that triggers on `event`. They wait in
    /// `self.triggered` until [`Game::stack_triggers`] puts them on the stack.
    fn fire(&mut self, event: Event) {
        for &id in &self.battlefield {
            let o = &self.objects[&id];
            for (i, ability) in o.def.abilities.iter().enumerate() {
                let Ability::Triggered { when, only_if, .. } = *ability;
                let that_player = match (when, event) {
                    (Trigger::Enters, Event::Entered(entered)) if entered == id => None,
                    (
                        Trigger::DealsCombatDamageToPlayer,
                        Event::CombatDamageToPlayer { source, player },
                    ) if source == id => Some(player),
                    _ => continue,
                };
                if only_if.is_some_and(|c| !self.holds(c, id)) {
                    continue;
                }
                self.triggered.push(Triggered {
                    source: id,
                    controller: o.controller,
                    ability: i,
                    that_player,
                });
            }
        }
    }

    /// Whether `condition` is true of the permanent `source`.
    fn holds(&self, condition: Condition, source: ObjectId) -> bool {
        let cast_from = self.objects[&source].cast_from;
        match condition {
            Condition::CastFrom { zone } => cast_from == Some(zone),
            Condition::NotCastFrom { zone } => cast_from != Some(zone),
        }
    }

    /// Puts waiting triggered abilities on the stack and gives the active
    /// player priority. Returns whether there were any.
    ///
    /// The active player's go on first, then each other player's in turn
    /// order, so the last player's resolve first (the real rules' "APNAP"
    /// order). Each player's own resolve in the order they triggered; real
    /// players would get to choose.
    fn stack_triggers(&mut self) -> bool {
        let mut waiting = std::mem::take(&mut self.triggered);
        waiting.retain(|t| !self.players[t.controller].lost);
        if waiting.is_empty() {
            return false;
        }
        let n = self.players.len();
        let active = self.active;
        waiting.sort_by_key(|t| (t.controller + n - active) % n);
        for group in waiting.chunk_by(|a, b| a.controller == b.controller) {
            for t in group.iter().rev() {
                let name = self.card_name(t.source);
                self.log(format!("{name}'s ability triggers."));
                let id = self.fresh_id();
                self.stack.push(StackItem {
                    id,
                    source: t.source,
                    controller: t.controller,
                    kind: StackKind::Ability(t.ability),
                    target: None,
                    that_player: t.that_player,
                    target_moves: None,
                });
            }
        }
        self.passes = 0;
        self.give_priority(self.active);
        true
    }

    fn apply_effect(&mut self, effect: Effect, item: &StackItem) {
        let (source, controller, target) = (item.source, item.controller, item.target);
        let players = |game: &Game, who| game.players_for(who, controller, item.that_player);
        match effect {
            Effect::Damage { amount, .. } => {
                if let Some(t) = target {
                    self.deal_damage(source, t, amount, false);
                }
            }
            Effect::Destroy { .. } => {
                if let Some(Target::Permanent(id)) = target {
                    self.log(format!("{} is destroyed.", self.card_name(id)));
                    self.move_to(id, Zone::Graveyard);
                }
            }
            Effect::ReturnToBattlefield { .. } => {
                if let Some(Target::GraveyardCard(id)) = target {
                    self.move_to(id, Zone::Battlefield);
                    self.objects.get_mut(&id).expect("object exists").controller = controller;
                    self.log(format!(
                        "{} returns to the battlefield under {}'s control.",
                        self.card_name(id),
                        self.name(controller)
                    ));
                    self.fire(Event::Entered(id));
                }
            }
            Effect::ReturnToHand { .. } => {
                if let Some(Target::GraveyardCard(id)) = target {
                    let owner = self.objects[&id].owner;
                    self.log(format!(
                        "{} returns to {}'s hand.",
                        self.card_name(id),
                        self.name(owner)
                    ));
                    self.move_to(id, Zone::Hand);
                }
            }
            Effect::Blink { .. } => {
                if let Some(Target::Permanent(id)) = target {
                    self.move_to(id, Zone::Battlefield);
                    let o = self.objects.get_mut(&id).expect("object exists");
                    o.controller = o.owner;
                    let name = o.def.name;
                    self.log(format!("{name} is exiled and returns to the battlefield."));
                    self.fire(Event::Entered(id));
                }
            }
            Effect::PutFromHand => unreachable!("handled by apply_effects"),
            Effect::Bounce { .. } => {
                if let Some(Target::Permanent(id)) = target {
                    let owner = self.objects[&id].owner;
                    self.log(format!(
                        "{} returns to {}'s hand.",
                        self.card_name(id),
                        self.name(owner)
                    ));
                    self.move_to(id, Zone::Hand);
                }
            }
            Effect::Pump {
                power, toughness, ..
            } => {
                if let Some(Target::Permanent(id)) = target {
                    let o = self.objects.get_mut(&id).expect("object exists");
                    o.power_mod += power;
                    o.toughness_mod += toughness;
                    let name = o.def.name;
                    self.log(format!(
                        "{name} gets +{power}/+{toughness} until end of turn."
                    ));
                }
            }
            Effect::Counter { .. } => {
                if let Some(Target::Spell(id)) = target {
                    self.log(format!("{} is countered.", self.card_name(id)));
                    self.spell_done(id);
                }
            }
            Effect::Draw { who, count } => {
                for q in players(self, who) {
                    for _ in 0..count {
                        self.draw(q);
                    }
                    let cards = if count == 1 {
                        "a card".to_string()
                    } else {
                        format!("{count} cards")
                    };
                    self.log(format!("{} draws {cards}.", self.name(q)));
                }
            }
            Effect::GainLife { who, amount } => {
                for q in players(self, who) {
                    self.players[q].life += amount;
                    self.log(format!("{} gains {amount} life.", self.name(q)));
                }
            }
            Effect::LoseLife { who, amount } => {
                for q in players(self, who) {
                    self.players[q].life -= amount;
                    self.log(format!("{} loses {amount} life.", self.name(q)));
                }
            }
            Effect::DamagePlayers { who, amount } => {
                for q in players(self, who) {
                    self.deal_damage(source, Target::Player(q), amount, false);
                }
            }
            Effect::LoseGame { who } => {
                let why = format!("loses the game ({})", self.card_name(source));
                for q in players(self, who) {
                    self.lose(q, &why);
                }
            }
        }
    }

    /// The players `who` refers to, for an effect controlled by `controller`.
    /// Players who have already lost are left out.
    fn players_for(
        &self,
        who: Who,
        controller: PlayerId,
        that_player: Option<PlayerId>,
    ) -> Vec<PlayerId> {
        let players = match who {
            Who::You => vec![controller],
            Who::EachOpponent => self.living().filter(|&q| q != controller).collect(),
            Who::ThatPlayer => that_player.into_iter().collect(),
        };
        players
            .into_iter()
            .filter(|&q| !self.players[q].lost)
            .collect()
    }

    fn deal_damage(&mut self, source: ObjectId, target: Target, amount: i32, combat: bool) {
        if amount <= 0 {
            return;
        }
        let src = &self.objects[&source];
        let (src_name, deathtouch, lifelink, src_controller) = (
            src.def.name,
            src.has(Keyword::Deathtouch),
            src.has(Keyword::Lifelink),
            src.controller,
        );
        match target {
            Target::Player(p) => {
                self.players[p].life -= amount;
                self.log(format!(
                    "{src_name} deals {amount} damage to {}.",
                    self.name(p)
                ));
                if combat {
                    self.fire(Event::CombatDamageToPlayer { source, player: p });
                }
            }
            Target::Permanent(id) => {
                if !self.on_battlefield(id) {
                    return;
                }
                let o = self.objects.get_mut(&id).expect("object exists");
                o.damage += amount;
                o.deathtouched |= deathtouch;
                let name = o.def.name;
                self.log(format!("{src_name} deals {amount} damage to {name}."));
            }
            // Validation keeps damage from targeting these.
            Target::Spell(_) | Target::GraveyardCard(_) => return,
        }
        if lifelink {
            self.players[src_controller].life += amount;
            self.log(format!(
                "{} gains {amount} life.",
                self.name(src_controller)
            ));
        }
    }

    // ----- combat -----

    fn attack_options(&self) -> Vec<AttackOption> {
        let p = self.active;
        let defenders: Vec<PlayerId> = self.living().filter(|&q| q != p).collect();
        self.battlefield
            .iter()
            .filter(|id| {
                let o = &self.objects[id];
                o.is_creature()
                    && o.controller == p
                    && !o.tapped
                    && (!o.sick || o.has(Keyword::Haste))
                    && !o.has(Keyword::Defender)
            })
            .map(|&attacker| AttackOption {
                attacker,
                defenders: defenders.clone(),
            })
            .collect()
    }

    fn can_block(&self, blocker: ObjectId, attacker: ObjectId) -> bool {
        let b = &self.objects[&blocker];
        let a = &self.objects[&attacker];
        b.is_creature()
            && !b.tapped
            && (!a.has(Keyword::Flying) || b.has(Keyword::Flying) || b.has(Keyword::Reach))
    }

    fn block_options(&self, defender: PlayerId) -> Vec<BlockOption> {
        let attackers: Vec<ObjectId> = self
            .combat
            .attacks
            .iter()
            .filter(|a| a.defender == defender && self.on_battlefield(a.attacker))
            .map(|a| a.attacker)
            .collect();
        self.battlefield
            .iter()
            .filter(|&&id| self.objects[&id].controller == defender)
            .filter_map(|&blocker| {
                let can: Vec<ObjectId> = attackers
                    .iter()
                    .copied()
                    .filter(|&a| self.can_block(blocker, a))
                    .collect();
                (!can.is_empty()).then_some(BlockOption {
                    blocker,
                    attackers: can,
                })
            })
            .collect()
    }

    fn declare_attackers(&mut self, attacks: Vec<Attack>) -> Result<(), ActionError> {
        let options = self.attack_options();
        let mut seen = HashSet::new();
        for attack in &attacks {
            let Some(option) = options.iter().find(|o| o.attacker == attack.attacker) else {
                return reject("that creature can't attack");
            };
            if !option.defenders.contains(&attack.defender) {
                return reject("you can't attack that player");
            }
            if !seen.insert(attack.attacker) {
                return reject("a creature can only attack once");
            }
        }
        let p = self.active;
        if attacks.is_empty() {
            self.log(format!("{} doesn't attack.", self.name(p)));
            self.enter_step(Step::Main2);
            return Ok(());
        }
        for attack in &attacks {
            let o = self
                .objects
                .get_mut(&attack.attacker)
                .expect("attacker exists");
            if !o.has(Keyword::Vigilance) {
                o.tapped = true;
            }
            self.log(format!(
                "{} attacks {}.",
                self.card_name(attack.attacker),
                self.name(attack.defender)
            ));
        }
        self.combat.attacks = attacks;
        self.give_priority(p);
        Ok(())
    }

    fn next_blocker(&mut self) {
        while let Some(d) = self.combat.to_declare.pop_front() {
            if !self.players[d].lost && !self.block_options(d).is_empty() {
                self.pending = Pending::Blockers(d);
                return;
            }
        }
        self.give_priority(self.active);
    }

    fn declare_blockers(&mut self, p: PlayerId, blocks: Vec<Block>) -> Result<(), ActionError> {
        let options = self.block_options(p);
        let mut seen = HashSet::new();
        for block in &blocks {
            let Some(option) = options.iter().find(|o| o.blocker == block.blocker) else {
                return reject("that creature can't block");
            };
            if !option.attackers.contains(&block.attacker) {
                return reject("that creature can't block that attacker");
            }
            if !seen.insert(block.blocker) {
                return reject("a creature can only block once");
            }
        }
        if blocks.is_empty() {
            self.log(format!("{} doesn't block.", self.name(p)));
        }
        for block in &blocks {
            self.combat.blocked.insert(block.attacker);
            self.log(format!(
                "{} blocks {}.",
                self.card_name(block.blocker),
                self.card_name(block.attacker)
            ));
        }
        self.combat.blocks.extend(blocks);
        self.next_blocker();
        Ok(())
    }

    fn combat_damage(&mut self) {
        // Work out all the damage first: combat damage is dealt simultaneously.
        let mut hits = Vec::new();
        for attack in &self.combat.attacks {
            if !self.on_battlefield(attack.attacker) {
                continue;
            }
            let attacker = &self.objects[&attack.attacker];
            if !self.combat.blocked.contains(&attack.attacker) {
                hits.push((
                    attack.attacker,
                    Target::Player(attack.defender),
                    attacker.power(),
                ));
                continue;
            }
            let blockers: Vec<ObjectId> = self
                .combat
                .blocks
                .iter()
                .filter(|b| b.attacker == attack.attacker && self.on_battlefield(b.blocker))
                .map(|b| b.blocker)
                .collect();
            // The attacker assigns lethal damage to each blocker in order, with
            // anything left over going to the last one.
            let mut remaining = attacker.power().max(0);
            for (i, &blocker) in blockers.iter().enumerate() {
                let b = &self.objects[&blocker];
                let lethal = if attacker.has(Keyword::Deathtouch) {
                    1
                } else {
                    (b.toughness() - b.damage).max(1)
                };
                let assigned = if i + 1 == blockers.len() {
                    remaining
                } else {
                    remaining.min(lethal)
                };
                remaining -= assigned;
                hits.push((attack.attacker, Target::Permanent(blocker), assigned));
                hits.push((blocker, Target::Permanent(attack.attacker), b.power()));
            }
        }
        for (source, target, amount) in hits {
            self.deal_damage(source, target, amount, true);
        }
    }

    fn discard(
        &mut self,
        p: PlayerId,
        count: usize,
        cards: Vec<ObjectId>,
    ) -> Result<(), ActionError> {
        let unique: HashSet<_> = cards.iter().collect();
        if cards.len() != count || unique.len() != count {
            return reject(format!("discard exactly {count} different card(s)"));
        }
        if !cards.iter().all(|c| self.players[p].hand.contains(c)) {
            return reject("you can only discard cards in your hand");
        }
        for card in cards {
            self.log(format!(
                "{} discards {}.",
                self.name(p),
                self.card_name(card)
            ));
            self.move_to(card, Zone::Graveyard);
        }
        self.finish_turn();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{self, DECKS};

    /// A game with empty hands and battlefields on player 0's first main phase.
    fn blank_game() -> Game {
        let seats = [
            ("Ann".to_string(), DECKS[0].cards),
            ("Bob".to_string(), DECKS[1].cards),
        ];
        let mut game = Game::new(&CardPool::builtin(), &seats, 7);
        for p in 0..2 {
            for id in std::mem::take(&mut game.players[p].hand) {
                game.objects.get_mut(&id).unwrap().zone = Zone::Library;
                game.players[p].library.push(id);
            }
        }
        for id in std::mem::take(&mut game.battlefield) {
            let owner = game.objects[&id].owner;
            game.objects.get_mut(&id).unwrap().zone = Zone::Library;
            game.players[owner].library.push(id);
        }
        game.stack.clear();
        game.turn = 2;
        game.active = 0;
        game.step = Step::Main1;
        game.land_played = false;
        game.combat = Combat::default();
        game.pending = Pending::Priority(0);
        game.passes = 0;
        game
    }

    fn put(game: &mut Game, p: PlayerId, key: &str, zone: Zone) -> ObjectId {
        let id = game.create(crate::cards::card(key).unwrap(), p, zone);
        match zone {
            Zone::Battlefield => game.battlefield.push(id),
            Zone::Hand => game.players[p].hand.push(id),
            _ => unreachable!(),
        }
        id
    }

    fn lands(game: &mut Game, p: PlayerId, key: &str, n: usize) {
        for _ in 0..n {
            put(game, p, key, Zone::Battlefield);
        }
    }

    /// Passes until `p` is asked to declare blockers or attackers, or the step changes.
    fn pass_until(game: &mut Game, done: impl Fn(&Game) -> bool) {
        for _ in 0..100 {
            if done(game) {
                return;
            }
            let p = game.waiting_on().unwrap();
            game.apply(p, Action::Pass).unwrap();
        }
        panic!("condition never reached");
    }

    #[test]
    fn burn_kills_creature_and_hits_face() {
        let mut g = blank_game();
        lands(&mut g, 0, "crag", 2);
        let bolt = put(&mut g, 0, "blaze", Zone::Hand);
        let bear = put(&mut g, 1, "night-leech", Zone::Battlefield);
        g.apply(
            0,
            Action::Cast {
                card: bolt,
                target: Some(Target::Permanent(bear)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&bear].zone, Zone::Graveyard);
        assert_eq!(g.objects[&bolt].zone, Zone::Graveyard);
    }

    #[test]
    fn counterspell_counters() {
        let mut g = blank_game();
        lands(&mut g, 0, "crag", 2);
        lands(&mut g, 1, "lagoon", 2);
        let bolt = put(&mut g, 0, "blaze", Zone::Hand);
        let counter = put(&mut g, 1, "dissolve-thought", Zone::Hand);
        g.apply(
            0,
            Action::Cast {
                card: bolt,
                target: Some(Target::Player(1)),
            },
        )
        .unwrap();
        // Ann has nothing else to do, so her priority passes automatically.
        assert_eq!(g.waiting_on(), Some(1));
        g.apply(
            1,
            Action::Cast {
                card: counter,
                target: Some(Target::Spell(bolt)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.players[1].life, STARTING_LIFE);
        assert_eq!(g.objects[&bolt].zone, Zone::Graveyard);
    }

    #[test]
    fn fizzles_when_target_leaves() {
        let mut g = blank_game();
        lands(&mut g, 0, "crag", 2);
        lands(&mut g, 1, "lagoon", 2);
        let bolt = put(&mut g, 0, "blaze", Zone::Hand);
        let drake = put(&mut g, 1, "mistwing-drake", Zone::Battlefield);
        let bounce = put(&mut g, 1, "undertow", Zone::Hand);
        g.apply(
            0,
            Action::Cast {
                card: bolt,
                target: Some(Target::Permanent(drake)),
            },
        )
        .unwrap();
        g.apply(
            1,
            Action::Cast {
                card: bounce,
                target: Some(Target::Permanent(drake)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&drake].zone, Zone::Hand);
        assert!(g.log.iter().any(|l| l.contains("fizzles")));
    }

    #[test]
    fn flying_needs_flying_or_reach_to_block() {
        let mut g = blank_game();
        let drake = put(&mut g, 0, "mistwing-drake", Zone::Battlefield);
        let bear = put(&mut g, 1, "grovekin", Zone::Battlefield);
        let spider = put(&mut g, 1, "canopy-spider", Zone::Battlefield);
        g.objects.get_mut(&drake).unwrap().sick = false;
        pass_until(&mut g, |g| g.pending == Pending::Attackers(0));
        g.apply(
            0,
            Action::DeclareAttackers {
                attacks: vec![Attack {
                    attacker: drake,
                    defender: 1,
                }],
            },
        )
        .unwrap();
        assert_eq!(g.pending, Pending::Blockers(1));
        let options = g.block_options(1);
        assert!(options.iter().any(|o| o.blocker == spider));
        assert!(!options.iter().any(|o| o.blocker == bear));
        assert!(
            g.apply(
                1,
                Action::DeclareBlockers {
                    blocks: vec![Block {
                        blocker: bear,
                        attacker: drake
                    }]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn combat_deathtouch_and_lifelink() {
        let mut g = blank_game();
        let leech = put(&mut g, 0, "night-leech", Zone::Battlefield);
        let boar = put(&mut g, 0, "thornback-boar", Zone::Battlefield);
        let stalker = put(&mut g, 1, "fen-stalker", Zone::Battlefield);
        for id in [leech, boar] {
            g.objects.get_mut(&id).unwrap().sick = false;
        }
        pass_until(&mut g, |g| g.pending == Pending::Attackers(0));
        let attacks = vec![
            Attack {
                attacker: leech,
                defender: 1,
            },
            Attack {
                attacker: boar,
                defender: 1,
            },
        ];
        g.apply(0, Action::DeclareAttackers { attacks }).unwrap();
        g.apply(
            1,
            Action::DeclareBlockers {
                blocks: vec![Block {
                    blocker: stalker,
                    attacker: boar,
                }],
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.step == Step::Main2);
        assert_eq!(
            g.objects[&boar].zone,
            Zone::Graveyard,
            "deathtouch kills the boar"
        );
        assert_eq!(g.objects[&stalker].zone, Zone::Graveyard);
        assert_eq!(g.players[1].life, STARTING_LIFE - 2);
        assert_eq!(g.players[0].life, STARTING_LIFE + 2, "lifelink");
    }

    #[test]
    fn summoning_sickness_and_haste() {
        let mut g = blank_game();
        lands(&mut g, 0, "crag", 2);
        let bear = put(&mut g, 0, "grovekin", Zone::Battlefield);
        let hound = put(&mut g, 0, "cinderhound", Zone::Hand);
        g.objects.get_mut(&bear).unwrap().sick = true;
        g.apply(
            0,
            Action::Cast {
                card: hound,
                target: None,
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.pending == Pending::Attackers(0));
        let options = g.attack_options();
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].attacker, hound);
    }

    #[test]
    fn enter_triggers_resolve_in_printed_order() {
        static TWO_TRIGGERS: CardDef = CardDef {
            key: "test-two-triggers",
            name: "Two Triggers",
            cost: "W",
            kind: CardKind::Creature {
                power: 1,
                toughness: 1,
            },
            subtype: "",
            keywords: &[],
            effects: &[],
            flashback: None,
            abilities: &[
                Ability::Triggered {
                    when: Trigger::Enters,
                    only_if: None,
                    effects: &[Effect::GainLife {
                        who: Who::You,
                        amount: 3,
                    }],
                },
                Ability::Triggered {
                    when: Trigger::Enters,
                    only_if: None,
                    effects: &[Effect::LoseLife {
                        who: Who::EachOpponent,
                        amount: 1,
                    }],
                },
            ],
        };
        let mut g = blank_game();
        lands(&mut g, 0, "meadow", 1);
        let card = g.create(&TWO_TRIGGERS, 0, Zone::Hand);
        g.players[0].hand.push(card);
        g.apply(0, Action::Cast { card, target: None }).unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&card].zone, Zone::Battlefield);
        assert_eq!(g.players[0].life, 23);
        assert_eq!(g.players[1].life, 19);
        let line = |text: &str| g.log.iter().position(|l| l == text).unwrap();
        assert!(line("Ann gains 3 life.") < line("Bob loses 1 life."));
    }

    /// A card from the Blightmaw test pack (`tests/fixtures/blightmaw.toml`).
    fn blightmaw_card(key: &str) -> &'static CardDef {
        static POOL: std::sync::OnceLock<CardPool> = std::sync::OnceLock::new();
        POOL.get_or_init(|| {
            let mut pool = CardPool::builtin();
            let pack = toml::from_str(include_str!("../tests/fixtures/blightmaw.toml")).unwrap();
            pool.add_pack(pack).unwrap();
            pool
        })
        .card(key)
        .unwrap()
    }

    fn cast_from_hand(g: &mut Game, p: PlayerId, def: &'static CardDef) -> ObjectId {
        let card = g.create(def, p, Zone::Hand);
        g.players[p].hand.push(card);
        g.apply(p, Action::Cast { card, target: None }).unwrap();
        card
    }

    #[test]
    fn blightmaw_rules_text() {
        assert_eq!(
            blightmaw_card("blightmaw-tyrant").rules_text(),
            "Flying\n\
             When this creature enters, if you didn't cast it from your hand, you lose the \
             game.\n\
             Whenever this creature deals combat damage to a player, that player loses the game."
        );
    }

    /// Casts Blightmaw from Ann's hand and lets it resolve.
    fn cast_blightmaw() -> (Game, ObjectId) {
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 7);
        let tyrant = cast_from_hand(&mut g, 0, blightmaw_card("blightmaw-tyrant"));
        pass_until(&mut g, |g| g.stack.is_empty() && g.on_battlefield(tyrant));
        (g, tyrant)
    }

    #[test]
    fn blightmaw_cast_from_hand_is_safe() {
        let (g, _) = cast_blightmaw();
        assert!(!g.is_over());
        assert!(
            !g.log.iter().any(|l| l.contains("ability triggers")),
            "the condition is false, so the ability doesn't even trigger"
        );
    }

    #[test]
    fn blightmaw_reanimated_loses_you_the_game() {
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 4);
        let tyrant = in_graveyard(&mut g, 0, blightmaw_card("blightmaw-tyrant"));
        cast_at(
            &mut g,
            0,
            "call-from-the-mire",
            Target::GraveyardCard(tyrant),
        );
        pass_until(&mut g, |g| g.is_over());
        assert!(g.on_battlefield(tyrant));
        assert_eq!(g.result(), Some(Some(1)), "Bob wins");
        assert!(
            g.log
                .contains(&"Ann loses the game (Blightmaw Tyrant).".to_string())
        );
    }

    #[test]
    fn blightmaw_blinked_loses_you_the_game() {
        // It was cast from hand, but the blinked one is a new object that wasn't.
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 7);
        lands(&mut g, 0, "meadow", 2);
        put(&mut g, 0, "veilstep", Zone::Hand);
        let tyrant = cast_from_hand(&mut g, 0, blightmaw_card("blightmaw-tyrant"));
        pass_until(&mut g, |g| g.stack.is_empty() && g.on_battlefield(tyrant));
        assert!(!g.is_over());
        let veilstep = *g.players[0].hand.last().unwrap();
        g.apply(
            0,
            Action::Cast {
                card: veilstep,
                target: Some(Target::Permanent(tyrant)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.is_over());
        assert_eq!(g.result(), Some(Some(1)));
    }

    /// Ann casts Blightmaw, and on her next turn attacks Bob with it.
    fn blightmaw_attacks(blocker: Option<&str>) -> (Game, ObjectId) {
        let (mut g, tyrant) = cast_blightmaw();
        pass_until(&mut g, |g| g.pending == Pending::Attackers(0));
        // Added only now so that Bob, with nothing to do, skips his own turn.
        let blocker = blocker.map(|key| put(&mut g, 1, key, Zone::Battlefield));
        let attacks = vec![Attack {
            attacker: tyrant,
            defender: 1,
        }];
        g.apply(0, Action::DeclareAttackers { attacks }).unwrap();
        if let Some(blocker) = blocker {
            let blocks = vec![Block {
                blocker,
                attacker: tyrant,
            }];
            g.apply(1, Action::DeclareBlockers { blocks }).unwrap();
        }
        (g, tyrant)
    }

    #[test]
    fn blightmaw_combat_damage_to_a_player_ends_their_game() {
        let (mut g, _) = blightmaw_attacks(None);
        pass_until(&mut g, |g| g.is_over());
        assert_eq!(
            g.players[1].life,
            STARTING_LIFE - 7,
            "the damage happened first"
        );
        assert_eq!(g.result(), Some(Some(0)), "Ann wins");
        assert!(
            g.log
                .contains(&"Bob loses the game (Blightmaw Tyrant).".to_string())
        );
    }

    #[test]
    fn blightmaw_blocked_deals_no_damage_to_the_player() {
        let (mut g, _) = blightmaw_attacks(Some("skyward-kestrel"));
        pass_until(&mut g, |g| g.step == Step::Main2);
        assert!(!g.is_over());
        assert_eq!(g.players[1].life, STARTING_LIFE);
    }

    /// Puts a card from the pool into `p`'s graveyard.
    fn in_graveyard(g: &mut Game, p: PlayerId, def: &'static CardDef) -> ObjectId {
        let id = g.create(def, p, Zone::Graveyard);
        g.players[p].graveyard.push(id);
        id
    }

    fn cast_at(g: &mut Game, p: PlayerId, key: &str, target: Target) -> ObjectId {
        let card = put(g, p, key, Zone::Hand);
        g.apply(
            p,
            Action::Cast {
                card,
                target: Some(target),
            },
        )
        .unwrap();
        card
    }

    #[test]
    fn condition_checks_how_a_permanent_arrived() {
        let wraith = cards::card("gravecall-wraith").unwrap();
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 4);
        let cast = cast_from_hand(&mut g, 0, wraith);
        pass_until(&mut g, |g| g.stack.is_empty() && g.on_battlefield(cast));
        assert!(g.players[0].hand.is_empty(), "cast from hand: no cards");
        // Bouncing it forgets how it was cast.
        g.move_to(cast, Zone::Hand);
        assert_eq!(g.objects[&cast].cast_from, None);

        let mut g = blank_game();
        lands(&mut g, 0, "bog", 4);
        let dead = in_graveyard(&mut g, 0, wraith);
        cast_at(&mut g, 0, "call-from-the-mire", Target::GraveyardCard(dead));
        pass_until(&mut g, |g| g.stack.is_empty() && g.on_battlefield(dead));
        assert_eq!(
            g.players[0].hand.len(),
            2,
            "returned from the graveyard: draw 2"
        );
    }

    #[test]
    fn graveyard_targets_are_described_for_clients() {
        let mut g = blank_game();
        let stalker = in_graveyard(&mut g, 0, cards::card("fen-stalker").unwrap());
        assert_eq!(
            g.view(1).describe_target(&Target::GraveyardCard(stalker)),
            "Fen Stalker (in Ann's graveyard)"
        );
    }

    #[test]
    fn targets_can_be_limited_to_whose_they_are() {
        let mut g = blank_game();
        lands(&mut g, 0, "meadow", 2);
        let mine = put(&mut g, 0, "grovekin", Zone::Battlefield);
        put(&mut g, 1, "grovekin", Zone::Battlefield);
        let veilstep = put(&mut g, 0, "veilstep", Zone::Hand);
        let plays = g.legal_plays(0);
        let play = plays.iter().find(|o| o.card == veilstep).unwrap();
        assert_eq!(play.targets, Some(vec![Target::Permanent(mine)]));
        assert_eq!(
            g.objects[&veilstep].def.rules_text(),
            "Exile target creature you control, then return it to the battlefield under its \
             owner's control."
        );
    }

    #[test]
    fn return_to_hand_from_graveyard() {
        let mut g = blank_game();
        lands(&mut g, 0, "thicket", 2);
        let boar = in_graveyard(&mut g, 0, cards::card("thornback-boar").unwrap());
        cast_at(&mut g, 0, "mossgrave-recovery", Target::GraveyardCard(boar));
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&boar].zone, Zone::Hand);
        assert!(g.players[0].hand.contains(&boar));
    }

    #[test]
    fn graveyard_targets_are_only_your_creature_cards() {
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 4);
        let mine = in_graveyard(&mut g, 0, cards::card("fen-stalker").unwrap());
        in_graveyard(&mut g, 0, cards::card("blaze").unwrap());
        in_graveyard(&mut g, 1, cards::card("night-leech").unwrap());
        let call = put(&mut g, 0, "call-from-the-mire", Zone::Hand);
        let plays = g.legal_plays(0);
        let play = plays.iter().find(|o| o.card == call).unwrap();
        assert_eq!(play.targets, Some(vec![Target::GraveyardCard(mine)]));
    }

    #[test]
    fn a_target_that_moves_away_and_back_is_lost() {
        // The card that comes back to the graveyard is a new object as far as
        // the rules are concerned, so the spell no longer knows about it.
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 4);
        // An instant Ann could still cast, so she keeps priority after casting.
        lands(&mut g, 0, "meadow", 1);
        put(&mut g, 0, "mending-light", Zone::Hand);
        let stalker = in_graveyard(&mut g, 0, cards::card("fen-stalker").unwrap());
        cast_at(
            &mut g,
            0,
            "call-from-the-mire",
            Target::GraveyardCard(stalker),
        );
        assert_eq!(g.stack.len(), 1, "not resolved yet");
        g.move_to(stalker, Zone::Hand);
        g.move_to(stalker, Zone::Graveyard);
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&stalker].zone, Zone::Graveyard);
        assert!(
            g.log
                .iter()
                .any(|l| l == "Call from the Mire fizzles: its target is gone.")
        );
    }

    #[test]
    fn blink_saves_a_creature_from_removal() {
        let mut g = blank_game();
        lands(&mut g, 0, "bog", 3);
        lands(&mut g, 1, "meadow", 2);
        let pilgrim = put(&mut g, 1, "lantern-pilgrim", Zone::Battlefield);
        put(&mut g, 1, "veilstep", Zone::Hand);
        cast_at(&mut g, 0, "grasp-of-ruin", Target::Permanent(pilgrim));
        // Ann has nothing else to do, so Bob gets priority and responds.
        assert_eq!(g.waiting_on(), Some(1));
        let veilstep = *g.players[1].hand.last().unwrap();
        g.apply(
            1,
            Action::Cast {
                card: veilstep,
                target: Some(Target::Permanent(pilgrim)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert!(g.on_battlefield(pilgrim), "the pilgrim survives");
        assert!(
            g.log
                .iter()
                .any(|l| l == "Grasp of Ruin fizzles: its target is gone.")
        );
        assert_eq!(
            g.players[1].life,
            STARTING_LIFE + 2,
            "and its enters ability happened again"
        );
    }

    #[test]
    fn decking_loses() {
        let mut g = blank_game();
        for p in 0..2 {
            let library = std::mem::take(&mut g.players[p].library);
            for id in library {
                g.objects.remove(&id);
            }
        }
        pass_until(&mut g, |g| g.is_over());
        // Player 1 draws first, from an empty library.
        assert_eq!(g.result(), Some(Some(0)));
    }

    #[test]
    fn flashback_casts_from_the_graveyard_then_exiles() {
        let mut g = blank_game();
        let spark = in_graveyard(&mut g, 0, cards::card("sparkfall").unwrap());
        lands(&mut g, 0, "crag", 2);
        assert!(
            g.legal_plays(0).iter().all(|o| o.card != spark),
            "two lands pay {{1}}{{R}} but not the flashback cost {{2}}{{R}}"
        );
        lands(&mut g, 0, "crag", 1);
        g.apply(
            0,
            Action::Cast {
                card: spark,
                target: Some(Target::Player(1)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.players[1].life, STARTING_LIFE - 2);
        assert_eq!(g.objects[&spark].zone, Zone::Exile);
        let names: Vec<_> = g.view(1).players[0]
            .exile
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["Sparkfall"], "exile is public");
    }

    #[test]
    fn a_countered_spell_cast_from_hand_goes_to_the_graveyard() {
        let mut g = blank_game();
        lands(&mut g, 0, "crag", 2);
        lands(&mut g, 1, "lagoon", 2);
        put(&mut g, 1, "dissolve-thought", Zone::Hand);
        let spark = cast_at(&mut g, 0, "sparkfall", Target::Player(1));
        let counter = *g.players[1].hand.last().unwrap();
        g.apply(
            1,
            Action::Cast {
                card: counter,
                target: Some(Target::Spell(spark)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&spark].zone, Zone::Graveyard);
    }

    #[test]
    fn countering_a_spell_cast_with_flashback_exiles_it() {
        let mut g = blank_game();
        let spark = in_graveyard(&mut g, 0, cards::card("sparkfall").unwrap());
        lands(&mut g, 0, "crag", 3);
        lands(&mut g, 1, "lagoon", 2);
        put(&mut g, 1, "dissolve-thought", Zone::Hand);
        g.apply(
            0,
            Action::Cast {
                card: spark,
                target: Some(Target::Player(1)),
            },
        )
        .unwrap();
        let counter = *g.players[1].hand.last().unwrap();
        g.apply(
            1,
            Action::Cast {
                card: counter,
                target: Some(Target::Spell(spark)),
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.players[1].life, STARTING_LIFE);
        assert_eq!(g.objects[&spark].zone, Zone::Exile);
    }

    #[test]
    fn beckon_puts_a_chosen_creature_onto_the_battlefield() {
        let mut g = blank_game();
        lands(&mut g, 0, "thicket", 3);
        let wraith = put(&mut g, 0, "gravecall-wraith", Zone::Hand);
        put(&mut g, 0, "blaze", Zone::Hand);
        let beckon = put(&mut g, 0, "beckon-the-wild", Zone::Hand);
        g.apply(
            0,
            Action::Cast {
                card: beckon,
                target: None,
            },
        )
        .unwrap();
        pass_until(&mut g, |g| g.pending == Pending::Choose(0));
        let Prompt::ChooseCard {
            options, optional, ..
        } = g.view(0).prompt
        else {
            panic!("expected a choice");
        };
        assert_eq!(options, [wraith], "only creature cards are offered");
        assert!(optional);
        assert_eq!(
            g.stack.len(),
            1,
            "Beckon stays on the stack while Ann chooses"
        );
        // Bob only learns that Ann is choosing, not what she could choose.
        let bob = g.view(1);
        assert_eq!(bob.prompt, Prompt::Waiting { on: 0 });
        assert!(!format!("{bob:?}").contains("Gravecall"));
        assert_eq!(
            g.apply(0, Action::ChooseCard { card: Some(beckon) }),
            Err(ActionError("you can't choose that card".into()))
        );

        let hand = g.players[0].hand.len();
        g.apply(0, Action::ChooseCard { card: Some(wraith) })
            .unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert!(g.on_battlefield(wraith));
        assert_eq!(g.objects[&beckon].zone, Zone::Graveyard);
        assert_eq!(
            g.players[0].hand.len(),
            hand - 1 + 2,
            "the wraith wasn't cast, so it draws 2"
        );
    }

    #[test]
    fn beckon_can_choose_nothing_or_find_nothing() {
        let mut g = blank_game();
        lands(&mut g, 0, "thicket", 6);
        let boar = put(&mut g, 0, "thornback-boar", Zone::Hand);
        cast_from_hand(&mut g, 0, cards::card("beckon-the-wild").unwrap());
        pass_until(&mut g, |g| g.pending == Pending::Choose(0));
        g.apply(0, Action::ChooseCard { card: None }).unwrap();
        pass_until(&mut g, |g| g.stack.is_empty());
        assert_eq!(g.objects[&boar].zone, Zone::Hand);

        // With no creature card in hand there's nothing to ask.
        g.move_to(boar, Zone::Graveyard);
        cast_from_hand(&mut g, 0, cards::card("beckon-the-wild").unwrap());
        pass_until(&mut g, |g| g.stack.is_empty());
        assert!(
            g.log
                .iter()
                .any(|l| l == "Ann has no creature card to put onto the battlefield.")
        );
    }

    #[test]
    fn blightmaw_beckoned_loses_you_the_game() {
        let mut g = blank_game();
        lands(&mut g, 0, "thicket", 3);
        let tyrant = g.create(blightmaw_card("blightmaw-tyrant"), 0, Zone::Hand);
        g.players[0].hand.push(tyrant);
        cast_from_hand(&mut g, 0, cards::card("beckon-the-wild").unwrap());
        pass_until(&mut g, |g| g.pending == Pending::Choose(0));
        g.apply(0, Action::ChooseCard { card: Some(tyrant) })
            .unwrap();
        pass_until(&mut g, |g| g.is_over());
        assert_eq!(g.result(), Some(Some(1)));
    }

    #[test]
    fn hidden_information_stays_hidden() {
        let g = blank_game();
        let mut g = g;
        let secret = put(&mut g, 1, "blaze", Zone::Hand);
        let view = g.view(0);
        assert!(view.hand.iter().all(|c| c.id != secret));
        assert_eq!(view.players[1].hand_size, 1);
        let json = format!("{view:?}");
        assert!(!json.contains("Blaze"));
    }
}
