//! Plays many games of random legal moves, using only what each player's view
//! offers, and checks that every game finishes and no card is ever lost.

use magus_core::cards::DECKS;
use magus_core::view::Prompt;
use magus_core::{Action, Attack, Block, Game, GameView};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

fn random_action(view: &GameView, rng: &mut StdRng) -> Action {
    match &view.prompt {
        Prompt::Priority { plays } => {
            if plays.is_empty() || rng.gen_bool(0.3) {
                return Action::Pass;
            }
            let play = plays.choose(rng).unwrap();
            match &play.targets {
                None if play.kind == magus_core::view::PlayKind::Land => {
                    Action::PlayLand { card: play.card }
                }
                None => Action::Cast {
                    card: play.card,
                    target: None,
                },
                Some(targets) => Action::Cast {
                    card: play.card,
                    target: Some(*targets.choose(rng).unwrap()),
                },
            }
        }
        Prompt::DeclareAttackers { options } => {
            let mut attacks = Vec::new();
            for o in options {
                if rng.gen_bool(0.6) {
                    attacks.push(Attack {
                        attacker: o.attacker,
                        defender: *o.defenders.choose(rng).unwrap(),
                    });
                }
            }
            Action::DeclareAttackers { attacks }
        }
        Prompt::DeclareBlockers { options } => {
            let mut blocks = Vec::new();
            for o in options {
                if rng.gen_bool(0.5) {
                    blocks.push(Block {
                        blocker: o.blocker,
                        attacker: *o.attackers.choose(rng).unwrap(),
                    });
                }
            }
            Action::DeclareBlockers { blocks }
        }
        Prompt::Discard { count } => {
            let mut hand: Vec<_> = view.hand.iter().map(|c| c.id).collect();
            hand.shuffle(rng);
            Action::Discard {
                cards: hand[..*count].to_vec(),
            }
        }
        Prompt::Waiting { .. } | Prompt::GameOver { .. } => {
            panic!("asked to act on {:?}", view.prompt)
        }
    }
}

fn card_count(view: &GameView, p: usize) -> usize {
    let pl = &view.players[p];
    pl.hand_size
        + pl.library_size
        + pl.graveyard.len()
        + view
            .battlefield
            .iter()
            .filter(|perm| perm.owner == p)
            .count()
        + view
            .stack
            .iter()
            .filter(|s| s.is_spell && s.controller == p)
            .count()
}

#[test]
fn random_games_finish_and_conserve_cards() {
    let mut finished = 0;
    for seed in 0..300u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let a = &DECKS[seed as usize % DECKS.len()];
        let b = &DECKS[(seed as usize / DECKS.len()) % DECKS.len()];
        let seats = [("A".to_string(), a), ("B".to_string(), b)];
        let mut game = Game::new(&seats, seed);
        for _ in 0..20_000 {
            let Some(p) = game.waiting_on() else { break };
            let view = game.view(p);
            for q in 0..2 {
                assert_eq!(
                    card_count(&view, q),
                    60,
                    "seed {seed}: player {q} lost track of a card"
                );
            }
            let action = random_action(&view, &mut rng);
            if let Err(e) = game.apply(p, action.clone()) {
                panic!("seed {seed}: legal-looking action {action:?} was rejected: {e}");
            }
        }
        assert!(game.is_over(), "seed {seed}: game did not finish");
        finished += 1;
    }
    assert_eq!(finished, 300);
}
