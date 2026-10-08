//! Deck files: a card pack (TOML) holding exactly one deck. The deck builder
//! writes them, `magus --deck <file>` plays them, and a server can offer one to
//! everybody by loading it with `--cards`.

use std::path::Path;

use anyhow::{Context, bail};
use magus_core::Pack;
use magus_core::pool::DeckSpec;
use magus_protocol::CustomDeck;

/// Reads the single deck in `path`.
pub fn load(path: &Path) -> anyhow::Result<DeckSpec> {
    let pack = magus_server::load_pack(path)?;
    let mut decks = pack.decks.into_iter();
    match (decks.next(), decks.next()) {
        (Some(deck), None) => Ok(deck),
        (None, _) => bail!("{} has no [[deck]]", path.display()),
        (Some(_), Some(_)) => bail!(
            "{} has more than one [[deck]]; a deck file holds just one",
            path.display()
        ),
    }
}

/// Writes `deck` to `path` as a pack with just that deck.
pub fn save(path: &Path, deck: &DeckSpec) -> anyhow::Result<()> {
    let pack = Pack {
        cards: Vec::new(),
        decks: vec![deck.clone()],
    };
    let text = toml::to_string(&pack).context("writing the deck as TOML")?;
    std::fs::write(path, text).with_context(|| format!("saving {}", path.display()))
}

/// The deck as sent to a server.
pub fn to_custom(deck: DeckSpec) -> CustomDeck {
    CustomDeck {
        name: deck.name,
        cards: deck.cards,
    }
}

/// A key and display name for a new deck saved at `path`: `my-deck.toml`
/// becomes `my-deck` and "My Deck".
pub fn names_for(path: &Path) -> (String, String) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".into());
    let name = stem
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    (stem, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_a_deck() {
        let dir = std::env::temp_dir().join(format!("magus-deckfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("red-rush.toml");
        let (key, name) = names_for(&path);
        assert_eq!((key.as_str(), name.as_str()), ("red-rush", "Red Rush"));
        let deck = DeckSpec {
            key,
            name,
            description: String::new(),
            cards: [("crag".to_string(), 56), ("blaze".to_string(), 4)]
                .into_iter()
                .collect(),
        };
        save(&path, &deck).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[deck]]"), "{text}");
        assert_eq!(load(&path).unwrap(), deck);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
