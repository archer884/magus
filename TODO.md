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
- [ ] **Activated abilities** ("{T}: deal 1 damage…"), and tapping mana by
      hand when the auto-payer picks badly. Auto-pay is fine for basic lands
      only; dual lands or mana creatures would need a real solver.
- [ ] **Targeted triggers** (e.g. "when this enters, destroy target creature").
      Creature triggers can't target today, because the trigger would need a
      new prompt to choose its target.
- [ ] **Static effects and auras/equipment** (layers-lite).
- [ ] **Tokens, counters (+1/+1), exile, "dies" triggers.**
- [ ] **Hexproof / shroud / protection** (they affect `valid_targets`).
- [ ] Zone changes keep the same `ObjectId`. Real MTG makes a new object, which
      would fix edge cases such as a spell still targeting a creature that was
      bounced and recast.

## Multiplayer & Commander

- [ ] Lobby support for 3–4 player rooms (the engine already takes N players).
- [ ] Commander format: 100-card singleton decks, 40 life, a command zone, a
      commander tax (+2 to recast each time), 21 commander combat damage
      knocks a player out, and a color-identity check at deck validation.
- [ ] Legendary creatures and the "legend rule."
- [ ] TUI layout for several opponents. It currently stacks opponent panels
      vertically, which won't fit 3 opponents on small terminals.

## Cards as data

- [ ] **Load the card pool and decklists from data files** instead of the
      `static CARDS` array in `magus-core/src/cards.rs`. Probably TOML or JSON
      (rich enough for effects); CSV only fits flat fields. A sketch:
      ```toml
      [[card]]
      key = "blaze"
      name = "Blaze"
      cost = "1R"
      kind = "instant"
      effects = [{ damage = { amount = 3, target = "any" } }]
      ```
      `CardDef` currently uses `&'static` fields, so either leak the loaded data
      (`Box::leak`, which is fine for a process-lifetime card pool) or switch to
      owned `String`s/`Arc<CardDef>`. Validate on load: one targeted effect per
      spell, no targeted creature effects, and decklists that reference real
      keys. Those checks are unit tests today; see `cards.rs` tests.
- [ ] Ship the card pool with the server and send card definitions to clients
      (pairs with the protocol work below).
- [ ] A small effect DSL as the card pool outgrows the `Effect` enum.

## Server: persistence, accounts, trading

- [ ] **Database** (SQLite via `sqlx` or `rusqlite` to start; Postgres-ready).
      Tables: players/accounts, card collection (player × card × quantity),
      saved decks, match history, trade offers.
- [ ] **Accounts & auth.** Today `hello` carries a free-form name. Add
      register/login (argon2 password hashes, session tokens). Bump
      `PROTOCOL_VERSION`.
- [ ] **Collections & deck building.** Players own cards; decks are built from
      the collection and validated server-side (60-card minimum, ≤4 copies,
      format rules).
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

- [ ] Mulligan, reconnect and account screens in the TUI.
- [ ] Configurable auto-pass "stops" (which steps you want to be asked at).
      Currently hard-coded in `App::wants_stop`.
- [ ] Graveyard viewer; card zoom; showing the opponent's last-played card.
- [ ] In-game chat.
- [ ] A smarter bot (it currently plays greedily and never bluffs).
- [ ] A web client: the protocol is already rules-free, so it's mostly rendering.
- [ ] Spectator mode.

## Project hygiene

- [ ] Initialize git; add CI (fmt, clippy `-D warnings`, test).
- [ ] Balance pass on the card pool, e.g. run bot-vs-bot tournaments across all
      deck pairs and compare win rates.
- [ ] A license.
