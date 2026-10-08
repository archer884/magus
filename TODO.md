# TODO

Roughly in priority order within each section. See [AGENTS.md](AGENTS.md) for
how the codebase fits together before picking something up.

## Gameplay rules

- [ ] **Mulligans.** A hand with no lands gets auto-passed for turns at a time.
      Add the London mulligan: draw 7, put N on the bottom. The engine needs a
      new `Pending::Mulligan` decision before turn 1.
- [ ] **First strike / double strike.** These need a second combat-damage step.
- [ ] **Trample**, plus letting the attacker choose damage assignment order
      (currently it's block order, and assignment is automatic).
- [x] Activated abilities (loyalty, {T}, mana costs) and planeswalkers.
- [ ] Tapping mana by hand when the auto-payer picks badly. Auto-pay is fine
      for basic lands only; dual lands or mana abilities ("{T}: add {G}" on a
      creature) would need a real solver.
- [ ] **Targeted triggers** (e.g. "when this enters, destroy target creature").
      Creature triggers can't target today, because the trigger would need a
      new prompt to choose its target.
- [x] Static abilities and emblems: creatures get +X/+Y and/or a keyword
      (`Boost`), computed through `Game::power`/`toughness`/`has_keyword`.
- [ ] More static effects beyond boosts (boosts can already be negative or
      aimed at opponents' creatures): "can't block", cost reductions, and
      auras/equipment (attached to one creature).
- [ ] Emblems with triggered abilities ("At the beginning of your upkeep,
      draw a card"), which need more triggers.
- [ ] **Tokens, +1/+1 counters (reuse `Counter`), "dies" triggers.**
- [ ] **Hexproof / shroud / protection** (they affect `valid_targets`).
- [x] A card that changes zones is treated as a new object for targeting
      (`Object::moves`), so blink and bounce-and-recast make spells fizzle.
      Its `ObjectId` is still reused, which is fine for everything so far.

## Multiplayer & Commander

- [ ] Lobby support for 3–4 player rooms (the engine already takes N players).
- [ ] Commander format: 100-card singleton decks, 40 life, a command zone, a
      commander tax (+2 to recast each time), 21 commander combat damage
      knocks a player out, and a color-identity check at deck validation.
- [x] Legendary permanents and the legend rule (the player chooses).
- [ ] TUI layout for several opponents. It currently stacks opponent panels
      vertically, which won't fit 3 opponents on small terminals.

## Cards as data

- [x] **Flavor text** (`flavor`), shown in italics under the rules text.
- [x] **Card builder** (`magus card-builder --pack pack.toml`).
- [ ] Card builder: multi-line flavor text (its text field is one line; TOML
      files can have several), and editing a pack's decks.
- [ ] **Card art** in packs: an optional `art` field (ASCII art inline, or a
      path to an image). See the card view under Clients.
- [x] Card packs: extra cards and decks loaded from TOML (`--cards`), added
      to the built-in pool (`magus-core/src/pool.rs`). Built-in cards stay in
      Rust for now.
- [x] Event-driven triggers (`Game::fire` → `stack_triggers` in `settle`),
      `DealsCombatDamageToPlayer`, `Who::ThatPlayer`, `LoseGame`, and
      `Condition` on how a permanent was cast. Test pack: `blightmaw.toml`.
- [ ] More triggers: "dies" (needs the dying card's abilities to be read
      from the graveyard), "attacks", "beginning of your upkeep", and
      "another creature enters" (the `Event` match in `fire` already scans
      the whole battlefield).
- [ ] When one player has several triggers at once, let them choose the
      order (a new `Prompt`). They currently resolve in the order they
      triggered.
- [x] Alternate paths, first slice: target a creature card in your
      graveyard; `return_to_battlefield`, `return_to_hand`, `blink`. Built-in
      cards: Call from the Mire, Gravecall Wraith, Mossgrave Recovery,
      Veilstep.
- [x] Alternate paths, second slice: flashback (cast from the graveyard, then
      exile), an exile zone, `put_from_hand` with a mid-resolution choice
      (`Pending::Choose`), and `whose` target restrictions. Built-in cards:
      Sparkfall, Beckon the Wild; Veilstep now targets your own creature.
- [x] Color mana symbols in the TUI wherever costs appear.
- [x] Library search (`search`: filter, count, to hand / battlefield
      (tapped) / top of library), choosing several cards, and cast options
      ("as you cast this, you may search…; if you do, it costs less").
- [ ] **Tokens**: `create_token` with an inline token description (name,
      power/toughness, color, subtype, keywords, count). Tokens vanish when
      they leave the battlefield; the fuzz card count must skip them.
- [ ] More cast option actions: pay life (or Phyrexian-style {B/P} mana),
      discard a card, sacrifice a creature, tap creatures (convoke). Each is a
      choice made while casting.
- [ ] More choices during resolution: "choose a card in your graveyard",
      "choose one of these modes".
- [ ] Target filters beyond kind + whose (e.g. "creature with flying",
      "power 3 or less"). The search's `CardFilter` could grow these fields
      and be reused for targets.
- [ ] The bot only knows built-in cards (`cards::card`), so it never casts a
      pack's targeted spells. It needs card definitions from the server (see
      next item), or a hint in the prompt about what a spell does.
- [ ] Accept JSON packs as well as TOML (choose by file extension), for the
      card builder and a web client.
- [ ] Ship the card pool with the server and send card definitions to clients
      (pairs with the protocol work below).

## Server: persistence, accounts, trading

- [ ] **A server rule against outside cards** (e.g. `--no-custom-cards`).
      Today a server only knows its own pool, so a deck with a card it lacks
      is already refused. The flag matters once players can bring their own
      card definitions (uploading cards made in the card builder).
- [ ] **Database** (SQLite via `sqlx` or `rusqlite` to start; Postgres-ready).
      Tables: players/accounts, card collection (player × card × quantity),
      saved decks, match history, trade offers.
- [ ] **Accounts & auth.** Today `hello` carries a free-form name. Add
      register/login (argon2 password hashes, session tokens). Bump
      `PROTOCOL_VERSION`.
- [x] Deck builder (`magus deck-builder --deck deck.toml`), deck files, playing
      them with `--deck <file>` (`ClientMsg::JoinCustom`, checked against the
      server's pool), and the 4-copy limit.
- [ ] Build decks against a remote server's cards (`deck-builder --server`);
      needs the card catalog message above.
- [ ] **Collections.** Players own cards; decks are built from the collection
      and validated server-side (format rules too).
- [ ] **Trading.** Offer/accept/cancel between two players. It must be atomic in
      a DB transaction, so a card can't be both traded and kept.
- [ ] Card acquisition (starter collection, packs, rewards). A design question.
- [ ] **Reconnects.** If a client drops mid-game it currently concedes at once.
      Hold the seat for a grace period and let the player reattach with a
      session token.
- [ ] TLS (`tokio-rustls`), or document running behind a TLS-terminating proxy.
- [ ] Rate limiting / max connections; idle timeouts for players who never act
      (turn timer / chess clock).
- [ ] Spans per connection/game in the server logs (`tracing` is in place, but only events are emitted).

## Wire protocol efficiency

Measured on a 13-turn bot game: **86 state messages, ~650 KB total, ~7.7 KB
average, ~12.6 KB max** per message. Every change resends the *entire*
`GameView` to every player.

- [ ] **Stop resending static card data.** Each `CardView` repeats name, type
      line, rules text, colors and keywords. Send the card catalog once (or let
      clients cache by `key`) and reference cards by `key` + `ObjectId`. This
      is probably the biggest single win.
- [ ] **Log deltas.** Each view carries the last 100 log lines. Send only new
      lines (the client keeps history), or switch to structured events.
- [ ] **Graveyards** are resent in full every time. Send counts, plus contents
      on demand or as deltas.
- [ ] **Diffs or events instead of snapshots.** Send `GameEvent`s
      (CardMoved, LifeChanged, Tapped, …) plus the current prompt, with a full
      snapshot only on join or when a client asks for a resync. More complex:
      clients must apply events correctly. Snapshot + version numbers make
      resync easy.
- [ ] **Fewer messages.** The engine auto-passes internally, but each accepted
      action still broadcasts to everyone. Consider coalescing, and skip sending
      a player a view identical to their last one.
- [ ] **Encoding/compression.** Once payloads are smaller, consider
      per-connection deflate/zstd, or a binary format (`postcard`/MessagePack)
      negotiated in `hello`. Keep JSON available for debugging and third-party
      clients.
- [ ] Profile server CPU too: `Game::view` rebuilds everything and recomputes
      `legal_plays` per player per change. Cheap at 2 players, but worth caching
      for 4-player Commander on small hardware.

## Clients

- [ ] **Card editor**: currently, it's basically not possible to add an on-cast
      effect that isn't EXACTLY the one that we discussed. Like, it'll complain
      that the cost reduction requires the effect to be a search or whatever--
      which is just kind of silly, given that cost reductions usually involve
      some other mechanic entirely.
- [ ] **Card view**: an inspect screen that draws a whole "virtual card"
      (name and cost bar, art box, type line, rules text, flavor text,
      power/toughness). Art is optional: ASCII art from the pack, or a real
      image on terminals that support one (kitty/iTerm2/sixel graphics, e.g.
      the way `viu` does it), falling back to the ASCII art or nothing.
- [ ] Mulligan, reconnect and account screens in the TUI.
- [ ] Configurable auto-pass "stops" (which steps you want to be asked at).
      Currently hard-coded in `App::wants_stop`.
- [ ] Card zoom; showing the opponent's last-played card; an exile viewer
      (the graveyard viewer, `g`, could grow a tab for it).
- [ ] In-game chat.
- [ ] A smarter bot (it currently plays greedily and never bluffs).
- [ ] A web client: the protocol is already rules-free, so it's mostly rendering.
- [ ] Spectator mode.

## Project hygiene

- [ ] Initialize git; add CI (fmt, clippy `-D warnings`, test).
- [ ] Balance pass on the card pool, e.g. run bot-vs-bot tournaments across all
      deck pairs and compare win rates.
- [ ] A license.
