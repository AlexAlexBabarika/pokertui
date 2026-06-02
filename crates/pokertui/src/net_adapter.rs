use poker_core::Card;
use poker_core::holdem::{Action, PlayerId};
use poker_net::state::PublicState;

use crate::adapter::{phase_label, position_label, pot_odds_pct};
use crate::state::{ChatLine, GameState, LogEntry, Phase, Seat, SeatStatus};

/// Build the renderer's `GameState` from a filtered `PublicState`. Equity,
/// raise selection, and the end-of-hand notice are layered on by `NetClient`
/// (they depend on client-only state), so they are left at their defaults here.
pub(crate) fn to_presentation_net(
    state: &PublicState,
    feed: &[LogEntry],
    chat: &[ChatLine],
) -> GameState {
    let n = state.num_players;
    let hero = state.your_seat;

    let players = (0..n)
        .map(|i| {
            let seat = &state.seats[i];
            let offset = (i + n - state.dealer.0) % n;
            let status = if PlayerId(i) == hero {
                SeatStatus::Hero
            } else if seat.folded && seat.stack == 0 {
                SeatStatus::Busted
            } else if seat.folded {
                SeatStatus::Folded
            } else if matches!(
                seat.last_action,
                Some(Action::Bet { .. } | Action::Raise { .. } | Action::AllIn)
            ) {
                SeatStatus::Bet
            } else {
                SeatStatus::Active
            };
            Seat {
                name: seat.name.clone(),
                pos: position_label(offset, n),
                stack: seat.stack,
                status,
                last_action: seat
                    .last_action
                    .map(format_action)
                    .unwrap_or_else(|| "—".into()),
                hole_cards: seat.hole,
                is_to_act: state.to_act == Some(PlayerId(i)),
                round_bet: seat.round_bet,
            }
        })
        .collect();

    // The hero's made hand from their own (revealed) hole cards plus the board.
    let rank = match state.seats[hero.0].hole {
        Some(hole) => {
            let mut cards: Vec<Card> = hole.to_vec();
            cards.extend(state.board.iter().copied());
            poker_core::combination_name(&cards).unwrap_or("—").into()
        }
        None => "—".into(),
    };

    let to_call = state
        .current_bet
        .saturating_sub(state.seats[hero.0].round_bet);

    let phase = Phase {
        label: phase_label(state.phase),
        board: state.board.clone(),
        dealt: state.board.len(),
        pot: state.pot,
        to_call,
        raise_to: None, // NetClient fills this from its raise selection.
        equity: 0,      // NetClient fills this from its cached simulation.
        odds_pct: pot_odds_pct(to_call, state.pot),
        rank,
    };

    GameState {
        blinds: format!("{} / {}", state.small_blind, state.big_blind),
        players,
        phase,
        log: feed.to_vec(),
        chat: chat.to_vec(),
        notice: None, // NetClient fills this (waiting / hand complete / error).
        show_win_rate: true,
    }
}

/// Render one engine `Action` as the short label the feed/roster shows.
pub(crate) fn format_action(a: Action) -> String {
    match a {
        Action::Fold => "fold".into(),
        Action::Check => "check".into(),
        Action::Call => "call".into(),
        Action::Bet { to } => format!("bet {to}"),
        Action::Raise { to } => format!("raise {to}"),
        Action::AllIn => "all-in".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poker_core::holdem::{HandConfig, HandState, apply};

    fn names() -> Vec<String> {
        ["nova", "delta", "gizmo", "you", "maple", "rook"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn public_for(seat: usize) -> PublicState {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        let mut engine = HandState::new_hand(cfg, vec![10_000; 6]);
        engine = apply(engine, Action::Call).unwrap(); // UTG (3) calls
        PublicState::for_recipient(&engine, &names(), PlayerId(seat))
    }

    #[test]
    fn the_recipient_is_the_hero_and_sees_their_own_rank() {
        let state = public_for(3);
        let gs = to_presentation_net(&state, &[], &[]);
        assert_eq!(gs.players[3].status, SeatStatus::Hero);
        assert!(gs.players[3].hole_cards.is_some(), "hero sees own cards");
        assert_ne!(gs.phase.rank, "—", "hero has a made-hand label");
    }

    #[test]
    fn non_hero_cards_are_hidden_and_positions_are_labelled() {
        let state = public_for(3);
        let gs = to_presentation_net(&state, &[], &[]);
        assert!(gs.players[0].hole_cards.is_none(), "opponents stay hidden");
        assert_eq!(gs.players[0].pos, "BTN");
        assert_eq!(gs.players[3].pos, "UTG");
    }

    #[test]
    fn a_caller_is_shown_with_its_last_action() {
        let state = public_for(0);
        let gs = to_presentation_net(&state, &[], &[]);
        assert_eq!(gs.players[3].last_action, "call");
        assert_eq!(gs.blinds, "50 / 100");
    }
}
