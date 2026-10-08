# Magus

A trading-card game you play in the terminal, in the style of classic
two-player Magic: The Gathering. It has an original card pool, a real
priority-and-stack rules engine, and a client/server split, so you can play
over the internet.

New to card games like this? Read [docs/HOW_TO_PLAY.md](docs/HOW_TO_PLAY.md).

## Playing

```sh
cargo build --release

# Host a server (default port 7878):
./target/release/magus-server

# Each player connects and joins the same room:
./target/release/magus --server host:7878 --room friday
```

You'll pick a deck when you join, or you can pass `--deck ember-thorn` (also
`tide-ash`, `dawn-grove` or `storm-cinder`). The game starts once a second
player joins the same room.

**To play against the computer**, one command is enough:

```sh
./target/release/magus --solo
# or, during development:
cargo run --release -p magus-tui -- --solo
```

Solo mode runs a private server and a bot inside the client, so you don't
start anything else. The bot plays a random deck. Pick it with `--bot-deck
tide-ash`, and speed it up or slow it down with `--bot-delay-ms` (default 600).
`--deck` still picks your own deck.

You can also run the pieces separately, for example to watch the server log
or to point a bot at a remote server:

```sh
./target/release/magus-server &
./target/release/magus-bot --room practice --deck tide-ash &
./target/release/magus --room practice
```

The server speaks plain TCP. To play over the internet, forward the port or
run the server on a reachable host. There's no TLS or authentication yet.

## Card packs

A server can offer your own cards and decks alongside the built-in ones. Write
them in a TOML file and pass it with `--cards` (repeat it for several packs):

```sh
./target/release/magus-server --cards my-pack.toml
./target/release/magus --solo --cards my-pack.toml   # solo mode too
```

Pack decks appear in the deck list when players join. A pack looks like this
(a full example is `crates/magus-core/tests/fixtures/sample-pack.toml`):

```toml
[[card]]
key = "gullwing-courier"        # unique; decks refer to cards by key
name = "Gullwing Courier"
type = "creature"               # creature, instant, sorcery or land
cost = "2U"                     # generic first, then W U B R G
subtype = "Bird"
power = 1
toughness = 2
keywords = ["flying"]

# Creatures do things through abilities. "enters" means "when this creature
# enters the battlefield". Add more [[card.ability]] blocks for more abilities.
[[card.ability]]
type = "triggered"
when = "enters"
effects = [{ type = "draw", who = "you", count = 1 }]

[[card]]
key = "riptide-lash"
name = "Riptide Lash"
type = "instant"
cost = "1UB"
effects = [
    { type = "damage", amount = 2, target = "creature" },
    { type = "draw", who = "you", count = 1 },
]

[[deck]]
key = "harbor-tides"
name = "Harbor Tides"
description = "Shown in the deck list."

[deck.cards]                    # card key = copies; exactly 60 in total
gullwing-courier = 4
# ...
```

Effects: `damage` (`amount`, `target`: `any`, `creature` or `player`),
`destroy`, `bounce`, `pump` (`power`, `toughness`), `counter`, `blink` (exile
a creature and return it at once, as a new object), `return_to_battlefield` /
`return_to_hand` (target a creature card in your graveyard), `draw` (`who`,
`count`), `gain_life` / `lose_life` / `damage_players` (`who`, `amount`),
`lose_game` (`who`). `who` is `you`, `each_opponent`, or `that_player`. A spell
has at most one effect that targets, and abilities can't target yet. Lands take `mana = "R"` (one symbol) and nothing else.

Abilities trigger (`when`) on `enters` or `deals_combat_damage_to_player`. With
the second one, `that_player` means the player who was hit:

```toml
[[card.ability]]
type = "triggered"
when = "deals_combat_damage_to_player"
effects = [{ type = "lose_life", who = "that_player", amount = 1 }]
```

An ability can also have an "if" (`only_if`), checked when it triggers and
again when it resolves: `{ type = "cast_from", zone = "hand" }` or
`{ type = "not_cast_from", zone = "hand" }`. "Not cast from" is also true when
the card wasn't cast at all, e.g. when an effect returns it from the
graveyard. `crates/magus-core/tests/fixtures/blightmaw.toml` uses both.

The server checks a pack when it starts and refuses to run if anything is
wrong, listing every problem: a typo'd field, a key that's already taken, a
creature missing its toughness, a deck that isn't 60 cards, and so on.

### Keys

| Key | |
|---|---|
| ↑ ↓ (or j k) | Move the selection |
| Enter | Play the selected card / confirm |
| Space | Pass priority / toggle the selection (attackers, blockers, discards) |
| ← → | Choose which attacker a creature blocks |
| a | Attack with everything (or nothing) |
| Tab | Switch between hand, battlefield and stack to inspect cards |
| f | Full control: get asked at every step instead of auto-passing routine ones |
| q | Quit (twice in a game, because leaving concedes) |

## Architecture

```
crates/
  magus-core      rules engine; no I/O; deterministic given a seed
  magus-protocol  JSON-lines messages + connection helpers
  magus-server    lobby (rooms) + one task per game, which owns the Game
  magus-tui       the `magus` terminal client (ratatui)
  magus-bot       a computer player that connects like any client
```

**The server is the only authority.** After every change it sends each player
a `GameView`. A view holds only what that player may see: their own hand, but
just a card count for the opponent's. It also carries a `Prompt` listing every
legal choice right now: playable cards, the legal targets for each, possible
attackers and blockers. A client therefore needs **no rules knowledge**. It
draws the view and sends back one of the offered actions. That keeps
non-terminal clients (web, GUI, mobile) cheap to write. The bot is built that
way too.

Every action quotes the view `version` it was chosen from, so the server
ignores an action that was meant for an older game state.

### Protocol

One JSON object per line over TCP:

```
→ {"type":"hello","name":"ann","protocol":1}
← {"type":"welcome","decks":[…]}
→ {"type":"join","room":"friday","deck":"ember-thorn"}
← {"type":"waiting","room":"friday"}
← {"type":"started","seat":0,"players":["ann","bob"]}
← {"type":"state","view":{…,"prompt":{"type":"priority","plays":[…]}}}
→ {"type":"act","version":12,"action":{"type":"cast","card":31,"target":{"kind":"player","id":1}}}
```

See `crates/magus-protocol/src/lib.rs` and `crates/magus-core/src/view.rs`.

## Rules implemented

- 20 life, 7-card hands, 60-card decks; the player who goes first skips their first draw
- Turn steps: untap, upkeep, draw, main, declare attackers, declare blockers, combat damage, main, end, cleanup (discard down to 7)
- One land per turn; mana is paid automatically from untapped lands
- Full priority and stack: instants at any time, responses to spells, counterspells, and spells that fizzle when their target is gone
- Combat: multiple blockers per attacker, damage assigned in block order
- Keywords: flying, reach, haste, vigilance, lifelink, deathtouch, defender
- Triggered abilities: "when this enters" and "whenever this deals combat damage to a player", optionally with an "if" about how the card was cast
- Returning creature cards from the graveyard to the battlefield or hand, and blinking (a blinked creature is a new object, so spells aimed at it fizzle)
- "Until end of turn" boosts
- Extra cards and decks from card packs
- You lose at 0 life, by drawing from an empty library, or when a card says so; conceding or disconnecting forfeits

To keep play fast, the engine passes priority for you whenever you have
nothing you could do. The client also skips routine steps unless you turn on
full control (`f`).

## Roadmap

Planned work is in [TODO.md](TODO.md): mulligans, multiplayer/Commander, more
card mechanics, a server database with accounts and trading, and a
leaner wire protocol.

## Contributing

```sh
cargo test --workspace                  # unit tests, 300 random full games, and a bot-vs-bot game over real TCP
cargo clippy --workspace --all-targets  # keep at zero warnings
cargo fmt --all
```

Before your first change, read [AGENTS.md](AGENTS.md). It's written for AI
agents but is the best map of the code for humans too. It covers the design
rules: the server is the only authority, hidden information stays hidden, and
every new decision type needs a `Prompt`.

Adding a built-in card means a line in `crates/magus-core/src/cards.rs`; to
try one out without touching the engine, put it in a card pack instead. Cards
must be original; don't copy names or text from real games. If the card does
something new, add a reusable building block (an `Effect` or `Trigger`
variant in `card.rs`) rather than code for that one card. The bot (`magus-bot`) is a quick way to playtest: run two
of them against each other, or play one yourself.
