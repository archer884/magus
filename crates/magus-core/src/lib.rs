//! The Magus rules engine: cards, decks, and an authoritative game state that
//! hands each player a filtered view of the game.

pub mod card;
pub mod cards;
pub mod game;
pub mod mana;
pub mod view;

pub use game::{Action, ActionError, Attack, Block, Game, ObjectId, PlayerId, Step, Target};
pub use view::{GameView, Prompt};
