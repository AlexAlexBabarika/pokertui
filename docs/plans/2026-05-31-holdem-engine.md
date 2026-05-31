# Texas Hold'em Engine + Hot-Seat Wiring — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

## Context

Phase 1 of this project produced a static TUI demo: `crates/poker-core` has `Card`, `Suit`, `Rank`, `Deck`, and a `evaluate()` function backed by `rs_poker`. `crates/pokertui` renders a hand of poker with `ratatui`, but the state shown is a hardcoded `GameState::demo()` — there is no game logic, only a frozen picture.

This plan adds the real rules engine for No-Limit Texas Hold'em and wires it to the existing renderer so a single terminal can play hot-seat (pass-and-play). The engine lives in `poker-core` as a pure, deterministic state machine: phases (Preflop → Flop → Turn → River → Showdown), per-phase betting rounds, dealer button rotation, blinds, and a `(state, action) → state` transition. The engine is the template Omaha will later reuse; Omaha itself is **out of scope** for this plan.

The hardest correctness problem in poker is **side pots** — when two or more players go all-in for different amounts, the pot must be split into layers so each player can only win the chips of opponents who matched their wager. Targeted property and example tests for this are budgeted as their own task.

**Goal:** Implement a deterministic, ID-based No-Limit Hold'em state machine in `poker-core` and drive it from the existing TUI via key actions, so two-to-nine human players can play a complete hand on one terminal.

**Architecture:** A pure `poker-core::holdem` module exposes a `HandState` struct, an `Action` enum, `legal_actions(&state)`, and `apply(state, action) -> Result<state, _>`. The engine is ID-based (players are `0..N`) and seed-deterministic. `pokertui` owns display strings (names, position labels) in a new `App` struct that holds the engine state, a `NameRegistry`, and a derived presentation `GameState`. Key events in `main.rs` map to engine actions; after each transition the adapter rebuilds the presentation `GameState` and re-renders.

**Tech Stack:** Rust 2024, `rand` for shuffling, `rs_poker` for the 5-card hand evaluator (already integrated), `ratatui` + `crossterm` for the TUI (already integrated). No new dependencies.

**Design decisions (locked):**

- No-Limit Hold'em only (no fixed-limit branch).
- Engine stores numeric `PlayerId` (a `usize` wrapper); names and position labels (UTG/MP/CO/BTN/SB/BB) live in the UI layer.
- 2 to 9 seats supported; demo defaults to 6. Heads-up uses the standard "SB-on-button" rule.
- Chip amounts are `u64`. No floating point in the engine.
- Deck is shuffled inside `HandState::new_hand(seed, ...)`; same seed → same hand.
- One action at a time. Round closure is computed after every `apply()`. The caller never asks "is the round over" — it just keeps calling `apply()` and watches for `phase` to advance.

---

## File Structure

### Created

| File | Responsibility |
| --- | --- |
| `crates/poker-core/src/holdem/mod.rs` | Module root + public re-exports |
| `crates/poker-core/src/holdem/types.rs` | `PlayerId`, `Phase`, `Action`, `HandConfig`, errors |
| `crates/poker-core/src/holdem/state.rs` | `HandState`, `BettingRound`, construction, accessors |
| `crates/poker-core/src/holdem/transitions.rs` | `legal_actions`, `apply`, round-closure logic, phase advance |
| `crates/poker-core/src/holdem/pots.rs` | Side-pot layering, eligibility, settlement |
| `crates/poker-core/tests/holdem_walkthrough.rs` | Integration test: scripted full hands |
| `crates/poker-core/tests/holdem_side_pots.rs` | Integration test: multi-way all-in scenarios |
| `crates/pokertui/src/app.rs` | `App { engine, names, view }`, key→action mapping |
| `crates/pokertui/src/adapter.rs` | Builds presentation `GameState` from `HandState` + names |

### Modified

| File | Change |
| --- | --- |
| `crates/poker-core/src/lib.rs` | Add `pub mod holdem;` and re-exports |
| `crates/poker-core/Cargo.toml` | No change (already has `rand`, `rs_poker`) |
| `crates/pokertui/src/state.rs` | Switch `&'static str` fields on `Seat`, `LogEntry`, `ChatLine`, `Phase` to `String`; keep `demo()` (port to `String::from`) |
| `crates/pokertui/src/ui.rs` | Mechanical: adjust spans that consumed `&'static str` to take `&str` slices |
| `crates/pokertui/src/main.rs` | Replace `GameState::demo()` with `App::new_demo_hand()`; dispatch keys (F/C/R/A/↑/↓/⏎) to `App::handle_key` |

---

## Public API Sketch (locked before Task 1)

```rust
// poker-core/src/holdem/types.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Fold,
    Check,
    Call,
    Bet { to: u64 },     // open a round; `to` is the new round-level wager
    Raise { to: u64 },   // raise an existing bet; `to` is the new round-level wager
    AllIn,               // shorthand: shove the actor's full stack
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandConfig {
    pub num_players: usize,    // 2..=9
    pub small_blind: u64,
    pub big_blind: u64,
    pub dealer: PlayerId,      // button position
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    NotYourTurn,
    IllegalAction(&'static str),
    HandComplete,
}
```

```rust
// poker-core/src/holdem/state.rs

#[derive(Debug, Clone)]
pub struct HandState {
    pub config: HandConfig,
    pub phase: Phase,
    pub board: Vec<Card>,                  // 0, 3, 4, or 5 cards
    pub hole: Vec<[Card; 2]>,              // indexed by PlayerId.0
    pub stacks: Vec<u64>,                  // remaining chips per player
    pub contributed: Vec<u64>,             // total chips put in this hand
    pub round_bet: Vec<u64>,               // chips put in this betting round
    pub folded: Vec<bool>,
    pub all_in: Vec<bool>,
    pub to_act: Option<PlayerId>,
    pub current_bet: u64,                  // max round_bet this round
    pub min_raise: u64,                    // size of the last full raise
    pub last_aggressor: Option<PlayerId>,  // last to bet/raise; needed for round closure
    pub log: Vec<LogEntry>,                // append-only history (engine-level)
    pub pots: Vec<Pot>,                    // populated at showdown
    pub winners: Vec<Vec<PlayerId>>,       // winners[pot_idx]
    deck: Vec<Card>,                       // remaining cards (private)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub actor: Option<PlayerId>,
    pub kind: LogKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    PostBlind { amount: u64, is_big: bool },
    Action(Action),
    DealFlop([Card; 3]),
    DealTurn(Card),
    DealRiver(Card),
    Showdown,
    WinPot { pot_idx: usize, amount: u64 },
}
```

```rust
// poker-core/src/holdem/transitions.rs
pub fn legal_actions(state: &HandState) -> Vec<Action>;
pub fn apply(state: HandState, action: Action) -> Result<HandState, (HandState, ApplyError)>;
```

```rust
// poker-core/src/holdem/pots.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pot {
    pub amount: u64,
    pub eligible: Vec<PlayerId>,  // sorted ascending
}

pub fn build_pots(contributed: &[u64], folded: &[bool]) -> Vec<Pot>;
pub fn settle(state: &HandState) -> (Vec<Pot>, Vec<Vec<PlayerId>>, Vec<u64>);
```

---

## Tasks

### Task 1: Module skeleton and types

**Files:**
- Create: `crates/poker-core/src/holdem/mod.rs`
- Create: `crates/poker-core/src/holdem/types.rs`
- Modify: `crates/poker-core/src/lib.rs:1`

- [ ] **Step 1: Write failing test**

Create `crates/poker-core/src/holdem/types.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preflop, Flop, Turn, River, Showdown, Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Fold,
    Check,
    Call,
    Bet { to: u64 },
    Raise { to: u64 },
    AllIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandConfig {
    pub num_players: usize,
    pub small_blind: u64,
    pub big_blind: u64,
    pub dealer: PlayerId,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    NotYourTurn,
    IllegalAction(&'static str),
    HandComplete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_distinct_identity() {
        assert_ne!(Phase::Preflop, Phase::Flop);
        assert_ne!(Phase::Showdown, Phase::Complete);
    }

    #[test]
    fn action_equality_uses_amount() {
        assert_eq!(Action::Bet { to: 100 }, Action::Bet { to: 100 });
        assert_ne!(Action::Bet { to: 100 }, Action::Bet { to: 200 });
    }
}
```

Create `crates/poker-core/src/holdem/mod.rs`:

```rust
pub mod types;

pub use types::{Action, ApplyError, HandConfig, Phase, PlayerId};
```

Modify `crates/poker-core/src/lib.rs` — add at the top of the file (line 1):

```rust
pub mod holdem;
```

- [ ] **Step 2: Run to verify**

Run: `cargo test -p poker-core holdem::types -- --nocapture`
Expected: PASS, 2 tests run.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/ crates/poker-core/src/lib.rs
git commit -m "feat(holdem): add core types and module skeleton"
```

---

### Task 2: HandState construction and initial deal

**Files:**
- Create: `crates/poker-core/src/holdem/state.rs`
- Modify: `crates/poker-core/src/holdem/mod.rs`

This task is split into TDD micro-steps because construction is where most off-by-one bugs hide.

- [ ] **Step 1: Write the failing test**

Append to `crates/poker-core/src/holdem/state.rs`:

```rust
use crate::Card;
use crate::Deck;
use super::types::{HandConfig, LogEntry, LogKind, Phase, PlayerId};
use super::pots::Pot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub actor: Option<PlayerId>,
    pub kind: LogKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    PostBlind { amount: u64, is_big: bool },
    Action(super::types::Action),
    DealFlop([Card; 3]),
    DealTurn(Card),
    DealRiver(Card),
    Showdown,
    WinPot { pot_idx: usize, amount: u64 },
}

#[derive(Debug, Clone)]
pub struct HandState {
    pub config: HandConfig,
    pub phase: Phase,
    pub board: Vec<Card>,
    pub hole: Vec<[Card; 2]>,
    pub stacks: Vec<u64>,
    pub contributed: Vec<u64>,
    pub round_bet: Vec<u64>,
    pub folded: Vec<bool>,
    pub all_in: Vec<bool>,
    pub to_act: Option<PlayerId>,
    pub current_bet: u64,
    pub min_raise: u64,
    pub last_aggressor: Option<PlayerId>,
    pub log: Vec<LogEntry>,
    pub pots: Vec<Pot>,
    pub winners: Vec<Vec<PlayerId>>,
    pub(crate) deck: Vec<Card>,
}

impl HandState {
    /// Build a new hand: shuffle deck (deterministically), post blinds, deal
    /// two hole cards per player, set `to_act` to the player UTG.
    pub fn new_hand(config: HandConfig, stacks: Vec<u64>) -> Self {
        assert!(
            (2..=9).contains(&config.num_players),
            "num_players must be in 2..=9"
        );
        assert_eq!(stacks.len(), config.num_players, "stacks must match num_players");
        assert!(config.big_blind > 0, "big blind must be positive");
        assert!(config.small_blind > 0 && config.small_blind < config.big_blind, "SB in (0, BB)");
        assert!(config.dealer.0 < config.num_players, "dealer index in range");

        let n = config.num_players;
        let mut deck = Deck::new();
        deck.shuffle_with_seed(config.seed);
        let mut deck: Vec<Card> = deck.cards().to_vec();

        // Deal 2 hole cards per player, alternating (real poker dealing order).
        let mut hole: Vec<[Card; 2]> = Vec::with_capacity(n);
        for _ in 0..n {
            hole.push([deck.remove(0), Card::new(crate::Rank::Two, crate::Suit::Clubs)]);
        }
        for p in 0..n {
            hole[p][1] = deck.remove(0);
        }

        // Blind positions.
        // Heads-up: dealer is SB, the other is BB, dealer acts first preflop.
        // 3+: SB = dealer+1, BB = dealer+2, UTG = dealer+3.
        let (sb_idx, bb_idx, first_to_act) = if n == 2 {
            (config.dealer.0, (config.dealer.0 + 1) % n, config.dealer.0)
        } else {
            let sb = (config.dealer.0 + 1) % n;
            let bb = (config.dealer.0 + 2) % n;
            let utg = (config.dealer.0 + 3) % n;
            (sb, bb, utg)
        };

        let mut stacks = stacks;
        let mut contributed = vec![0u64; n];
        let mut round_bet = vec![0u64; n];
        let mut all_in = vec![false; n];
        let mut log: Vec<LogEntry> = Vec::new();

        // Post small blind (capped at stack).
        let sb_amount = config.small_blind.min(stacks[sb_idx]);
        stacks[sb_idx] -= sb_amount;
        contributed[sb_idx] += sb_amount;
        round_bet[sb_idx] += sb_amount;
        if stacks[sb_idx] == 0 { all_in[sb_idx] = true; }
        log.push(LogEntry {
            actor: Some(PlayerId(sb_idx)),
            kind: LogKind::PostBlind { amount: sb_amount, is_big: false },
        });

        // Post big blind (capped at stack).
        let bb_amount = config.big_blind.min(stacks[bb_idx]);
        stacks[bb_idx] -= bb_amount;
        contributed[bb_idx] += bb_amount;
        round_bet[bb_idx] += bb_amount;
        if stacks[bb_idx] == 0 { all_in[bb_idx] = true; }
        log.push(LogEntry {
            actor: Some(PlayerId(bb_idx)),
            kind: LogKind::PostBlind { amount: bb_amount, is_big: true },
        });

        HandState {
            config,
            phase: Phase::Preflop,
            board: Vec::new(),
            hole,
            stacks,
            contributed,
            round_bet,
            folded: vec![false; n],
            all_in,
            to_act: Some(PlayerId(first_to_act)),
            current_bet: bb_amount,
            min_raise: config.big_blind,
            last_aggressor: Some(PlayerId(bb_idx)), // BB is the "aggressor" until someone raises
            log,
            pots: Vec::new(),
            winners: Vec::new(),
            deck,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdem::types::{HandConfig, PlayerId};

    fn cfg(n: usize) -> HandConfig {
        HandConfig {
            num_players: n,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 0xC0FFEE,
        }
    }

    #[test]
    fn six_handed_new_hand_posts_blinds_and_sets_to_act() {
        let s = HandState::new_hand(cfg(6), vec![10_000; 6]);
        assert_eq!(s.phase, Phase::Preflop);
        assert_eq!(s.stacks[1], 9_950, "SB posts 50");
        assert_eq!(s.stacks[2], 9_900, "BB posts 100");
        assert_eq!(s.contributed[1], 50);
        assert_eq!(s.contributed[2], 100);
        assert_eq!(s.current_bet, 100);
        assert_eq!(s.min_raise, 100);
        assert_eq!(s.to_act, Some(PlayerId(3)), "UTG acts first preflop");
        assert_eq!(s.hole.len(), 6);
        assert_eq!(s.board.len(), 0);
    }

    #[test]
    fn heads_up_dealer_is_sb_and_acts_first() {
        let s = HandState::new_hand(cfg(2), vec![10_000; 2]);
        assert_eq!(s.stacks[0], 9_950, "dealer/SB posts 50 in HU");
        assert_eq!(s.stacks[1], 9_900, "BB posts 100");
        assert_eq!(s.to_act, Some(PlayerId(0)), "dealer/SB acts first preflop in HU");
    }

    #[test]
    fn same_seed_deals_identical_hole_cards() {
        let a = HandState::new_hand(cfg(6), vec![10_000; 6]);
        let b = HandState::new_hand(cfg(6), vec![10_000; 6]);
        assert_eq!(a.hole, b.hole);
    }

    #[test]
    fn hole_cards_are_all_distinct() {
        let s = HandState::new_hand(cfg(9), vec![10_000; 9]);
        let mut seen: Vec<Card> = s.hole.iter().flat_map(|h| h.iter().copied()).collect();
        seen.sort_by_key(|c| (c.suit() as u8, c.rank() as u8));
        let unique = seen.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), seen.len(), "no duplicate hole cards");
    }

    #[test]
    fn short_stacked_blind_goes_all_in_on_post() {
        let mut stacks = vec![10_000u64; 6];
        stacks[1] = 30; // SB has only 30 chips
        let s = HandState::new_hand(cfg(6), stacks);
        assert_eq!(s.contributed[1], 30, "SB caps at stack");
        assert!(s.all_in[1]);
    }
}
```

Then update `crates/poker-core/src/holdem/mod.rs`:

```rust
pub mod pots;     // forward-declare for state.rs's `Pot` import
pub mod state;
pub mod types;

pub use state::{HandState, LogEntry, LogKind};
pub use types::{Action, ApplyError, HandConfig, Phase, PlayerId};
```

Create a stub `crates/poker-core/src/holdem/pots.rs` so the import resolves:

```rust
use super::types::PlayerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pot {
    pub amount: u64,
    pub eligible: Vec<PlayerId>,
}
```

- [ ] **Step 2: Run to verify**

Run: `cargo test -p poker-core holdem::state -- --nocapture`
Expected: 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/
git commit -m "feat(holdem): HandState construction, blinds, initial deal"
```

---

### Task 3: `legal_actions` for a fresh hand

**Files:**
- Create: `crates/poker-core/src/holdem/transitions.rs`
- Modify: `crates/poker-core/src/holdem/mod.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/poker-core/src/holdem/transitions.rs`:

```rust
use super::state::HandState;
use super::types::{Action, ApplyError, Phase, PlayerId};

/// Return every legal action for the player in `state.to_act`.
/// Returns empty if no one is to act (hand complete or between rounds).
pub fn legal_actions(state: &HandState) -> Vec<Action> {
    let Some(actor) = state.to_act else { return Vec::new(); };
    let i = actor.0;
    if state.folded[i] || state.all_in[i] {
        return Vec::new();
    }

    let mut out = Vec::new();
    out.push(Action::Fold);

    let to_call = state.current_bet.saturating_sub(state.round_bet[i]);
    if to_call == 0 {
        out.push(Action::Check);
    } else {
        out.push(Action::Call);
    }

    let stack = state.stacks[i];
    let max_to = state.round_bet[i] + stack;

    if state.current_bet == 0 {
        // No bet yet this round; minimum bet is BB.
        let min_to = state.config.big_blind.max(state.config.big_blind);
        if stack >= min_to {
            out.push(Action::Bet { to: min_to });
        }
    } else {
        // Raise: minimum total is current_bet + min_raise.
        let min_to = state.current_bet + state.min_raise;
        if max_to >= min_to {
            out.push(Action::Raise { to: min_to });
        }
    }

    if stack > 0 {
        out.push(Action::AllIn);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdem::state::HandState;
    use crate::holdem::types::{HandConfig, PlayerId};

    fn cfg6() -> HandConfig {
        HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        }
    }

    #[test]
    fn utg_facing_bb_can_fold_call_raise_or_shove() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        let actions = legal_actions(&s);
        assert!(actions.contains(&Action::Fold));
        assert!(actions.contains(&Action::Call));
        assert!(actions.contains(&Action::Raise { to: 200 }), "min-raise to 200");
        assert!(actions.contains(&Action::AllIn));
        assert!(!actions.iter().any(|a| matches!(a, Action::Check)),
            "cannot check facing the BB");
    }

    #[test]
    fn short_stacked_actor_cannot_raise_only_shove() {
        let mut stacks = vec![10_000u64; 6];
        stacks[3] = 150; // UTG has 150 — call costs 100, can't make min-raise (200)
        let s = HandState::new_hand(cfg6(), stacks);
        let actions = legal_actions(&s);
        assert!(actions.contains(&Action::Call));
        assert!(actions.contains(&Action::AllIn));
        assert!(!actions.iter().any(|a| matches!(a, Action::Raise { .. })),
            "stack too small for min-raise; only AllIn covers the raise");
    }
}
```

Update `crates/poker-core/src/holdem/mod.rs`:

```rust
pub mod pots;
pub mod state;
pub mod transitions;
pub mod types;

pub use state::{HandState, LogEntry, LogKind};
pub use transitions::{apply, legal_actions};
pub use types::{Action, ApplyError, HandConfig, Phase, PlayerId};
```

Note: `apply` is referenced in the re-exports but not yet defined. Add a stub in `transitions.rs` to keep it compiling:

```rust
pub fn apply(state: HandState, _action: Action) -> Result<HandState, (HandState, ApplyError)> {
    Err((state, ApplyError::IllegalAction("not yet implemented")))
}
```

- [ ] **Step 2: Run to verify**

Run: `cargo test -p poker-core holdem::transitions -- --nocapture`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/
git commit -m "feat(holdem): legal_actions for preflop opener"
```

---

### Task 4: `apply` — Fold and Check

**Files:**
- Modify: `crates/poker-core/src/holdem/transitions.rs`

- [ ] **Step 1: Write failing tests**

Add to `crates/poker-core/src/holdem/transitions.rs` (replace the stub `apply`):

```rust
/// Apply one action by the current actor. Returns the new state on success,
/// or `(state_unchanged, error)` on failure.
pub fn apply(
    mut state: HandState,
    action: Action,
) -> Result<HandState, (HandState, ApplyError)> {
    let Some(actor) = state.to_act else {
        return Err((state, ApplyError::HandComplete));
    };
    let i = actor.0;
    if !legal_actions(&state).contains(&action) {
        return Err((state, ApplyError::IllegalAction("not in legal_actions")));
    }

    match action {
        Action::Fold => {
            state.folded[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Fold),
            });
        }
        Action::Check => {
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Check),
            });
        }
        Action::Call | Action::Bet { .. } | Action::Raise { .. } | Action::AllIn => {
            // implemented in later tasks
            return Err((state, ApplyError::IllegalAction("call/bet/raise/all-in not yet wired")));
        }
    }

    advance_actor(&mut state);
    Ok(state)
}

/// Move `to_act` to the next player who still needs to act, or close the round.
fn advance_actor(state: &mut HandState) {
    let n = state.config.num_players;
    let Some(PlayerId(curr)) = state.to_act else { return; };
    for step in 1..=n {
        let cand = (curr + step) % n;
        if state.folded[cand] || state.all_in[cand] {
            continue;
        }
        // Player has acted and matched the bet → skip
        if has_acted_and_matched(state, cand) {
            continue;
        }
        state.to_act = Some(PlayerId(cand));
        return;
    }
    // No one left to act → round closes (handled in Task 6).
    state.to_act = None;
}

fn has_acted_and_matched(state: &HandState, idx: usize) -> bool {
    // A player is done for the round if their round_bet matches current_bet
    // AND they've been given a chance to act since the last raise.
    //
    // We track this loosely: a player matches the current bet means they're done,
    // UNLESS they're the last_aggressor in which case the round will close back to them.
    //
    // For Fold/Check only (this task), this simplified check is sufficient.
    state.round_bet[idx] == state.current_bet
        && state.last_aggressor != Some(PlayerId(idx))
}

#[cfg(test)]
mod apply_tests {
    use super::*;
    use crate::holdem::state::HandState;
    use crate::holdem::types::{HandConfig, PlayerId};

    fn cfg6() -> HandConfig {
        HandConfig {
            num_players: 6, small_blind: 50, big_blind: 100,
            dealer: PlayerId(0), seed: 1,
        }
    }

    #[test]
    fn fold_marks_player_folded_and_advances() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        let after = apply(s, Action::Fold).expect("fold legal");
        assert!(after.folded[3], "UTG (idx 3) folded");
        assert_eq!(after.to_act, Some(PlayerId(4)), "next active actor");
    }

    #[test]
    fn check_is_illegal_facing_bb() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        let result = apply(s, Action::Check);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run to verify**

Run: `cargo test -p poker-core holdem::transitions::apply_tests -- --nocapture`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/transitions.rs
git commit -m "feat(holdem): apply Fold and Check with actor advance"
```

---

### Task 5: `apply` — Call (including all-in call)

**Files:**
- Modify: `crates/poker-core/src/holdem/transitions.rs`

- [ ] **Step 1: Write failing tests**

In `transitions.rs`, replace the `Action::Call | ...` match arm:

```rust
        Action::Call => {
            let to_call = state.current_bet - state.round_bet[i];
            let pay = to_call.min(state.stacks[i]);
            state.stacks[i] -= pay;
            state.contributed[i] += pay;
            state.round_bet[i] += pay;
            if state.stacks[i] == 0 {
                state.all_in[i] = true;
            }
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Call),
            });
        }
```

(Keep `Bet`/`Raise`/`AllIn` returning the "not yet wired" error.)

Add tests:

```rust
    #[test]
    fn call_moves_chips_from_stack_to_round_bet() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        let after = apply(s, Action::Call).expect("call legal");
        assert_eq!(after.stacks[3], 9_900, "UTG paid 100");
        assert_eq!(after.round_bet[3], 100);
        assert_eq!(after.contributed[3], 100);
        assert!(!after.all_in[3]);
    }

    #[test]
    fn call_with_short_stack_goes_all_in() {
        let mut stacks = vec![10_000u64; 6];
        stacks[3] = 70; // UTG can only put in 70 of the 100 call
        let s = HandState::new_hand(cfg6(), stacks);
        let after = apply(s, Action::Call).expect("call legal");
        assert_eq!(after.stacks[3], 0);
        assert_eq!(after.round_bet[3], 70);
        assert!(after.all_in[3], "all-in on short call");
    }
```

- [ ] **Step 2: Run to verify**

Run: `cargo test -p poker-core holdem::transitions::apply_tests -- --nocapture`
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/transitions.rs
git commit -m "feat(holdem): apply Call with short-stack all-in"
```

---

### Task 6: `apply` — Bet, Raise, AllIn + min-raise rules

**Files:**
- Modify: `crates/poker-core/src/holdem/transitions.rs`

- [ ] **Step 1: Add failing tests first**

```rust
    #[test]
    fn raise_to_300_updates_current_bet_and_min_raise() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        let after = apply(s, Action::Raise { to: 300 }).expect("legal raise");
        assert_eq!(after.current_bet, 300);
        assert_eq!(after.min_raise, 200, "raise size was 300-100=200");
        assert_eq!(after.last_aggressor, Some(PlayerId(3)));
        assert_eq!(after.stacks[3], 9_700);
    }

    #[test]
    fn raise_below_min_is_illegal() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        // min raise-to is 200 (current 100 + min_raise 100); 150 is illegal
        let result = apply(s, Action::Raise { to: 150 });
        assert!(result.is_err());
    }

    #[test]
    fn all_in_for_less_than_min_raise_does_not_reopen_betting() {
        // UTG has 180. Shove for 180 (current_bet 100 → new 180).
        // 180-100 = 80 < min_raise (100), so betting does NOT reopen for the BB.
        let mut stacks = vec![10_000u64; 6];
        stacks[3] = 180;
        let s = HandState::new_hand(cfg6(), stacks);
        let after = apply(s, Action::AllIn).expect("legal shove");
        assert_eq!(after.current_bet, 180);
        assert_eq!(after.min_raise, 100, "min_raise unchanged on under-shove");
        assert!(after.all_in[3]);
    }

    #[test]
    fn all_in_for_a_full_raise_does_reopen_betting() {
        let mut stacks = vec![10_000u64; 6];
        stacks[3] = 300; // shove to 300 → raise of 200 ≥ min_raise 100 → reopens
        let s = HandState::new_hand(cfg6(), stacks);
        let after = apply(s, Action::AllIn).expect("legal shove");
        assert_eq!(after.current_bet, 300);
        assert_eq!(after.min_raise, 200);
        assert_eq!(after.last_aggressor, Some(PlayerId(3)));
    }
```

- [ ] **Step 2: Implement Bet/Raise/AllIn**

Replace the `Bet`/`Raise`/`AllIn` arms in `apply`:

```rust
        Action::Bet { to } => {
            let extra = to - state.round_bet[i];
            debug_assert!(extra <= state.stacks[i]);
            state.stacks[i] -= extra;
            state.round_bet[i] += extra;
            state.contributed[i] += extra;
            state.current_bet = to;
            state.min_raise = to;
            state.last_aggressor = Some(actor);
            if state.stacks[i] == 0 { state.all_in[i] = true; }
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Bet { to }),
            });
        }
        Action::Raise { to } => {
            let raise_size = to - state.current_bet;
            let extra = to - state.round_bet[i];
            state.stacks[i] -= extra;
            state.round_bet[i] += extra;
            state.contributed[i] += extra;
            state.current_bet = to;
            state.min_raise = raise_size;
            state.last_aggressor = Some(actor);
            if state.stacks[i] == 0 { state.all_in[i] = true; }
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Raise { to }),
            });
        }
        Action::AllIn => {
            let extra = state.stacks[i];
            let new_to = state.round_bet[i] + extra;
            let raise_size = new_to.saturating_sub(state.current_bet);
            state.stacks[i] = 0;
            state.round_bet[i] = new_to;
            state.contributed[i] += extra;
            state.all_in[i] = true;
            if new_to > state.current_bet {
                state.current_bet = new_to;
                // Only a full-sized raise reopens betting and updates min_raise.
                if raise_size >= state.min_raise {
                    state.min_raise = raise_size;
                    state.last_aggressor = Some(actor);
                }
                // Otherwise: under-shove. current_bet rises but last_aggressor
                // and min_raise are unchanged → players who already acted at the
                // old bet do NOT get to re-raise (only call the new amount).
            }
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::AllIn),
            });
        }
```

Also extend `legal_actions` to permit `Bet`/`Raise` at any legal `to` amount, not only the minimum. Change the test predicate from exact match to "any Bet/Raise". Cleanest: switch `legal_actions` to validate via a helper, and have `apply` re-validate the exact amount. Replace `legal_actions` with:

```rust
pub fn legal_actions(state: &HandState) -> Vec<Action> {
    let Some(actor) = state.to_act else { return Vec::new(); };
    let i = actor.0;
    if state.folded[i] || state.all_in[i] { return Vec::new(); }

    let mut out = Vec::new();
    out.push(Action::Fold);
    let to_call = state.current_bet.saturating_sub(state.round_bet[i]);
    if to_call == 0 { out.push(Action::Check); } else { out.push(Action::Call); }

    let stack = state.stacks[i];
    let max_to = state.round_bet[i] + stack;
    if state.current_bet == 0 {
        let min_to = state.config.big_blind;
        if stack >= min_to { out.push(Action::Bet { to: min_to }); }
    } else {
        let min_to = state.current_bet + state.min_raise;
        if max_to >= min_to { out.push(Action::Raise { to: min_to }); }
    }
    if stack > 0 { out.push(Action::AllIn); }
    out
}

/// Returns true if `action` is legal for the current actor.
/// Bet/Raise variants here take *any* `to` between the min and max,
/// not just the minimum the `legal_actions` summary returns.
fn is_legal(state: &HandState, action: Action) -> bool {
    let Some(actor) = state.to_act else { return false; };
    let i = actor.0;
    if state.folded[i] || state.all_in[i] { return false; }
    match action {
        Action::Fold => true,
        Action::Check => state.round_bet[i] == state.current_bet,
        Action::Call => state.current_bet > state.round_bet[i],
        Action::Bet { to } => {
            state.current_bet == 0
                && to >= state.config.big_blind
                && to <= state.round_bet[i] + state.stacks[i]
        }
        Action::Raise { to } => {
            state.current_bet > 0
                && to >= state.current_bet + state.min_raise
                && to <= state.round_bet[i] + state.stacks[i]
        }
        Action::AllIn => state.stacks[i] > 0,
    }
}
```

In `apply`, replace `if !legal_actions(&state).contains(&action)` with `if !is_legal(&state, action)`.

- [ ] **Step 3: Run all transition tests**

Run: `cargo test -p poker-core holdem::transitions -- --nocapture`
Expected: all transitions tests PASS (6 in `apply_tests`, 2 in the top-level `tests`).

- [ ] **Step 4: Commit**

```bash
git add crates/poker-core/src/holdem/transitions.rs
git commit -m "feat(holdem): apply Bet/Raise/AllIn with min-raise + under-shove rule"
```

---

### Task 7: Round closure + advance to next phase

**Files:**
- Modify: `crates/poker-core/src/holdem/transitions.rs`

The round ends when every non-folded, non-all-in player has matched `current_bet` AND has acted since the last aggressor. Once it ends, we either:

- deal the next street (flop/turn/river) and set `to_act` to the first non-folded, non-all-in player left of the dealer, **OR**
- jump straight to showdown if everyone left is all-in, **OR**
- end the hand if only one player remains.

- [ ] **Step 1: Write failing tests**

```rust
    #[test]
    fn preflop_closes_when_all_call_bb() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        // UTG, MP, CO, BTN call; SB call; BB check
        let s = apply(s, Action::Call).unwrap(); // UTG
        let s = apply(s, Action::Call).unwrap(); // MP
        let s = apply(s, Action::Call).unwrap(); // CO
        let s = apply(s, Action::Call).unwrap(); // BTN (idx 0)
        let s = apply(s, Action::Call).unwrap(); // SB
        let s = apply(s, Action::Check).unwrap(); // BB
        assert_eq!(s.phase, Phase::Flop);
        assert_eq!(s.board.len(), 3, "flop dealt");
        assert_eq!(s.current_bet, 0);
        assert_eq!(s.round_bet, vec![0; 6]);
        assert_eq!(s.to_act, Some(PlayerId(1)), "SB acts first postflop");
    }

    #[test]
    fn folding_around_ends_hand_immediately() {
        let s = HandState::new_hand(cfg6(), vec![10_000; 6]);
        // Everyone folds to BB
        let s = apply(s, Action::Fold).unwrap(); // UTG
        let s = apply(s, Action::Fold).unwrap(); // MP
        let s = apply(s, Action::Fold).unwrap(); // CO
        let s = apply(s, Action::Fold).unwrap(); // BTN
        let s = apply(s, Action::Fold).unwrap(); // SB
        assert_eq!(s.phase, Phase::Complete);
        // BB wins SB+BB = 150 plus their own BB back = stack 10_050
        assert_eq!(s.stacks[2], 10_050);
    }

    #[test]
    fn all_remaining_all_in_jumps_to_showdown() {
        // Two players, both shove preflop.
        let cfg = HandConfig {
            num_players: 2, small_blind: 50, big_blind: 100,
            dealer: PlayerId(0), seed: 7,
        };
        let s = HandState::new_hand(cfg, vec![1_000, 1_000]);
        let s = apply(s, Action::AllIn).unwrap();   // SB shoves
        let s = apply(s, Action::Call).unwrap();    // BB calls (also all-in)
        assert_eq!(s.phase, Phase::Complete);
        assert_eq!(s.board.len(), 5, "all 5 community cards dealt before showdown");
    }
```

- [ ] **Step 2: Implement round closure + phase advance**

In `transitions.rs`, replace `advance_actor` and add helpers:

```rust
fn advance_actor(state: &mut HandState) {
    if try_terminate_by_folds(state) { return; }
    if round_is_closed(state) {
        close_round_and_advance(state);
        return;
    }
    let n = state.config.num_players;
    let Some(PlayerId(curr)) = state.to_act else { return; };
    for step in 1..=n {
        let cand = (curr + step) % n;
        if state.folded[cand] || state.all_in[cand] { continue; }
        state.to_act = Some(PlayerId(cand));
        return;
    }
    // Nobody can act — close the round.
    close_round_and_advance(state);
}

fn try_terminate_by_folds(state: &mut HandState) -> bool {
    let active: Vec<usize> = (0..state.config.num_players)
        .filter(|&i| !state.folded[i])
        .collect();
    if active.len() == 1 {
        // Award the entire pot to the lone survivor.
        let winner = active[0];
        let total: u64 = state.contributed.iter().sum();
        state.stacks[winner] += total;
        state.phase = Phase::Complete;
        state.to_act = None;
        return true;
    }
    false
}

fn round_is_closed(state: &HandState) -> bool {
    // The round is closed when, scanning from `to_act` around the table,
    // we reach someone who is not eligible to act:
    //   * non-aggressor whose round_bet matches current_bet → already done
    //   * the last_aggressor themselves → action has returned to them
    //   * or there is nobody who still owes chips and hasn't acted.
    //
    // Concretely: among non-folded, non-all-in players, the round is closed
    // when *every* such player has round_bet == current_bet AND we are not
    // simply at the start of a new round before anyone has acted.
    let n = state.config.num_players;
    let actionable: Vec<usize> = (0..n)
        .filter(|&i| !state.folded[i] && !state.all_in[i])
        .collect();

    if actionable.is_empty() { return true; }

    // All matched the bet?
    let all_matched = actionable.iter().all(|&i| state.round_bet[i] == state.current_bet);
    if !all_matched { return false; }

    // The BB gets a special "option" preflop: even if everyone limped,
    // the BB is allowed to raise. We detect this by checking if the
    // last_aggressor is the BB *and* the BB hasn't been given a chance to act yet.
    //
    // Simpler model: store `acted_this_round: Vec<bool>` and clear it on round
    // start; a round is closed when all actionable players have `acted_this_round`
    // *and* their round_bet matches current_bet.
    state.actor_has_acted_this_round(actionable.as_slice())
}
```

To make `actor_has_acted_this_round` work, add a `acted_this_round: Vec<bool>` field to `HandState` and reset it on every phase change. Add to `state.rs`:

```rust
// In HandState struct:
pub acted_this_round: Vec<bool>,

// In new_hand, after building fields, BEFORE returning HandState:
let mut acted_this_round = vec![false; n];
// BB has not "acted" voluntarily — they posted; they still have the option.
// SB also has not acted voluntarily.

// In the returned HandState literal, include:
acted_this_round,
```

And in `state.rs`, add an `impl HandState`:

```rust
impl HandState {
    pub(crate) fn actor_has_acted_this_round(&self, actionable: &[usize]) -> bool {
        actionable.iter().all(|&i| self.acted_this_round[i])
    }
}
```

In `apply`, set `state.acted_this_round[i] = true` at the end of every Fold/Check/Call/Bet/Raise/AllIn arm (before `advance_actor`).

Now implement the round-close helper:

```rust
fn close_round_and_advance(state: &mut HandState) {
    // Sweep round_bet into contributed already happened on each bet; nothing to do.
    state.round_bet.iter_mut().for_each(|b| *b = 0);
    state.current_bet = 0;
    state.min_raise = state.config.big_blind;
    state.last_aggressor = None;
    state.acted_this_round.iter_mut().for_each(|b| *b = false);

    // If only one player can still act AND every other live player is all-in,
    // there's no betting left — run out the board to showdown.
    let active: Vec<usize> = (0..state.config.num_players)
        .filter(|&i| !state.folded[i])
        .collect();
    let actionable_count = active.iter().filter(|&&i| !state.all_in[i]).count();
    let runout = actionable_count <= 1;

    let next = match state.phase {
        Phase::Preflop => Phase::Flop,
        Phase::Flop => Phase::Turn,
        Phase::Turn => Phase::River,
        Phase::River => Phase::Showdown,
        Phase::Showdown | Phase::Complete => state.phase,
    };
    state.phase = next;

    match next {
        Phase::Flop => {
            let c = [state.deck.remove(0), state.deck.remove(0), state.deck.remove(0)];
            state.board.extend_from_slice(&c);
            state.log.push(super::state::LogEntry {
                actor: None,
                kind: super::state::LogKind::DealFlop(c),
            });
        }
        Phase::Turn => {
            let c = state.deck.remove(0);
            state.board.push(c);
            state.log.push(super::state::LogEntry {
                actor: None,
                kind: super::state::LogKind::DealTurn(c),
            });
        }
        Phase::River => {
            let c = state.deck.remove(0);
            state.board.push(c);
            state.log.push(super::state::LogEntry {
                actor: None,
                kind: super::state::LogKind::DealRiver(c),
            });
        }
        Phase::Showdown => {
            // Implemented in Task 9.
            state.to_act = None;
            return;
        }
        _ => {}
    }

    if runout && !matches!(next, Phase::Showdown) {
        // Recurse to deal next street with no betting.
        close_round_and_advance(state);
        return;
    }

    // First to act postflop: first non-folded, non-all-in left of dealer.
    let n = state.config.num_players;
    let dealer = state.config.dealer.0;
    for step in 1..=n {
        let cand = (dealer + step) % n;
        if !state.folded[cand] && !state.all_in[cand] {
            state.to_act = Some(PlayerId(cand));
            return;
        }
    }
    state.to_act = None;
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p poker-core holdem -- --nocapture`
Expected: all earlier tests + the 3 new ones PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/poker-core/src/holdem/
git commit -m "feat(holdem): round closure, phase advance, runout when no actors remain"
```

---

### Task 8: Side pot construction

**Files:**
- Modify: `crates/poker-core/src/holdem/pots.rs`

Side pots are layers built from each player's `contributed` total. Algorithm:

1. Collect each non-folded player's contribution into a sorted list of distinct levels.
2. Walk the levels from lowest to highest. At each level `L`, the pot's `amount` is `(L − previous_L) * (count of all players, folded or not, whose contributed ≥ L)`. Folded players' chips count toward the pot but they're not eligible to win.
3. Eligible winners for the pot at level `L` are non-folded players whose contributed ≥ `L`.

- [ ] **Step 1: Write failing tests**

In `pots.rs`:

```rust
use super::types::PlayerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pot {
    pub amount: u64,
    pub eligible: Vec<PlayerId>,
}

/// Build the layered pot structure from per-player contributions.
///
/// `contributed[i]` = total chips player i has put in across the hand.
/// `folded[i]` = whether player i has folded.
///
/// Folded players' chips go into the pot but they cannot win it.
pub fn build_pots(contributed: &[u64], folded: &[bool]) -> Vec<Pot> {
    let n = contributed.len();
    assert_eq!(folded.len(), n);

    // Levels = distinct positive contribution amounts from non-folded players,
    // sorted ascending. (Folded players don't *create* levels — they only have
    // their chips swept into whatever level encloses them.)
    let mut levels: Vec<u64> = (0..n)
        .filter(|&i| !folded[i] && contributed[i] > 0)
        .map(|i| contributed[i])
        .collect();
    levels.sort();
    levels.dedup();

    let mut pots: Vec<Pot> = Vec::new();
    let mut prev = 0u64;
    for &lvl in &levels {
        let band = lvl - prev;
        let amount: u64 = (0..n)
            .map(|i| contributed[i].min(lvl).saturating_sub(prev))
            .sum::<u64>();
        let _ = band; // explicit clarity; `amount` already accounts for partial contributors
        if amount == 0 { prev = lvl; continue; }
        let mut eligible: Vec<PlayerId> = (0..n)
            .filter(|&i| !folded[i] && contributed[i] >= lvl)
            .map(PlayerId)
            .collect();
        eligible.sort();
        pots.push(Pot { amount, eligible });
        prev = lvl;
    }

    pots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_one_contributed_yields_no_pots() {
        let pots = build_pots(&[0, 0, 0], &[false, false, false]);
        assert!(pots.is_empty());
    }

    #[test]
    fn all_equal_contributions_one_pot() {
        let pots = build_pots(&[100, 100, 100], &[false, false, false]);
        assert_eq!(pots.len(), 1);
        assert_eq!(pots[0].amount, 300);
        assert_eq!(pots[0].eligible, vec![PlayerId(0), PlayerId(1), PlayerId(2)]);
    }

    #[test]
    fn one_short_stack_creates_main_and_side_pot() {
        // P0 all-in for 50, P1 and P2 each in for 200.
        let pots = build_pots(&[50, 200, 200], &[false, false, false]);
        assert_eq!(pots.len(), 2);

        // Main pot: 50 * 3 = 150, all three eligible
        assert_eq!(pots[0].amount, 150);
        assert_eq!(pots[0].eligible, vec![PlayerId(0), PlayerId(1), PlayerId(2)]);

        // Side pot: (200-50)*2 = 300, only P1 and P2 eligible
        assert_eq!(pots[1].amount, 300);
        assert_eq!(pots[1].eligible, vec![PlayerId(1), PlayerId(2)]);
    }

    #[test]
    fn folded_player_chips_go_into_pot_but_they_cannot_win() {
        // P0 in 100, P1 in 100 then folded, P2 in 200
        let pots = build_pots(&[100, 100, 200], &[false, true, false]);
        // Levels (non-folded only): 100 (P0), 200 (P2)
        assert_eq!(pots.len(), 2);
        // Main pot at level 100: all three contributed ≥ 100 → 100+100+100 = 300
        // P1 is in the pot but not eligible.
        assert_eq!(pots[0].amount, 300);
        assert_eq!(pots[0].eligible, vec![PlayerId(0), PlayerId(2)]);
        // Side pot at level 200: 0 (P0 didn't reach) + 0 (P1 didn't either) + 100 (P2)
        assert_eq!(pots[1].amount, 100);
        assert_eq!(pots[1].eligible, vec![PlayerId(2)]);
    }

    #[test]
    fn three_way_all_in_different_stacks() {
        // P0 all-in 100, P1 all-in 250, P2 all-in 600
        let pots = build_pots(&[100, 250, 600], &[false, false, false]);
        assert_eq!(pots.len(), 3);
        // Level 100: 100*3 = 300, eligible {0,1,2}
        assert_eq!(pots[0], Pot { amount: 300, eligible: vec![PlayerId(0), PlayerId(1), PlayerId(2)] });
        // Level 250: (250-100)*2 = 300 (only P1 and P2 contribute beyond 100)
        assert_eq!(pots[1], Pot { amount: 300, eligible: vec![PlayerId(1), PlayerId(2)] });
        // Level 600: (600-250)*1 = 350 (only P2)
        assert_eq!(pots[2], Pot { amount: 350, eligible: vec![PlayerId(2)] });
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p poker-core holdem::pots -- --nocapture`
Expected: 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/src/holdem/pots.rs
git commit -m "feat(holdem): side pot construction with folded chip accounting"
```

---

### Task 9: Showdown — settle pots and award chips

**Files:**
- Modify: `crates/poker-core/src/holdem/pots.rs`
- Modify: `crates/poker-core/src/holdem/transitions.rs`

Settlement: for each pot, evaluate each eligible player's best 7-card hand using the existing `evaluate(&[Card])` function. Ties split the pot evenly; **leftover chips go to the eligible player closest to the left of the dealer** (standard rule — keeps determinism and matches casino practice).

- [ ] **Step 1: Write failing tests**

Add to `pots.rs`:

```rust
use crate::evaluate;
use crate::Card;
use super::state::HandState;

/// Resolve every pot, mutate stacks, return per-pot winners and amounts.
/// Pre: state.phase == Phase::Showdown or state.phase == Phase::Complete-by-runout.
pub fn settle(state: &mut HandState) -> Vec<(Pot, Vec<PlayerId>, u64)> {
    let pots = build_pots(&state.contributed, &state.folded);
    let mut results = Vec::with_capacity(pots.len());
    for (pot_idx, pot) in pots.iter().enumerate() {
        let winners = best_among(&pot.eligible, state);
        let share = pot.amount / winners.len() as u64;
        let mut leftover = pot.amount - share * winners.len() as u64;

        // Distribute base share.
        for &PlayerId(i) in &winners {
            state.stacks[i] += share;
            state.log.push(super::state::LogEntry {
                actor: Some(PlayerId(i)),
                kind: super::state::LogKind::WinPot { pot_idx, amount: share },
            });
        }
        // Distribute leftover chips one at a time, starting left of dealer.
        let n = state.config.num_players;
        let mut step = 1usize;
        while leftover > 0 {
            let cand = (state.config.dealer.0 + step) % n;
            if winners.contains(&PlayerId(cand)) {
                state.stacks[cand] += 1;
                leftover -= 1;
            }
            step += 1;
            if step > 2 * n { break; } // safety
        }
        results.push((pot.clone(), winners, pot.amount));
    }
    state.pots = pots;
    state.winners = results.iter().map(|(_, w, _)| w.clone()).collect();
    state.phase = super::types::Phase::Complete;
    state.to_act = None;
    results
}

fn best_among(eligible: &[PlayerId], state: &HandState) -> Vec<PlayerId> {
    let mut best_strength: Option<crate::HandStrength> = None;
    let mut winners: Vec<PlayerId> = Vec::new();
    for &PlayerId(i) in eligible {
        let mut seven: Vec<Card> = state.board.clone();
        seven.extend_from_slice(&state.hole[i]);
        let s = evaluate(&seven);
        match best_strength {
            None => { best_strength = Some(s); winners.push(PlayerId(i)); }
            Some(curr) if s > curr => { best_strength = Some(s); winners.clear(); winners.push(PlayerId(i)); }
            Some(curr) if s == curr => { winners.push(PlayerId(i)); }
            _ => {}
        }
    }
    winners
}

#[cfg(test)]
mod settle_tests {
    use super::*;
    use crate::holdem::state::HandState;
    use crate::holdem::types::{Action, HandConfig, PlayerId, Phase};
    use crate::holdem::transitions::apply;

    #[test]
    fn heads_up_all_in_winner_takes_all() {
        let cfg = HandConfig {
            num_players: 2, small_blind: 50, big_blind: 100,
            dealer: PlayerId(0), seed: 42,
        };
        let mut s = HandState::new_hand(cfg, vec![1_000, 1_000]);
        s = apply(s, Action::AllIn).unwrap();
        s = apply(s, Action::Call).unwrap();
        // settle is called inside the runout path in Task 10. For now, call directly:
        let results = super::settle(&mut s);
        assert_eq!(results.len(), 1);
        let total_stacks: u64 = s.stacks.iter().sum();
        assert_eq!(total_stacks, 2_000, "chips conserved");
    }
}
```

- [ ] **Step 2: Wire settlement into round-closure**

In `transitions.rs::close_round_and_advance`, replace the `Phase::Showdown =>` arm with:

```rust
        Phase::Showdown => {
            state.log.push(super::state::LogEntry {
                actor: None,
                kind: super::state::LogKind::Showdown,
            });
            super::pots::settle(state);
            return;
        }
```

- [ ] **Step 3: Run**

Run: `cargo test -p poker-core holdem -- --nocapture`
Expected: all earlier + the new settle test PASS. Conservation check holds.

- [ ] **Step 4: Commit**

```bash
git add crates/poker-core/src/holdem/
git commit -m "feat(holdem): showdown settlement with hand ranking and odd-chip rule"
```

---

### Task 10: Integration test — full scripted hands

**Files:**
- Create: `crates/poker-core/tests/holdem_walkthrough.rs`

- [ ] **Step 1: Write the test**

```rust
use poker_core::holdem::{apply, Action, HandConfig, HandState, Phase, PlayerId};

fn cfg6(seed: u64) -> HandConfig {
    HandConfig {
        num_players: 6, small_blind: 50, big_blind: 100,
        dealer: PlayerId(0), seed,
    }
}

#[test]
fn full_hand_six_handed_limp_check_down() {
    let mut s = HandState::new_hand(cfg6(1), vec![10_000; 6]);
    // Preflop: 3, 4, 5, 0, 1 all call; 2 checks.
    for _ in 0..5 { s = apply(s, Action::Call).unwrap(); }
    s = apply(s, Action::Check).unwrap();
    assert_eq!(s.phase, Phase::Flop);

    // Flop: everyone checks.
    for _ in 0..6 { s = apply(s, Action::Check).unwrap(); }
    assert_eq!(s.phase, Phase::Turn);
    // Turn: everyone checks.
    for _ in 0..6 { s = apply(s, Action::Check).unwrap(); }
    assert_eq!(s.phase, Phase::River);
    // River: everyone checks.
    for _ in 0..6 { s = apply(s, Action::Check).unwrap(); }
    assert_eq!(s.phase, Phase::Complete);

    let total: u64 = s.stacks.iter().sum();
    assert_eq!(total, 60_000, "chips conserved over the whole hand");
}

#[test]
fn raise_and_fold_around() {
    let mut s = HandState::new_hand(cfg6(2), vec![10_000; 6]);
    // UTG raises to 300, others fold.
    s = apply(s, Action::Raise { to: 300 }).unwrap();
    for _ in 0..5 { s = apply(s, Action::Fold).unwrap(); }
    assert_eq!(s.phase, Phase::Complete);
    // UTG wins pot of SB(50)+BB(100) = 150; UTG put in 300 then got it all back.
    assert_eq!(s.stacks[3], 10_000 + 150);
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p poker-core --test holdem_walkthrough -- --nocapture`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/tests/holdem_walkthrough.rs
git commit -m "test(holdem): scripted full hands — limp-checkdown and raise-fold-around"
```

---

### Task 11: Integration test — multi-way all-in side pots

**Files:**
- Create: `crates/poker-core/tests/holdem_side_pots.rs`

This is the **single most important correctness test in the engine**. Cover three scenarios:

1. Two short stacks all-in for different amounts against a big stack — produces a main pot and one side pot.
2. Three different all-in stack sizes — produces main + two side pots.
3. A short all-in plus a folded mid-stack — folded chips swept into main, mid-stack not eligible.

- [ ] **Step 1: Write tests**

```rust
use poker_core::holdem::{apply, Action, HandConfig, HandState, Phase, PlayerId};

fn cfg(n: usize, seed: u64) -> HandConfig {
    HandConfig {
        num_players: n, small_blind: 50, big_blind: 100,
        dealer: PlayerId(0), seed,
    }
}

#[test]
fn three_way_all_in_three_pots() {
    // P0 short (200), P1 mid (500), P2 big (10_000)
    let mut s = HandState::new_hand(cfg(3, 99), vec![200, 500, 10_000]);
    // HU-style 3-player preflop order: dealer(0)+1=SB(1), +2=BB(2), +3=UTG=(0) is dealer too
    // In 3-handed: dealer=0 (BTN), 1=SB, 2=BB, action starts on BTN (idx 0).
    s = apply(s, Action::AllIn).unwrap(); // BTN shoves 200
    s = apply(s, Action::AllIn).unwrap(); // SB shoves 500
    s = apply(s, Action::AllIn).unwrap(); // BB shoves 10_000
    assert_eq!(s.phase, Phase::Complete);

    let total: u64 = s.stacks.iter().sum();
    assert_eq!(total, 200 + 500 + 10_000, "chips conserved across multi-way all-in");

    // Three pots, smallest first.
    assert_eq!(s.pots.len(), 3);
    assert_eq!(s.pots[0].amount, 600,   "main pot = 200 * 3");
    assert_eq!(s.pots[1].amount, 600,   "side pot 1 = (500-200)*2");
    assert_eq!(s.pots[2].amount, 9_500, "side pot 2 = (10_000-500)*1");
}

#[test]
fn folded_player_chips_count_toward_main_pot() {
    // 4-handed: BTN(0)=200 (short), SB(1)=400 (mid, will fold), BB(2)=10_000, UTG(3)=10_000
    let mut s = HandState::new_hand(cfg(4, 17), vec![200, 400, 10_000, 10_000]);
    // Preflop action: UTG raises to 300, BTN shoves 200 (under-raise),
    // SB folds (loses 50 SB), BB calls 300, UTG... already raised.
    s = apply(s, Action::Raise { to: 300 }).unwrap(); // UTG raises to 300
    s = apply(s, Action::AllIn).unwrap();             // BTN shoves 200
    s = apply(s, Action::Fold).unwrap();              // SB folds, loses 50
    s = apply(s, Action::Call).unwrap();              // BB calls 300

    // UTG already put 300; current_bet = 300; UTG is last_aggressor.
    // Action is back to UTG, who already matched → round closes.
    // Run out the board (only BB + UTG can act postflop; BTN is all-in).
    // Postflop is checked down because they have no incentive (no model: we just check).
    loop {
        if matches!(s.phase, Phase::Complete) { break; }
        // Always check on every postflop street.
        s = apply(s, Action::Check).unwrap();
    }

    let total: u64 = s.stacks.iter().sum();
    assert_eq!(total, 200 + 400 + 10_000 + 10_000, "chips conserved");

    // Main pot includes BTN's 200 from each contributor + SB's folded 50:
    //   3 (UTG, BB, BTN) put in 200 at the 200 level = 600
    //   SB's 50 also got swept: contributed[1]=50, min(50,200)-0 = 50 → main = 650
    assert_eq!(s.pots[0].amount, 650, "main pot includes folded SB's 50");
    // Side pot at level 300: UTG and BB contributed 300; BTN capped at 200.
    //   (300-200)*2 = 200
    assert_eq!(s.pots[1].amount, 200);
    // BTN cannot be eligible for the side pot.
    assert!(!s.pots[1].eligible.contains(&PlayerId(0)));
}

#[test]
fn two_way_all_in_chips_conserved() {
    let mut s = HandState::new_hand(cfg(2, 7), vec![1_000, 1_500]);
    s = apply(s, Action::AllIn).unwrap();
    s = apply(s, Action::Call).unwrap();
    assert_eq!(s.phase, Phase::Complete);
    let total: u64 = s.stacks.iter().sum();
    assert_eq!(total, 2_500);
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p poker-core --test holdem_side_pots -- --nocapture`
Expected: 3 tests PASS. If a side-pot test fails, dump `s.pots` and `s.contributed` to diagnose.

- [ ] **Step 3: Commit**

```bash
git add crates/poker-core/tests/holdem_side_pots.rs
git commit -m "test(holdem): multi-way all-in side pots and folded-chip accounting"
```

---

### Task 12: Convert UI presentation types to owned strings

**Files:**
- Modify: `crates/pokertui/src/state.rs`
- Modify: `crates/pokertui/src/ui.rs`

The current `Seat`, `LogEntry`, `ChatLine`, and `Phase` types use `&'static str` because they're only constructed from string literals in `demo()`. To take values from the engine, we need owned strings.

- [ ] **Step 1: Change types**

In `crates/pokertui/src/state.rs`:

- `Seat.name: &'static str` → `String`
- `Seat.pos: &'static str` → `String`
- `Seat.last_action: &'static str` → `String`
- `LogEntry.who: &'static str` → `String`
- `LogEntry.what: &'static str` → `String`
- `ChatLine.who: &'static str` → `String`
- `ChatLine.msg: &'static str` → `String`
- `Phase.label: &'static str` → `String`
- `Phase.rank: &'static str` → `String`
- `Phase.hint: &'static str` → `String`

Update `demo()` to wrap all literals with `.into()` or `String::from`.

- [ ] **Step 2: Fix UI breakage**

In `crates/pokertui/src/ui.rs`, every place that took `&'static str` from these structs (e.g. `p.name`, `state.phase.label`, `e.who`, `c.msg`) now provides `&str` via implicit deref or `.as_str()`. Mostly mechanical:

- `Span::raw(p.name)` → `Span::raw(p.name.clone())` **or** restructure to take `&str` (preferred: change span constructors to use `&str` via `.clone()`-free paths where possible). Easiest: append `.clone()` to fields used in `Span::styled(..., ...)` constructors that require `'static` or owned `String`.
- `state.blinds: &'static str` stays — only the dynamic fields need to change.

Run `cargo check -p pokertui` repeatedly until clean.

- [ ] **Step 3: Run**

```
cargo test -p pokertui
```
Expected: existing UI tests still PASS (`workbench_rail_roomy_renders`, `too_small_terminal_shows_notice`).

- [ ] **Step 4: Commit**

```bash
git add crates/pokertui/src/state.rs crates/pokertui/src/ui.rs
git commit -m "refactor(ui): owned strings on Seat/LogEntry/ChatLine/Phase for engine-driven content"
```

---

### Task 13: `adapter.rs` — translate `HandState` → presentation `GameState`

**Files:**
- Create: `crates/pokertui/src/adapter.rs`
- Modify: `crates/pokertui/src/main.rs` (just `mod adapter;`)

- [ ] **Step 1: Write tests**

Create `crates/pokertui/src/adapter.rs`:

```rust
use poker_core::holdem::{HandState, Phase as EnginePhase, PlayerId};

use crate::state::{ChatLine, GameState, LogEntry, LogTone, Phase, Seat, SeatStatus};

pub struct NameRegistry {
    pub names: Vec<String>,        // indexed by PlayerId
    pub hero: PlayerId,
}

impl NameRegistry {
    pub fn demo_six() -> Self {
        Self {
            names: vec![
                "nova".into(), "delta".into(), "gizmo".into(),
                "you".into(),  "maple".into(), "rook".into(),
            ],
            hero: PlayerId(3),
        }
    }
}

/// Position labels for a standard 6-handed table, indexed by offset from the dealer.
fn position_label(offset: usize, n: usize) -> String {
    if n == 2 {
        return if offset == 0 { "BTN".into() } else { "BB".into() };
    }
    match (offset, n) {
        (0, _) => "BTN".into(),
        (1, _) => "SB".into(),
        (2, _) => "BB".into(),
        (3, _) => "UTG".into(),
        (4, n) if n >= 6 => "MP".into(),
        (5, n) if n >= 6 => "CO".into(),
        // Catch-all for short tables
        _ => format!("P{}", offset),
    }
}

pub fn to_presentation(engine: &HandState, names: &NameRegistry) -> GameState {
    let n = engine.config.num_players;
    let mut players = Vec::with_capacity(n);
    for i in 0..n {
        let offset = (i + n - engine.config.dealer.0) % n;
        let pos = position_label(offset, n);
        let status = if PlayerId(i) == names.hero {
            SeatStatus::Hero
        } else if engine.folded[i] {
            SeatStatus::Folded
        } else if Some(PlayerId(i)) == engine.last_aggressor {
            SeatStatus::Bet
        } else {
            SeatStatus::Active
        };
        let last_action = engine.log.iter().rev().find_map(|e| {
            if e.actor == Some(PlayerId(i)) {
                Some(format_log_kind(&e.kind))
            } else {
                None
            }
        }).unwrap_or_else(|| "—".into());
        let hole_cards = if PlayerId(i) == names.hero || engine.phase == EnginePhase::Complete {
            Some(engine.hole[i])
        } else {
            None
        };
        players.push(Seat {
            name: names.names[i].clone(),
            pos,
            stack: engine.stacks[i],
            status,
            last_action,
            hole_cards,
        });
    }

    let phase = Phase {
        label: phase_label(engine.phase),
        board: engine.board.clone(),
        dealt: engine.board.len(),
        pot: engine.contributed.iter().sum(),
        to_call: to_call_for_hero(engine, names.hero),
        equity: 0,
        odds_pct: 0.0,
        rank: "—".into(),
        hint: "—".into(),
    };

    let log: Vec<LogEntry> = engine.log.iter().map(|e| LogEntry {
        who: e.actor.map(|p| names.names[p.0].clone()).unwrap_or_else(|| "·".into()),
        what: format_log_kind(&e.kind),
        tone: tone_for(&e.kind),
    }).collect();

    GameState {
        blinds: format!("{} / {}", engine.config.small_blind, engine.config.big_blind),
        players,
        phase,
        log,
        chat: Vec::<ChatLine>::new(),
    }
}

fn phase_label(p: EnginePhase) -> String {
    match p {
        EnginePhase::Preflop => "PREFLOP".into(),
        EnginePhase::Flop => "FLOP".into(),
        EnginePhase::Turn => "TURN".into(),
        EnginePhase::River => "RIVER".into(),
        EnginePhase::Showdown => "SHOWDOWN".into(),
        EnginePhase::Complete => "COMPLETE".into(),
    }
}

fn to_call_for_hero(engine: &HandState, hero: PlayerId) -> u64 {
    engine.current_bet.saturating_sub(engine.round_bet[hero.0])
}

fn format_log_kind(kind: &poker_core::holdem::LogKind) -> String {
    use poker_core::holdem::LogKind::*;
    use poker_core::holdem::Action::*;
    match kind {
        PostBlind { amount, is_big } => {
            format!("posts {} {}", if *is_big { "BB" } else { "SB" }, amount)
        }
        Action(Fold) => "fold".into(),
        Action(Check) => "check".into(),
        Action(Call) => "call".into(),
        Action(Bet { to }) => format!("bet {}", to),
        Action(Raise { to }) => format!("raise {}", to),
        Action(AllIn) => "all-in".into(),
        DealFlop(_) => "flop".into(),
        DealTurn(_) => "turn".into(),
        DealRiver(_) => "river".into(),
        Showdown => "showdown".into(),
        WinPot { amount, .. } => format!("wins {}", amount),
    }
}

fn tone_for(kind: &poker_core::holdem::LogKind) -> LogTone {
    use poker_core::holdem::LogKind::*;
    use poker_core::holdem::Action::*;
    match kind {
        Action(Bet { .. } | Raise { .. } | AllIn) => LogTone::Amber,
        DealFlop(_) | DealTurn(_) | DealRiver(_) | Showdown => LogTone::Dim,
        PostBlind { .. } => LogTone::Dim,
        WinPot { .. } => LogTone::Amber,
        _ => LogTone::Fg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poker_core::holdem::{HandConfig, HandState};

    #[test]
    fn fresh_hand_presents_six_seats_with_correct_positions() {
        let cfg = HandConfig {
            num_players: 6, small_blind: 50, big_blind: 100,
            dealer: PlayerId(0), seed: 1,
        };
        let engine = HandState::new_hand(cfg, vec![10_000; 6]);
        let names = NameRegistry::demo_six();
        let view = to_presentation(&engine, &names);
        assert_eq!(view.players.len(), 6);
        assert_eq!(view.players[0].pos, "BTN");
        assert_eq!(view.players[1].pos, "SB");
        assert_eq!(view.players[2].pos, "BB");
        assert_eq!(view.players[3].pos, "UTG");
        assert_eq!(view.phase.label, "PREFLOP");
        assert_eq!(view.phase.pot, 150, "SB + BB in pot");
    }
}
```

In `crates/pokertui/src/main.rs`, add `mod adapter;` near `mod state; mod ui;`.

- [ ] **Step 2: Run**

Run: `cargo test -p pokertui adapter -- --nocapture`
Expected: 1 test PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/pokertui/src/adapter.rs crates/pokertui/src/main.rs
git commit -m "feat(ui): adapter — HandState + NameRegistry → presentation GameState"
```

---

### Task 14: `App` — wire keys to engine actions

**Files:**
- Create: `crates/pokertui/src/app.rs`
- Modify: `crates/pokertui/src/main.rs`

`App` owns the engine state and the name registry, exposes a derived `GameState` for rendering, and handles key dispatch. Raise/bet sizing in this minimal first cut: pressing R/B raises to the **minimum legal raise**. A `+`/`-` adjustment for raise size is a follow-up.

- [ ] **Step 1: Write tests**

Create `crates/pokertui/src/app.rs`:

```rust
use crossterm::event::KeyCode;
use poker_core::holdem::{
    apply, legal_actions, Action, HandConfig, HandState, PlayerId,
};

use crate::adapter::{to_presentation, NameRegistry};
use crate::state::GameState;

pub struct App {
    pub engine: HandState,
    pub names: NameRegistry,
}

impl App {
    pub fn new_demo_hand() -> Self {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 0xC0FFEE,
        };
        let engine = HandState::new_hand(cfg, vec![10_000; 6]);
        let names = NameRegistry::demo_six();
        Self { engine, names }
    }

    pub fn view(&self) -> GameState {
        to_presentation(&self.engine, &self.names)
    }

    /// Returns true if the key was consumed (engine state may have changed).
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        let Some(actor) = self.engine.to_act else { return false; };
        // Only the hero (the human at the keyboard for pass-and-play) acts via keys.
        // In hot-seat, every active player IS the hero from their own perspective —
        // we accept input for whoever is to_act. This makes pass-and-play work.
        let _ = actor;

        let action = match key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(Action::Fold),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if legal_actions(&self.engine).contains(&Action::Check) {
                    Some(Action::Check)
                } else {
                    Some(Action::Call)
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // Minimum legal raise / bet.
                if self.engine.current_bet == 0 {
                    Some(Action::Bet { to: self.engine.config.big_blind })
                } else {
                    Some(Action::Raise {
                        to: self.engine.current_bet + self.engine.min_raise,
                    })
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::AllIn),
            _ => None,
        };

        let Some(action) = action else { return false; };
        // Replace engine state in place, ignoring illegal moves.
        let taken = std::mem::replace(&mut self.engine, dummy_engine());
        match apply(taken, action) {
            Ok(next) => { self.engine = next; true }
            Err((restored, _)) => { self.engine = restored; false }
        }
    }
}

fn dummy_engine() -> HandState {
    // Placeholder used only between `replace` and `match` above. Never observed
    // because the match arms re-assign immediately.
    HandState::new_hand(
        HandConfig { num_players: 2, small_blind: 1, big_blind: 2, dealer: PlayerId(0), seed: 0 },
        vec![10, 10],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_f_folds_current_actor() {
        let mut app = App::new_demo_hand();
        let actor_before = app.engine.to_act.unwrap();
        app.handle_key(KeyCode::Char('f'));
        assert!(app.engine.folded[actor_before.0]);
    }

    #[test]
    fn pressing_c_calls_when_facing_bet() {
        let mut app = App::new_demo_hand();
        let actor = app.engine.to_act.unwrap();
        app.handle_key(KeyCode::Char('c'));
        assert_eq!(app.engine.round_bet[actor.0], 100);
    }

    #[test]
    fn pressing_r_raises_to_min() {
        let mut app = App::new_demo_hand();
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.engine.current_bet, 200);
    }
}
```

- [ ] **Step 2: Wire `App` into `main.rs`**

In `crates/pokertui/src/main.rs`:

- Add `mod app;`
- Import `use crate::app::App;`
- In `run`, replace `let state = GameState::demo();` with `let mut app = App::new_demo_hand();`.
- In the draw closure, render `&app.view()` instead of `&state`.
- In the key handler, before the `q`/`Esc` exit, dispatch to `app.handle_key(key.code)` for F/C/R/A keys.

Updated `run` skeleton:

```rust
fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = App::new_demo_hand();
    loop {
        let view = app.view();
        terminal.draw(|frame| ui::render(frame, &view))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                code => { app.handle_key(code); }
            }
        }
    }
}
```

- [ ] **Step 3: Run unit tests, then run the app**

Run: `cargo test -p pokertui app -- --nocapture`
Expected: 3 tests PASS.

Run: `cargo run -p pokertui`
Expected: TUI launches showing the engine-driven preflop state with 6 players. Pressing F/C/R/A executes legal actions; the log and seats update.

- [ ] **Step 4: Commit**

```bash
git add crates/pokertui/src/app.rs crates/pokertui/src/main.rs
git commit -m "feat(ui): App ties engine to renderer; F/C/R/A keys drive actions"
```

---

### Task 15: Manual verification + cleanup

**Files:** none (manual check + final commit).

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace -- --nocapture`
Expected: every test PASSes.

- [ ] **Step 2: Lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. If anything is flagged, fix it (do not allow it through).

- [ ] **Step 3: Play a full hot-seat hand**

Run: `cargo run -p pokertui`

Walk through one complete hand by pressing keys (passing the terminal between players in your head — you are every seat in turn). Confirm:

- Preflop, flop, turn, river all advance correctly.
- The board reveals the right number of cards each street.
- Folding reduces the active player count.
- Going to showdown awards the pot to one (or more, in a split) seats and ends the hand (phase shown as `COMPLETE`).
- Pot total in the title row matches the sum of contributions.

If any of the above is wrong, file a follow-up task; do not paper over.

- [ ] **Step 4: Final commit if cleanup was needed**

```bash
git add -A
git commit -m "chore: post-integration cleanup"
```

---

## Out of Scope (deferred)

- **Omaha.** Once Hold'em is solid, the path is: add `HoleCards::Four([Card; 4])`, change `evaluate_holdem_hand` to enumerate `C(4, 2) × C(5, 3) = 60` candidate 5-card hands and pick the best. The betting engine is untouched.
- **Bet-size adjustment UI.** R currently picks the minimum legal raise. Adding `↑`/`↓` to scale the raise toward pot/half-pot/all-in is a small follow-up — extend `App` with a `pending_raise_to: Option<u64>` and a confirm step.
- **Equity / pot-odds numbers in the rail.** The presentation `Phase` has `equity` and `odds_pct` fields. The adapter currently zeroes them. Adding a Monte Carlo equity computation (or pulling `rs_poker`'s) is a separate phase.
- **Bot opponents.** Pass-and-play is the MVP; AI agents are not part of this plan.
- **Persistence / hand histories.** The engine log is in-memory only.

## Verification

End-to-end checks for this plan (in order):

1. `cargo test --workspace` — every unit and integration test passes, including the side-pot integration tests in `crates/poker-core/tests/holdem_side_pots.rs`.
2. `cargo clippy --workspace --all-targets -- -D warnings` — clean.
3. `cargo run -p pokertui` — manual hand walk-through as described in Task 15, Step 3.
4. **Chip-conservation invariant:** at the end of every test that runs to `Phase::Complete`, `sum(stacks) == sum(starting_stacks)`. This is asserted in `holdem_walkthrough.rs` and `holdem_side_pots.rs` and is the single most important correctness gate for the engine.
