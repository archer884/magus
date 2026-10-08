//! The rules engine. A `Game` is driven entirely by `apply`; after every
//! accepted action the engine advances on its own until some player has a real
//! decision to make, automatically passing priority for anyone with nothing to do.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::card::{CardDef, CardKind, Effect, Keyword, TargetKind};
use crate::cards::{self, DeckList};
use crate::mana::{Color, plan_payment};
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
    lost: bool,
    drew_from_empty: bool,
}

#[derive(Debug, Clone)]
struct StackItem {
    /// For spells, the card's own id; abilities get a fresh id.
    id: ObjectId,
    source: ObjectId,
    controller: PlayerId,
    is_spell: bool,
    target: Option<Target>,
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
    GameOver(Option<PlayerId>),
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
    next_id: u32,
    version: u64,
    rng: StdRng,
    log: Vec<String>,
}

impl Game {
    /// Starts a game. Each seat is a player name and a deck; the first player
    /// is chosen at random and skips their first draw.
    pub fn new(seats: &[(String, &'static DeckList)], seed: u64) -> Game {
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
            next_id: 1,
            version: 0,
            rng: StdRng::seed_from_u64(seed),
            log: Vec::new(),
        };
        for (pid, (name, deck)) in seats.iter().enumerate() {
            let mut defs: Vec<&'static CardDef> = deck
                .cards
                .iter()
                .flat_map(|&(key, n)| {
                    let def = cards::card(key).unwrap_or_else(|| panic!("unknown card {key}"));
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
            | Pending::Discard(p, _) => Some(p),
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
                    is_spell: item.is_spell,
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
    }

    fn draw(&mut self, p: PlayerId) {
        match self.players[p].library.pop() {
            Some(id) => {
                self.players[p].hand.push(id);
                self.objects.get_mut(&id).expect("object exists").zone = Zone::Hand;
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
            match self.stack.pop() {
                Some(item) => {
                    self.resolve(item);
                    self.give_priority(self.active);
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
                self.start_turn(next);
                continue;
            }
            match self.pending {
                Pending::Priority(p) if self.players[p].lost || self.legal_plays(p).is_empty() => {
                    self.pass_priority()
                }
                Pending::Blockers(p) if self.players[p].lost => self.next_blocker(),
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
        for &id in &self.players[p].hand {
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
            if plan_payment(&def.mana_cost(), &lands).is_none() {
                continue;
            }
            let targets = match def.spell_target() {
                None => None,
                Some(kind) => {
                    let targets = self.valid_targets(kind);
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

    fn valid_targets(&self, kind: TargetKind) -> Vec<Target> {
        let mut targets = Vec::new();
        if matches!(kind, TargetKind::Any | TargetKind::Player) {
            targets.extend(self.living().map(Target::Player));
        }
        if matches!(kind, TargetKind::Any | TargetKind::Creature) {
            targets.extend(
                self.battlefield
                    .iter()
                    .filter(|id| self.objects[id].is_creature())
                    .map(|&id| Target::Permanent(id)),
            );
        }
        if kind == TargetKind::Spell {
            targets.extend(
                self.stack
                    .iter()
                    .filter(|s| s.is_spell)
                    .map(|s| Target::Spell(s.id)),
            );
        }
        targets
    }

    fn describe_target(&self, target: Target) -> String {
        match target {
            Target::Player(p) => self.name(p).to_string(),
            Target::Permanent(id) | Target::Spell(id) => self.card_name(id).to_string(),
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
        let lands = plan_payment(&def.mana_cost(), &self.untapped_lands(p))
            .expect("affordability was checked");
        for land in lands {
            self.objects.get_mut(&land).expect("land exists").tapped = true;
        }
        self.move_to(card, Zone::Stack);
        self.stack.push(StackItem {
            id: card,
            source: card,
            controller: p,
            is_spell: true,
            target,
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
            && !self.valid_targets(kind).contains(&target)
        {
            self.log(format!("{} fizzles: its target is gone.", def.name));
            if item.is_spell {
                self.move_to(item.id, Zone::Graveyard);
            }
            return;
        }
        if item.is_spell && def.is_creature() {
            self.move_to(item.id, Zone::Battlefield);
            self.objects
                .get_mut(&item.id)
                .expect("object exists")
                .controller = item.controller;
            self.log(format!("{} enters the battlefield.", def.name));
            if !def.effects.is_empty() {
                let id = self.fresh_id();
                self.stack.push(StackItem {
                    id,
                    source: item.id,
                    controller: item.controller,
                    is_spell: false,
                    target: None,
                });
                self.log(format!("{}'s ability triggers.", def.name));
            }
            return;
        }
        if !item.is_spell {
            self.log(format!("{}'s ability resolves.", def.name));
        }
        for effect in def.effects {
            self.apply_effect(*effect, item.source, item.controller, item.target);
        }
        if item.is_spell {
            self.move_to(item.id, Zone::Graveyard);
        }
    }

    fn apply_effect(
        &mut self,
        effect: Effect,
        source: ObjectId,
        controller: PlayerId,
        target: Option<Target>,
    ) {
        let you = self.name(controller).to_string();
        match effect {
            Effect::Damage { amount, .. } => {
                if let Some(t) = target {
                    self.deal_damage(source, t, amount);
                }
            }
            Effect::Destroy => {
                if let Some(Target::Permanent(id)) = target {
                    self.log(format!("{} is destroyed.", self.card_name(id)));
                    self.move_to(id, Zone::Graveyard);
                }
            }
            Effect::Bounce => {
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
            Effect::Pump { power, toughness } => {
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
            Effect::Counter => {
                if let Some(Target::Spell(id)) = target {
                    self.log(format!("{} is countered.", self.card_name(id)));
                    self.move_to(id, Zone::Graveyard);
                }
            }
            Effect::Draw(n) => {
                for _ in 0..n {
                    self.draw(controller);
                }
                let cards = if n == 1 {
                    "a card".to_string()
                } else {
                    format!("{n} cards")
                };
                self.log(format!("{you} draws {cards}."));
            }
            Effect::GainLife(n) => {
                self.players[controller].life += n;
                self.log(format!("{you} gains {n} life."));
            }
            Effect::LoseLife(n) => {
                self.players[controller].life -= n;
                self.log(format!("{you} loses {n} life."));
            }
            Effect::DamageEachOpponent(n) => {
                let opponents: Vec<_> = self.living().filter(|&q| q != controller).collect();
                for q in opponents {
                    self.deal_damage(source, Target::Player(q), n);
                }
            }
        }
    }

    fn deal_damage(&mut self, source: ObjectId, target: Target, amount: i32) {
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
            Target::Spell(_) => return,
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
            self.deal_damage(source, target, amount);
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
    use crate::cards::DECKS;

    /// A game with empty hands and battlefields on player 0's first main phase.
    fn blank_game() -> Game {
        let seats = [
            ("Ann".to_string(), &DECKS[0]),
            ("Bob".to_string(), &DECKS[1]),
        ];
        let mut game = Game::new(&seats, 7);
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
        let id = game.create(cards::card(key).unwrap(), p, zone);
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
