use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    White,
    Blue,
    Black,
    Red,
    Green,
}

impl Color {
    pub const ALL: [Color; 5] = [
        Color::White,
        Color::Blue,
        Color::Black,
        Color::Red,
        Color::Green,
    ];

    pub fn symbol(self) -> char {
        match self {
            Color::White => 'W',
            Color::Blue => 'U',
            Color::Black => 'B',
            Color::Red => 'R',
            Color::Green => 'G',
        }
    }

    pub fn from_symbol(c: char) -> Option<Color> {
        Color::ALL.into_iter().find(|color| color.symbol() == c)
    }

    pub fn name(self) -> &'static str {
        match self {
            Color::White => "white",
            Color::Blue => "blue",
            Color::Black => "black",
            Color::Red => "red",
            Color::Green => "green",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A mana cost such as `{2}{R}{R}`: some generic mana plus colored requirements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ManaCost {
    pub generic: u32,
    pub colored: [u32; 5],
}

impl ManaCost {
    /// Parses compact notation: `"2RR"` is two generic plus two red.
    ///
    /// Panics on a malformed cost; use [`ManaCost::try_parse`] for untrusted text.
    pub fn parse(s: &str) -> ManaCost {
        ManaCost::try_parse(s).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like [`ManaCost::parse`], but reports a malformed cost instead of panicking.
    pub fn try_parse(s: &str) -> Result<ManaCost, String> {
        let mut cost = ManaCost::default();
        let mut digits = String::new();
        for ch in s.chars() {
            if ch.is_ascii_digit() {
                // "1R1" would otherwise quietly mean eleven generic mana.
                if cost.colored.iter().any(|&n| n > 0) {
                    return Err(format!("generic mana must come first in cost {s:?}"));
                }
                digits.push(ch);
            } else if let Some(color) = Color::from_symbol(ch) {
                cost.colored[color.index()] += 1;
            } else {
                return Err(format!("bad mana symbol {ch:?} in cost {s:?}"));
            }
        }
        if !digits.is_empty() {
            cost.generic = digits
                .parse()
                .map_err(|_| format!("generic mana in {s:?} is too large"))?;
        }
        Ok(cost)
    }

    /// This cost made cheaper by `by`. Colored mana only reduces the same
    /// color, and generic only generic, as in the real rules; nothing goes
    /// below zero.
    pub fn reduce(&self, by: &ManaCost) -> ManaCost {
        let mut cost = *self;
        cost.generic = cost.generic.saturating_sub(by.generic);
        for (c, b) in cost.colored.iter_mut().zip(by.colored) {
            *c = c.saturating_sub(b);
        }
        cost
    }

    pub fn mana_value(&self) -> u32 {
        self.generic + self.colored.iter().sum::<u32>()
    }

    pub fn colors(&self) -> Vec<Color> {
        Color::ALL
            .into_iter()
            .filter(|c| self.colored[c.index()] > 0)
            .collect()
    }
}

impl fmt::Display for ManaCost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.generic > 0 {
            write!(f, "{{{}}}", self.generic)?;
        }
        for color in Color::ALL {
            for _ in 0..self.colored[color.index()] {
                write!(f, "{{{}}}", color.symbol())?;
            }
        }
        Ok(())
    }
}

/// Chooses which mana sources to tap to pay `cost`, or `None` if it can't be paid.
///
/// Every source makes exactly one mana of one color, so colored requirements are
/// met first and generic mana is then drawn from whichever color is most plentiful,
/// leaving the player as flexible as possible.
pub fn plan_payment<T: Copy>(cost: &ManaCost, sources: &[(T, Color)]) -> Option<Vec<T>> {
    let mut by_color: [Vec<T>; 5] = Default::default();
    for &(id, color) in sources {
        by_color[color.index()].push(id);
    }
    let mut chosen = Vec::new();
    for color in Color::ALL {
        let need = cost.colored[color.index()] as usize;
        let pool = &mut by_color[color.index()];
        if pool.len() < need {
            return None;
        }
        chosen.extend(pool.drain(..need));
    }
    for _ in 0..cost.generic {
        let pool = by_color.iter_mut().max_by_key(|pool| pool.len())?;
        chosen.push(pool.pop()?);
    }
    Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let cost = ManaCost::parse("2RR");
        assert_eq!(cost.generic, 2);
        assert_eq!(cost.mana_value(), 4);
        assert_eq!(cost.to_string(), "{2}{R}{R}");
        assert_eq!(ManaCost::parse("").to_string(), "");
        assert!(ManaCost::try_parse("2X").is_err());
        assert!(ManaCost::try_parse("1R1").is_err());
        let reduced = ManaCost::parse("2BB").reduce(&ManaCost::parse("1BR"));
        assert_eq!(reduced.to_string(), "{1}{B}");
    }

    #[test]
    fn payment_prefers_colored_then_most_plentiful() {
        let sources = [
            (1, Color::Red),
            (2, Color::Green),
            (3, Color::Green),
            (4, Color::Red),
        ];
        let paid = plan_payment(&ManaCost::parse("1R"), &sources).unwrap();
        assert_eq!(paid.len(), 2);
        // The red requirement takes a red source; the generic comes from green
        // because green is then the more plentiful color.
        assert!(paid.contains(&1));
        assert!(paid.contains(&3) || paid.contains(&2));
        assert!(plan_payment(&ManaCost::parse("RRR"), &sources).is_none());
        assert!(plan_payment(&ManaCost::parse("4"), &sources).is_some());
        assert!(plan_payment(&ManaCost::parse("5"), &sources).is_none());
    }
}
