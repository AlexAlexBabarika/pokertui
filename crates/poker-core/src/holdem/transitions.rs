use super::state::HandState;
use super::types::{Action, ApplyError, PlayerId};

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
    if !is_legal(&state, action) {
        return Err((state, ApplyError::IllegalAction("illegal action")));
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
            debug_assert!(extra <= state.stacks[i]);
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
    }

    advance_actor(&mut state);
    Ok(state)
}

/// Move `to_act` to the next player who still needs to act, or `None` if the
/// round will close. Round-close handling (deal next street etc.) comes in Task 7;
/// for now we just walk the seat order and skip non-actionable seats.
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
    // No one left to act → round closes (handled in Task 7).
    state.to_act = None;
}

fn has_acted_and_matched(state: &HandState, idx: usize) -> bool {
    // A player is done for the round if their round_bet matches current_bet
    // AND they're not the last_aggressor (the aggressor is who closure returns to).
    //
    // For Fold/Check only (this task), this simplified check is sufficient.
    state.round_bet[idx] == state.current_bet
        && state.last_aggressor != Some(PlayerId(idx))
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
}
