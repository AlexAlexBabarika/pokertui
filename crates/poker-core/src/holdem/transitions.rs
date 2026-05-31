use super::state::HandState;
use super::types::{Action, ApplyError};

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

/// Apply one action by the current actor. Returns the new state on success,
/// or `(state_unchanged, error)` on failure. Full implementation comes in
/// later tasks; for now this is a stub so the public re-export resolves.
pub fn apply(state: HandState, _action: Action) -> Result<HandState, (HandState, ApplyError)> {
    Err((state, ApplyError::IllegalAction("not yet implemented")))
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
