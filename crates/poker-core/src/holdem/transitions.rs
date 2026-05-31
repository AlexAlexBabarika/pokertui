use super::state::HandState;
use super::types::{Action, ApplyError, Phase, PlayerId};

/// Return every legal action for the player in `state.to_act`.
/// Returns empty if no one is to act (hand complete or between rounds).
pub fn legal_actions(state: &HandState) -> Vec<Action> {
    let Some(actor) = state.to_act else {
        return Vec::new();
    };
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
        let min_to = state.config.big_blind;
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

/// Returns true if `action` is legal for the current actor.
/// Bet/Raise variants here take *any* `to` between the min and max,
/// not just the minimum the `legal_actions` summary returns.
fn is_legal(state: &HandState, action: Action) -> bool {
    let Some(actor) = state.to_act else {
        return false;
    };
    let i = actor.0;
    if state.folded[i] || state.all_in[i] {
        return false;
    }
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

/// Apply one action by the current actor. Returns the new state on success,
/// or `(state_unchanged, error)` on failure.
pub fn apply(mut state: HandState, action: Action) -> Result<HandState, (HandState, ApplyError)> {
    let Some(actor) = state.to_act else {
        return Err((state, ApplyError::HandComplete));
    };
    let i = actor.0;
    if !is_legal(&state, action) {
        return Err((state, ApplyError::IllegalAction("illegal action")));
    }

    match action {
        Action::Fold => {
            state.folded[i] = true;
            state.acted_this_round[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Fold),
            });
        }
        Action::Check => {
            state.acted_this_round[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Check),
            });
        }
        Action::Call => {
            let to_call = state.current_bet - state.round_bet[i];
            let pay = to_call.min(state.stacks[i]);
            state.stacks[i] -= pay;
            state.contributed[i] += pay;
            state.round_bet[i] += pay;
            if state.stacks[i] == 0 {
                state.all_in[i] = true;
            }
            state.acted_this_round[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Call),
            });
        }
        Action::Bet { to } => {
            let extra = to - state.round_bet[i];
            debug_assert!(extra <= state.stacks[i]);
            state.stacks[i] -= extra;
            state.round_bet[i] += extra;
            state.contributed[i] += extra;
            state.current_bet = to;
            state.min_raise = to;
            state.last_aggressor = Some(actor);
            if state.stacks[i] == 0 {
                state.all_in[i] = true;
            }
            state.acted_this_round[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::Bet { to }),
            });
        }
        Action::Raise { to } => {
            let raise_size = to - state.current_bet;
            let extra = to - state.round_bet[i];
            debug_assert!(extra <= state.stacks[i]);
            state.stacks[i] -= extra;
            state.round_bet[i] += extra;
            state.contributed[i] += extra;
            state.current_bet = to;
            state.min_raise = raise_size;
            state.last_aggressor = Some(actor);
            if state.stacks[i] == 0 {
                state.all_in[i] = true;
            }
            state.acted_this_round[i] = true;
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
            state.acted_this_round[i] = true;
            state.log.push(super::state::LogEntry {
                actor: Some(actor),
                kind: super::state::LogKind::Action(Action::AllIn),
            });
        }
    }

    advance_actor(&mut state);
    Ok(state)
}

fn advance_actor(state: &mut HandState) {
    if try_terminate_by_folds(state) {
        return;
    }
    if round_is_closed(state) {
        close_round_and_advance(state);
        return;
    }
    let n = state.config.num_players;
    let Some(PlayerId(curr)) = state.to_act else {
        return;
    };
    for step in 1..=n {
        let cand = (curr + step) % n;
        if state.folded[cand] || state.all_in[cand] {
            continue;
        }
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
        let winner = active[0];
        let total: u64 = state.contributed.iter().sum();
        state.stacks[winner] += total;
        state.pots.push(super::pots::Pot {
            amount: total,
            eligible: vec![PlayerId(winner)],
        });
        state.winners.push(vec![PlayerId(winner)]);
        state.log.push(super::state::LogEntry {
            actor: Some(PlayerId(winner)),
            kind: super::state::LogKind::WinPot {
                pot_idx: 0,
                amount: total,
            },
        });
        state.phase = Phase::Complete;
        state.to_act = None;
        return true;
    }
    false
}

fn round_is_closed(state: &HandState) -> bool {
    let n = state.config.num_players;
    let actionable: Vec<usize> = (0..n)
        .filter(|&i| !state.folded[i] && !state.all_in[i])
        .collect();
    if actionable.is_empty() {
        return true;
    }
    let all_matched = actionable
        .iter()
        .all(|&i| state.round_bet[i] == state.current_bet);
    if !all_matched {
        return false;
    }
    state.all_have_acted_this_round(&actionable)
}

fn close_round_and_advance(state: &mut HandState) {
    // Reset round-level state.
    state.round_bet.iter_mut().for_each(|b| *b = 0);
    state.current_bet = 0;
    state.min_raise = state.config.big_blind;
    state.last_aggressor = None;
    state.acted_this_round.iter_mut().for_each(|b| *b = false);

    // Determine if there's still real betting to do (≥ 2 actionable players).
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
            let c = [
                state.deck.remove(0),
                state.deck.remove(0),
                state.deck.remove(0),
            ];
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
            state.log.push(super::state::LogEntry {
                actor: None,
                kind: super::state::LogKind::Showdown,
            });
            super::pots::settle(state);
            return;
        }
        _ => {}
    }

    if runout && !matches!(next, Phase::Showdown) {
        // Safe to recurse: `state.phase` strictly advances each call along
        // Preflop → Flop → Turn → River → Showdown, capping recursion at ≤4.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdem::state::HandState;
    use crate::holdem::types::{HandConfig, Phase, PlayerId};

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
        assert!(
            actions.contains(&Action::Raise { to: 200 }),
            "min-raise to 200"
        );
        assert!(actions.contains(&Action::AllIn));
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Check)),
            "cannot check facing the BB"
        );
    }

    #[test]
    fn short_stacked_actor_cannot_raise_only_shove() {
        let mut stacks = vec![10_000u64; 6];
        stacks[3] = 150; // UTG has 150 — call costs 100, can't make min-raise (200)
        let s = HandState::new_hand(cfg6(), stacks);
        let actions = legal_actions(&s);
        assert!(actions.contains(&Action::Call));
        assert!(actions.contains(&Action::AllIn));
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Raise { .. })),
            "stack too small for min-raise; only AllIn covers the raise"
        );
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
            num_players: 2,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 7,
        };
        let s = HandState::new_hand(cfg, vec![1_000, 1_000]);
        let s = apply(s, Action::AllIn).unwrap(); // SB shoves
        let s = apply(s, Action::Call).unwrap(); // BB calls (also all-in)
        assert_eq!(s.phase, Phase::Complete);
        assert_eq!(
            s.board.len(),
            5,
            "all 5 community cards dealt before showdown"
        );
    }
}
