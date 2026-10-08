//! A simple computer opponent. It connects like any other client and decides
//! using only its own `GameView`, plus the public card definitions.

use std::time::Duration;

use magus_core::card::{Effect, Keyword};
use magus_core::cards;
use magus_core::view::{PermanentView, PlayKind, PlayOption, Prompt};
use magus_core::{Action, Attack, Block, GameView, ObjectId, Step, Target};
use magus_protocol::{ClientMsg, PROTOCOL_VERSION, ServerMsg};

/// How a finished game went for the bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Won,
    Lost,
    Draw,
}

/// Plays one game in `room` and returns how it went, or `None` if the
/// connection dropped first. `delay` paces the bot so humans can follow along.
pub async fn run(
    addr: &str,
    name: &str,
    room: &str,
    deck: &str,
    delay: Duration,
) -> anyhow::Result<Option<Outcome>> {
    let (tx, mut rx) = magus_protocol::connect(addr).await?;
    tx.send(ClientMsg::Hello {
        name: name.into(),
        protocol: PROTOCOL_VERSION,
    })?;
    let mut last: Option<GameView> = None;
    while let Some(msg) = rx.recv().await {
        match msg {
            ServerMsg::Welcome { .. } => tx.send(ClientMsg::Join {
                room: room.into(),
                deck: deck.into(),
            })?,
            ServerMsg::Waiting { .. } | ServerMsg::Started { .. } => {}
            ServerMsg::State { view } => {
                if let Prompt::GameOver { winner } = view.prompt {
                    return Ok(Some(match winner {
                        Some(w) if w == view.you => Outcome::Won,
                        Some(_) => Outcome::Lost,
                        None => Outcome::Draw,
                    }));
                }
                if let Some(action) = choose(&view) {
                    if action != Action::Pass && !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    tx.send(ClientMsg::Act {
                        version: view.version,
                        action,
                    })?;
                }
                last = Some(*view);
            }
            ServerMsg::Error { message } => {
                tracing::warn!(%name, %message, "server rejected an action");
                // Never get stuck: fall back to the most passive legal choice.
                if let Some(view) = &last
                    && let Some(action) = fallback(view)
                {
                    tx.send(ClientMsg::Act {
                        version: view.version,
                        action,
                    })?;
                }
            }
        }
    }
    Ok(None)
}

fn fallback(view: &GameView) -> Option<Action> {
    Some(match &view.prompt {
        Prompt::Priority { .. } => Action::Pass,
        Prompt::DeclareAttackers { .. } => Action::DeclareAttackers { attacks: vec![] },
        Prompt::DeclareBlockers { .. } => Action::DeclareBlockers { blocks: vec![] },
        Prompt::Discard { count } => Action::Discard {
            cards: view.hand.iter().take(*count).map(|c| c.id).collect(),
        },
        Prompt::Waiting { .. } | Prompt::GameOver { .. } => return None,
    })
}

/// Picks an action for whatever the prompt asks, or `None` if it isn't our turn to decide.
pub fn choose(view: &GameView) -> Option<Action> {
    match &view.prompt {
        Prompt::Priority { plays } => Some(choose_play(view, plays)),
        Prompt::DeclareAttackers { options } => Some(Action::DeclareAttackers {
            attacks: options
                .iter()
                .filter(|o| should_attack(view, o.attacker))
                .map(|o| Attack {
                    attacker: o.attacker,
                    defender: o.defenders[0],
                })
                .collect(),
        }),
        Prompt::DeclareBlockers { options } => Some(Action::DeclareBlockers {
            blocks: choose_blocks(view, options),
        }),
        Prompt::Discard { count } => {
            let mut hand: Vec<_> = view.hand.iter().collect();
            // Keep lands only while we have few on the battlefield.
            let lands_out = view
                .battlefield
                .iter()
                .filter(|p| p.controller == view.you && p.card.is_land)
                .count();
            hand.sort_by_key(|c| {
                if c.is_land && lands_out >= 5 {
                    0
                } else {
                    100 - c.mana_value as i32
                }
            });
            Some(Action::Discard {
                cards: hand.iter().take(*count).map(|c| c.id).collect(),
            })
        }
        Prompt::Waiting { .. } | Prompt::GameOver { .. } => None,
    }
}

fn mine(view: &GameView) -> impl Iterator<Item = &PermanentView> {
    view.battlefield
        .iter()
        .filter(move |p| p.controller == view.you && p.card.is_creature)
}

fn theirs(view: &GameView) -> impl Iterator<Item = &PermanentView> {
    view.battlefield
        .iter()
        .filter(move |p| p.controller != view.you && p.card.is_creature)
}

fn threat(p: &PermanentView) -> i32 {
    p.power.unwrap_or(0) * 2
        + p.toughness.unwrap_or(0)
        + if p.card.keywords.contains(&Keyword::Flying) {
            2
        } else {
            0
        }
}

fn choose_play(view: &GameView, plays: &[PlayOption]) -> Action {
    // Don't respond to our own spells.
    if view.stack.last().is_some_and(|s| s.controller == view.you) {
        return Action::Pass;
    }
    if let Some(land) = plays.iter().find(|p| p.kind == PlayKind::Land) {
        return Action::PlayLand { card: land.card };
    }
    let mut spells: Vec<_> = plays
        .iter()
        .filter(|p| p.kind == PlayKind::Spell)
        .filter_map(|p| view.hand_card(p.card).map(|c| (p, c)))
        .collect();
    spells.sort_by_key(|(_, c)| std::cmp::Reverse(c.mana_value));
    for (play, card) in spells {
        let Some(def) = cards::card(&card.key) else {
            continue;
        };
        match &play.targets {
            None => {
                // Hold creatures and sorcery-speed cards for our own main phase,
                // which is the only time they're offered anyway.
                return Action::Cast {
                    card: play.card,
                    target: None,
                };
            }
            Some(targets) => {
                if let Some(target) = pick_target(view, def.effects, targets) {
                    return Action::Cast {
                        card: play.card,
                        target: Some(target),
                    };
                }
            }
        }
    }
    Action::Pass
}

fn pick_target(view: &GameView, effects: &[Effect], targets: &[Target]) -> Option<Target> {
    let enemy_creatures = || {
        targets.iter().filter_map(|t| match t {
            Target::Permanent(id) => view.permanent(*id).filter(|p| p.controller != view.you),
            _ => None,
        })
    };
    let effect = effects.iter().find(|e| e.target().is_some())?;
    match *effect {
        Effect::Damage { amount, .. } => {
            let killable =
                enemy_creatures().filter(|p| p.toughness.unwrap_or(0) - p.damage <= amount).max_by_key(|p| threat(p));
            if let Some(p) = killable {
                return Some(Target::Permanent(p.card.id));
            }
            let opponent = targets.iter().find(|t| matches!(t, Target::Player(p) if *p != view.you))?;
            // Save burn for creatures unless it's the end of the opponent's turn or lethal.
            let life = match opponent {
                Target::Player(p) => view.players[*p].life,
                _ => unreachable!(),
            };
            let late = view.active != view.you && view.step == Step::End;
            (late || life <= amount).then_some(*opponent)
        }
        Effect::Destroy | Effect::Bounce => enemy_creatures().max_by_key(|p| threat(p)).map(|p| Target::Permanent(p.card.id)),
        Effect::Pump { .. } => {
            if !matches!(view.step, Step::DeclareBlockers) {
                return None;
            }
            mine(view)
                .filter(|p| p.attacking.is_some() || p.blocking.is_some())
                .filter(|p| targets.contains(&Target::Permanent(p.card.id)))
                .max_by_key(|p| p.power.unwrap_or(0))
                .map(|p| Target::Permanent(p.card.id))
        }
        Effect::Counter => targets
            .iter()
            .find(|t| matches!(t, Target::Spell(id) if view.stack_item(*id).is_some_and(|s| s.controller != view.you)))
            .copied(),
        _ => targets.first().copied(),
    }
}

fn should_attack(view: &GameView, attacker: ObjectId) -> bool {
    let Some(me) = view.permanent(attacker) else {
        return false;
    };
    let power = me.power.unwrap_or(0);
    let toughness = me.toughness.unwrap_or(0);
    if power <= 0 {
        return false;
    }
    let flying = me.card.keywords.contains(&Keyword::Flying);
    // Attack unless some untapped enemy could block and kill us without dying.
    !theirs(view).filter(|b| !b.tapped).any(|b| {
        let can_block = !flying
            || b.card.keywords.contains(&Keyword::Flying)
            || b.card.keywords.contains(&Keyword::Reach);
        let kills =
            b.power.unwrap_or(0) >= toughness || b.card.keywords.contains(&Keyword::Deathtouch);
        let survives =
            b.toughness.unwrap_or(0) > power && !me.card.keywords.contains(&Keyword::Deathtouch);
        can_block && kills && survives
    })
}

fn choose_blocks(view: &GameView, options: &[magus_core::view::BlockOption]) -> Vec<Block> {
    let my_life = view.players[view.you].life;
    let mut attackers: Vec<&PermanentView> = theirs(view)
        .filter(|p| p.attacking == Some(view.you))
        .collect();
    attackers.sort_by_key(|p| std::cmp::Reverse(threat(p)));
    let incoming: i32 = attackers.iter().map(|p| p.power.unwrap_or(0)).sum();
    let mut used: Vec<ObjectId> = Vec::new();
    let mut blocks = Vec::new();
    for attacker in attackers {
        let a_power = attacker.power.unwrap_or(0);
        let a_tough = attacker.toughness.unwrap_or(0) - attacker.damage;
        let a_deathtouch = attacker.card.keywords.contains(&Keyword::Deathtouch);
        let candidates: Vec<&PermanentView> = options
            .iter()
            .filter(|o| o.attackers.contains(&attacker.card.id) && !used.contains(&o.blocker))
            .filter_map(|o| view.permanent(o.blocker))
            .collect();
        // A block that survives, preferably one that also kills.
        let good = candidates
            .iter()
            .filter(|b| b.toughness.unwrap_or(0) > a_power && !a_deathtouch)
            .max_by_key(|b| (b.power.unwrap_or(0) >= a_tough, -threat(b)));
        // Or a fair trade.
        let trade = candidates
            .iter()
            .filter(|b| b.power.unwrap_or(0) >= a_tough && threat(b) <= threat(attacker))
            .min_by_key(|b| threat(b));
        // Or chump-block if we're in danger.
        let chump = (incoming >= my_life - 4)
            .then(|| candidates.iter().min_by_key(|b| threat(b)))
            .flatten();
        if let Some(b) = good.or(trade).or(chump) {
            used.push(b.card.id);
            blocks.push(Block {
                blocker: b.card.id,
                attacker: attacker.card.id,
            });
        }
    }
    blocks
}
