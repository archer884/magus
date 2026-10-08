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

### Building your own deck

```sh
./target/release/magus deck-builder --deck my-deck.toml
./target/release/magus deck-builder --deck my-deck.toml --cards my-pack.toml  # pack cards too
```

The deck builder lists every card on the left and your deck in the middle,
with the selected card, a mana curve, the color split and any problems on the
right. It works offline. Enter (or `+`) adds a copy, `-` removes one, `c` and
`t` filter the cards by color and type, `r` renames the deck, `s` saves and
`q` quits. If the file already exists, it's opened for editing.

A deck is exactly 60 cards, with at most 4 copies of any card except basic
lands. The builder won't let you add a fifth copy, and lists anything else
that's wrong. You can save an unfinished deck and come back to it.

Play it by giving its path to `--deck`, which takes either a deck file or the
name of a deck the server offers:

```sh
./target/release/magus --deck my-deck.toml --room friday
./target/release/magus --solo --deck my-deck.toml
```

The server checks your deck against its own cards, so a deck of built-in cards
works anywhere, while pack cards only work on servers that loaded that pack.
A deck file is also a card pack with one deck in it, so a server can offer
your deck to everyone with `--cards my-deck.toml`.

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

A server can offer your own cards and decks alongside the built-in ones. Make
them with the card builder, or write the TOML file by hand, and pass it with
`--cards` (repeat it for several packs):

```sh
./target/release/magus-server --cards my-pack.toml
./target/release/magus --solo --cards my-pack.toml   # solo mode too
```

### The card builder

```sh
./target/release/magus card-builder --pack my-pack.toml
```

Your pack's cards are on the left, the selected card's fields in the middle,
and a live preview on the right: how the card reads, or what's wrong with it.
It opens an existing pack (keeping any decks in it) or starts a new one.

- Cards list: `n` new card, `d` duplicate, `x` delete (press twice), Enter
  or Tab to edit.
- Card fields: Enter edits a field (type the text, Enter to finish, Esc to
  cancel), ← → change choices and numbers, `x` removes an effect or ability
  or clears an optional field. Which fields appear depends on the type.
- Effects open an editor: ← → on `type` picks the kind of effect, and the
  other fields change its details. It shows how the effect will read.
- `s` saves, even an unfinished card, and says whether a server would accept
  the pack. `q` quits (asking first if there are unsaved changes).

Build a deck from your pack with `magus deck-builder --deck my-deck.toml --cards
my-pack.toml`, and play it on any server started with `--cards my-pack.toml`.

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

Effects that **target** something:

| Effect | Fields | Targets |
|---|---|---|
| `damage` | `amount`, `target` (`any`, `creature` or `player`) | |
| `destroy`, `bounce`, `blink` | | a creature |
| `pump` | `power`, `toughness` | a creature |
| `counter` | | a spell |
| `return_to_battlefield`, `return_to_hand` | | a creature card in a graveyard |

Each of these also takes `whose`: `anyone` (the default), `you` or
`opponent`, meaning who controls the creature or spell, whose graveyard the
card is in, or which player. `return_to_*` default to `you`. For example
`{ type = "destroy", whose = "opponent" }` is "destroy target creature an
opponent controls". `blink` exiles a creature and returns it at once, as a
new object. A spell has at most one effect that targets, and abilities can't
target yet.

Effects that don't target: `draw` (`who`, `count`), `gain_life` /
`lose_life` / `damage_players` (`who`, `amount`), `lose_game` (`who`), and
`put_from_hand` ("you may put a creature card from your hand onto the
battlefield"; the player picks one while the spell resolves). `who` is `you`,
`each_opponent`, or `that_player`.

`search` searches your library ("tutoring"): you choose up to `count` cards
(default 1) that match a filter, they go `to` your `hand` (the default),
the `battlefield` (add `tapped = true` to have them enter tapped), or the
`top_of_library`, and the library is shuffled (for `top_of_library`, the rest
is shuffled and the first card you chose ends up on top). You may always find
fewer, even none. The filter is any of `kind` (`creature`, `instant`,
`sorcery`, `land`), `color` (for lands, the color of mana they make) and
`named` (a card's name); leave them all out to find any card.

```toml
effects = [{ type = "search", kind = "land", color = "black", to = "battlefield", tapped = true }]
effects = [{ type = "search", count = 2, to = "top_of_library" }]     # stack the deck
effects = [{ type = "search", named = "Bog" }]
```

A card found for your hand is revealed (shown in the log) unless the search
was for any card at all.

A card can have **cast options**: "As you cast this spell, you may search your
library for …. If you do, this spell costs {B} less to cast." The action is a
`search` for one card, and `reduction` is how much cheaper it gets (colored
mana only lowers the same color). When you cast the card you're offered each
way: normally, or "bringing" each different card the search could find.

```toml
[[card.cast_option]]
reduction = "B"
action = { type = "search", kind = "land", color = "black", to = "battlefield", tapped = true }
```

An instant or sorcery can have `flashback = "2R"`: it may also be cast from your
graveyard for that cost, and is then exiled instead of returning to the
graveyard. Lands take `mana = "R"` (one symbol) and nothing else.

### Abilities, planeswalkers and emblems

Creatures and planeswalkers can have three kinds of `[[card.ability]]`:

- `triggered`: "when X happens, do Y" (`when`, optional `only_if`, `effects`),
  shown above.
- `activated`: "pay a cost: do something". The `cost` is any of `loyalty`
  (planeswalkers only), `tap = true` ({T}: the permanent taps, and a creature
  can't do it the turn it arrives) and `mana`. It's activated like casting a
  spell, and may have one targeted effect.
- `static`: always on while the card is on the battlefield. Creatures
  `whose` (default `you`) control get +`power`/+`toughness` and/or have a
  `keyword`; `other = true` leaves out the card itself ("other creatures you
  control get +1/+1", which players call a lord).

```toml
[[card.ability]]
type = "activated"
cost = { mana = "1", tap = true }           # {1}, {T}: …
effects = [{ type = "damage", amount = 1, target = "any" }]

[[card.ability]]
type = "static"
other = true
power = 1
toughness = 1
```

A **planeswalker** (`type = "planeswalker"`, with a starting `loyalty`) is an
ally that fights with loyalty abilities instead of attacking or blocking:

- Each activated ability's cost is a loyalty change, like `{ loyalty = 1 }`
  for "+1" or `{ loyalty = -2 }` for "−2". You may use one loyalty ability per
  planeswalker per turn, on your own turn when you could cast a sorcery, and
  only if it has enough loyalty.
- Opponents' creatures can attack it instead of you, and damage to it
  (combat, or "any target" spells) removes loyalty. At 0 loyalty it goes to
  the graveyard.
- Its big last ability often gives you an **emblem**: `{ type = "emblem",
  power = 2, toughness = 2, keyword = "flying" }` is "You get an emblem with
  'Creatures you control get +2/+2 and have flying.'" An emblem lasts the rest
  of the game and can't be removed.

**Legendary** cards (`legendary = true`; planeswalkers always are) follow the
legend rule: if you control two with the same name, you choose one to keep
and the other goes to the graveyard.

`crates/magus-core/tests/fixtures/sample-pack.toml` has a planeswalker, a lord,
a {T} ability and a legendary creature.

Any card can have `flavor`: text shown in italics under the rules text, with no
effect on the game. TOML's triple-quoted strings work for several lines:

```toml
flavor = '''
"I knew I forgot something..."
'''
```

Triggered abilities (`when`) fire on `enters` or `deals_combat_damage_to_player`. With
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
| g | Browse every graveyard; cast a card marked ● (flashback) from yours |
| Enter (on the battlefield) | Activate an ability of the selected permanent (marked ●) |
| Esc | Cancel targeting, close a popup, or choose nothing when that's allowed |
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
→ {"type":"hello","name":"ann","protocol":6}
← {"type":"welcome","decks":[…]}
→ {"type":"join","room":"friday","deck":"ember-thorn"}
  (or your own deck: {"type":"join_custom","room":"friday","deck":{"name":"Burn","cards":{"blaze":4,"crag":56}}})
← {"type":"waiting","room":"friday"}
← {"type":"started","seat":0,"players":["ann","bob"]}
← {"type":"state","view":{…,"prompt":{"type":"priority","plays":[…]}}}
→ {"type":"act","version":12,"action":{"type":"cast","card":31,"target":{"kind":"player","id":1}}}
```

See `crates/magus-protocol/src/lib.rs` and `crates/magus-core/src/view.rs`.

## Rules implemented

- 20 life, 7-card hands, 60-card decks with at most 4 copies of any card but basic lands; the player who goes first skips their first draw
- Turn steps: untap, upkeep, draw, main, declare attackers, declare blockers, combat damage, main, end, cleanup (discard down to 7)
- One land per turn; mana is paid automatically from untapped lands
- Full priority and stack: instants at any time, responses to spells, counterspells, and spells that fizzle when their target is gone
- Combat: multiple blockers per attacker, damage assigned in block order
- Keywords: flying, reach, haste, vigilance, lifelink, deathtouch, defender
- Triggered abilities: "when this enters" and "whenever this deals combat damage to a player", optionally with an "if" about how the card was cast
- Returning creature cards from the graveyard to the battlefield or hand, and blinking (a blinked creature is a new object, so spells aimed at it fizzle)
- Putting a creature from your hand onto the battlefield without casting it, and casting spells from your graveyard (flashback), after which they're exiled
- Planeswalkers: loyalty abilities, attacking them, damage removing loyalty; emblems
- Activated abilities ({T}, mana) and static abilities ("other creatures you control get +1/+1")
- Legendary permanents and the legend rule
- Targets limited by whose they are ("creature you control", "spell an opponent controls")
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
