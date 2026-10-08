# AGENTS.md

Notes for AI coding agents (and anyone else) working in this repo. Read this
before changing things. For the player-facing overview see README.md; for open
work see TODO.md.

## What this is

A Magic: The Gathering-style card game, played in the terminal over TCP. It is
Rust (edition 2024), a Cargo workspace under `crates/`. The project owner is
new to TCGs: explain game-rules decisions in plain terms, and don't assume
MTG jargon is understood.

**Cards are original: don't copy real Magic cards** (their names, rules text
or art) into the card pool. Everything else should use the real game's
standard terms: keywords, mechanics (flying, flashback…), zones, steps and
rules concepts. Players will read online guides, and invented synonyms make
those confusing. Be pragmatic about this rule: it's about not copying cards,
not about avoiding every word Magic uses.

## Layout

| Crate | Role |
|---|---|
| `magus-core` | Rules engine. **No I/O, no async, no networking.** Deterministic given a seed. |
| `magus-protocol` | `ClientMsg`/`ServerMsg`, JSON-lines framing, `connect()` helper. |
| `magus-server` | Lobby (rooms → pair two players), one tokio task per game owning the `Game`. Lib + bin. |
| `magus-tui` | The `magus` binary: ratatui 0.29 / crossterm 0.28. `app.rs` = game state + input, `ui.rs` = drawing only (its card helpers are shared). `builder.rs` = `magus deck-builder` and `cardbuilder.rs` = `magus card-builder` (each: state, then read-only drawing), `deckfile.rs` = deck files. |
| `magus-bot` | AI client. Lib (`run`, `choose`) + bin. Also hosts the end-to-end test. |

Key files in core:
- `game.rs`: the engine. `Game::apply(player, Action)` is the only way to
  mutate. `Game::view(player)` produces the filtered `GameView`.
- `view.rs`: everything clients see, including `Prompt`, the list of legal
  choices.
- `cards.rs`: the card pool (`static CARDS`) and decklists (`static DECKS`).
- `card.rs`: `CardDef`, `Effect`, `Ability`/`Trigger`, `Who`, `Keyword`,
  rules-text generation.
- `mana.rs`: costs, plus `plan_payment` (auto-tap).
- `pool.rs`: `CardPool` (built-in cards plus card packs) and the `Pack` data
  types packs are read from and written to. `Game::new` takes a pool and each
  player's decklist entries (card key, copies). `CardPool::decklist_problems`
  is the one deck check (60 cards, known cards, at most 4 copies but basic
  lands), used for pack decks, the deck builder and `ClientMsg::JoinCustom`.
  Parsing TOML lives in `magus-server` (`load_pack`/`load_pool`), not in core.

## Core design rules (don't break these)

1. **The server is the only authority; clients know no rules.** Every legal
   choice must be listed in `Prompt`. If you add a new kind of decision, add a
   `Pending` variant in `game.rs` *and* a `Prompt` variant in `view.rs`, then
   handle it in the TUI (`app.rs` `set_view`/`on_game_key`, `ui.rs`) and in the
   bot (`choose` and `fallback`).
2. **Hidden information stays hidden.** `view(p)` must never include another
   player's hand or library contents or order. There's a test
   (`hidden_information_stays_hidden`); extend it when adding zones.
3. **Validate by membership.** Actions are checked against the same functions
   that build prompts (`legal_plays`, `attack_options`, `block_options`,
   `valid_targets`), so prompts and validation can't drift apart. Keep it that
   way.
4. **`apply` → `settle()`.** After each action the engine runs state-based
   actions (`check_state`) and auto-passes priority for anyone with no legal
   play. This means:
   - Tests must not assume a player gets priority back after casting; they may
     have been auto-passed. Use `waiting_on()`.
   - `settle` has a loop guard. If it trips, something isn't making progress.
5. **Versioning.** `Game::version` increments per accepted action; clients
   quote it in `ClientMsg::Act`, and the server rejects stale ones. Bump
   `PROTOCOL_VERSION` for any wire-incompatible change.
6. **Card invariants** live in `CardDef::problems` and `CardPool::problems`,
   and apply equally to built-in cards and packs: at most one targeted effect
   per spell; spells use `effects` and permanents use `abilities`, never the
   other way round; triggered abilities never target; all decks are 60 cards
   of known keys. Add new rules there, not in tests, so packs get them too.
   Pack data is untrusted: never `panic!` on it (e.g. use
   `ManaCost::try_parse`), and keep `deny_unknown_fields` so typos are errors.
7. **Card behavior is composed, not named.** Rules-changing mechanics (flying,
   trample…) are `Keyword`s, checked where the rule applies. Everything of the
   form "when X, do Y" is a `Trigger` plus `Effect`s, and effects take a `Who`
   rather than baking in the player they affect. Add a new mechanic as a
   reusable building block, never as code for one specific card: cards will
   be loaded from card packs and edited in a card builder. A new variant
   changes the pack format: document it in README's "Card packs" section. The
   serialized shape is pinned by tests in `card.rs`.
8. **The card builder learns the vocabulary from core.** Its effect editor
   edits an effect's serialized form and offers `Effect::examples()` plus the
   `ALL` lists (`Keyword`, `Who`, `Whose`, `Trigger`, `CastZone`). Tests fail
   if a new variant is missing from them. A new kind of *field* (not just a
   new value) also needs its choices in `field_options` in `cardbuilder.rs`
   (`null` in a choice list means "any"/"none").

## Engine notes

- Library top = end of `Vec`. `ObjectId`s are assigned *after* shuffling so
  they leak no card info.
- Objects keep their `ObjectId` across zone changes, but `Object::moves`
  counts them. In the real rules a card that changes zones is a new object;
  we get the same effect by recording the target's `moves` on the stack item
  (`target_moves`) and treating a mismatch as "target gone"
  (`target_still_legal`). So any zone change, including battlefield →
  battlefield (blink), must go through `move_to`, which bumps the count,
  resets per-battlefield state and removes the object from combat. `draw`
  bumps it too.
- `valid_targets(spec, caster)` takes the caster, because `TargetSpec::whose`
  (`anyone`/`you`/`opponent`) is relative to whoever controls the spell.
- Resolution can pause for a choice. The top stack item stays on the stack
  while it resolves; `apply_effects(item, from)` asks `choice_options` whether
  an effect needs cards chosen (`put_from_hand`, `search`), stores
  `Choosing { item, effect, options, max }` and sets `Pending::Choose`.
  `Action::ChooseCards` (validated against `options`/`max`) runs
  `finish_choice`, resumes from the next effect, then `finish_resolving`
  removes the item and gives priority. Triggers wait while choosing. New
  choosing effects add a case to `choice_options` and `finish_choice`.
- `Prompt::ChooseCards` carries full `CardView`s because options can be in a
  hidden zone (a library during a search). It's only ever sent to the
  chooser. Options are listed by name, never in library order, and every
  search shuffles afterwards.
- Cast options: `legal_plays` lists one `PlayOption` per way to cast a card
  (`cast_ways`: normally, plus each cast option × each distinct card key its
  search could find), with that way's `cost`. `Action::Cast { way }` must
  match one exactly. Copies of a card are identical, so the engine fetches
  any copy and no mid-cast choice is needed.
- Casting from the graveyard: `legal_plays` also offers graveyard cards with
  `flashback`, `cast_cost` picks the cost by zone, and `spell_done` sends a
  spell cast from the graveyard to exile instead (also when it fizzles or is
  countered). Exile is public and appears in `PlayerView`.
- Card conservation in `fuzz.rs` counts every zone; add new zones there.
- Stack items: spells use the card's id; triggered abilities get a fresh id
  that is *not* in `objects`. Use `item.source` for the card, and
  `StackKind::Ability(i)` for which of its `abilities` is resolving.
- Triggers: things that happen call `Game::fire(Event)`, which records each
  matching ability (if its `only_if` holds) in `self.triggered`. They go on
  the stack in `settle` (`stack_triggers`), the next time anyone would get
  priority, in APNAP order. This is why abilities of a creature that dies in
  the same combat still trigger: they're recorded before state-based
  actions run. To add a trigger: a `Trigger` variant, an `Event` variant if
  needed, a `fire` call where it happens, and the match arm in `fire`. Set
  `Trigger::has_player` if effects may use `Who::ThatPlayer`.
- Characteristics are computed, never read off the card: use
  `Game::power`/`toughness`/`has_keyword`, which add static abilities and
  emblems (`BoostSource`s) to the printed values and "until end of turn"
  changes. Hot paths (`check_state`, `view`, attack/block options) collect
  `boost_sources()` once and use the `*_from` variants: calling the plain
  versions per creature made the fuzz test twice as slow.
- Counters live in `Object::counters` (loyalty so far); `move_to` clears
  them and gives a planeswalker its starting loyalty when it enters. Damage
  to a planeswalker removes loyalty; `check_state` puts one at 0 into the
  graveyard.
- Activated abilities are plays: `legal_plays` adds one `PlayKind::Ability(i)`
  per usable ability (`ability_plays`, `can_pay`), and `Action::Activate`
  must match one. Costs are paid at once (loyalty, tapping, mana); the
  ability goes on the stack as `StackKind::Ability(i)` and may target.
  Loyalty abilities: sorcery speed, once per planeswalker per turn.
- `Attack::planeswalker` attacks a planeswalker; `defender` is then its
  controller (who blocks). Damage to a planeswalker that has left goes nowhere.
- Pending choices (`Choosing`) are either a paused resolution or the legend
  rule (`ChoiceFor`), which `check_state` raises and `choose` resolves,
  restoring what was pending before.
- `Object::cast_from` is set by `cast()` and cleared by `move_to` for any move
  except stack → battlefield, so bounced-and-recast cards don't remember.
- Combat damage is computed first, then applied, so it's simultaneous.
  Attackers assign damage to blockers in block order.
- Mana is paid automatically; basic lands only, so greedy payment is exact.
- Multiplayer is partially built: turn order, priority rotation and
  per-defender attacks/blocks all handle N players. The lobby only seats two.

## Client notes

- The TUI auto-passes "routine" priority client-side (`App::wants_stop`),
  separate from the engine's no-legal-play auto-pass. `f` toggles it.
- `ui.rs` must stay read-only over `App`.
- `magus --solo` starts `magus_server::serve` and `magus_bot::run` in-process
  on a random local port (`start_solo` in `magus-tui/src/main.rs`). For that
  reason the server and bot libraries log through `tracing`, **never
  `eprintln!`**: only their own binaries install a subscriber
  (`tracing-subscriber`, `RUST_LOG` / `EnvFilter`, default `info`), so stray
  output can't corrupt the TUI.
- The TUI uses `ratatui::init()/restore()`; don't print to stdout while it
  runs.

## Commands

```sh
cargo test --workspace                       # all tests (~2s)
cargo clippy --workspace --all-targets       # keep at zero warnings
cargo fmt --all                              # rustfmt defaults
cargo run -p magus-server -- --bind 127.0.0.1:7878 [--cards pack.toml]
cargo run -p magus-bot -- --room practice --delay-ms 0
cargo run -p magus-tui -- --room practice    # needs a real terminal
cargo run -p magus-tui -- --solo             # one-command game vs. the bot
cargo run -p magus-tui -- deck-builder --deck my-deck.toml
cargo run -p magus-tui -- --solo --deck my-deck.toml
```

Testing layers:
- `magus-core` unit tests in `game.rs` use `blank_game()` + `put()` to set up
  exact board states. Prefer this for new rules.
- `magus-core/tests/fuzz.rs` plays 300 random games using only the prompts and
  checks card conservation, across the built-in decks plus the deck in
  `tests/fixtures/sample-pack.toml`. Run it after any engine change. Add new
  mechanics to that pack so the fuzzer exercises them.
- `magus-bot/tests/e2e.rs` runs a real server plus two bots over TCP.
- `magus-tui` renders into `TestBackend` (see `main.rs` tests). Use it to check
  layout without a terminal; no tmux is available on the dev machine.

## Conventions

- Match the existing style: small focused functions, comments explain *why*,
  doc comments on public items.
- Errors to players are `ActionError` strings and should be readable by a
  human.
- Don't add dependencies to `magus-core` beyond `serde`/`rand` without a good
  reason; it should stay embeddable (e.g. WASM for a web client).
- Commit or push only when asked.
