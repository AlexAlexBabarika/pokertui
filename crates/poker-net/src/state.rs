use poker_core::Card;
use poker_core::holdem::{Action, HandState, LogKind, Phase, PlayerId};
use serde::{Deserialize, Serialize};

/// One seat as seen by a particular recipient. `hole` is `Some` only for the
/// recipient's own seat, or for any non-folded seat once the hand is `Complete`
/// (the showdown reveal). Every other seat's cards are `None` on the wire — they
/// are never serialized, so they cannot be recovered by inspecting traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicSeat {
    pub name: String,
    pub stack: u64,
    pub round_bet: u64,
    pub folded: bool,
    pub all_in: bool,
    pub hole: Option<[Card; 2]>,
    pub last_action: Option<Action>,
}

/// The whole table, filtered for one recipient. Carries everything the client
/// needs to render and to run its own equity simulation (its own hole cards plus
/// the public board), and nothing that would let it see another player's hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicState {
    pub small_blind: u64,
    pub big_blind: u64,
    pub dealer: PlayerId,
    pub num_players: usize,
    pub phase: Phase,
    pub board: Vec<Card>,
    pub seats: Vec<PublicSeat>,
    pub to_act: Option<PlayerId>,
    pub current_bet: u64,
    pub min_raise: u64,
    pub pot: u64,
    pub your_seat: PlayerId,
    pub winners: Vec<Vec<PlayerId>>,
}

impl PublicState {
    /// Build the view delivered to `recipient`. The ONLY hole cards included are
    /// the recipient's own, plus those of non-folded seats once the hand is
    /// `Complete` (showdown). `names` is indexed by seat.
    pub fn for_recipient(engine: &HandState, names: &[String], recipient: PlayerId) -> Self {
        let n = engine.config.num_players;
        let complete = engine.phase == Phase::Complete;

        let seats = (0..n)
            .map(|i| {
                let reveal = PlayerId(i) == recipient || (complete && !engine.folded[i]);
                let hole = reveal.then(|| engine.hole[i]);
                let round_bet = if complete { 0 } else { engine.round_bet[i] };
                PublicSeat {
                    name: names.get(i).cloned().unwrap_or_default(),
                    stack: engine.stacks[i],
                    round_bet,
                    folded: engine.folded[i],
                    all_in: engine.all_in[i],
                    hole,
                    last_action: last_action_of(engine, PlayerId(i)),
                }
            })
            .collect();

        // The live pot is the sum of contributions; once the hand is paid out
        // (`Complete`) it is zero, matching the local adapter's behaviour.
        let pot = if complete {
            0
        } else {
            engine.contributed.iter().sum()
        };

        PublicState {
            small_blind: engine.config.small_blind,
            big_blind: engine.config.big_blind,
            dealer: engine.config.dealer,
            num_players: n,
            phase: engine.phase,
            board: engine.board.clone(),
            seats,
            to_act: engine.to_act,
            current_bet: engine.current_bet,
            min_raise: engine.min_raise,
            pot,
            your_seat: recipient,
            winners: engine.winners.clone(),
        }
    }
}

/// The most recent committed `Action` by `seat` this hand, read off the log.
/// Blinds and board deals are not player actions, so they are skipped.
fn last_action_of(engine: &HandState, seat: PlayerId) -> Option<Action> {
    engine.log.iter().rev().find_map(|e| {
        if e.actor == Some(seat)
            && let LogKind::Action(a) = e.kind
        {
            return Some(a);
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use poker_core::holdem::{Action, HandConfig, apply};

    fn names() -> Vec<String> {
        vec![
            "nova".into(),
            "delta".into(),
            "gizmo".into(),
            "you".into(),
            "maple".into(),
            "rook".into(),
        ]
    }

    fn fresh() -> HandState {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        HandState::new_hand(cfg, vec![10_000; 6])
    }

    #[test]
    fn recipient_sees_only_their_own_hole_cards_preflop() {
        let engine = fresh();
        let view = PublicState::for_recipient(&engine, &names(), PlayerId(3));
        assert!(view.seats[3].hole.is_some(), "recipient sees own cards");
        for i in [0usize, 1, 2, 4, 5] {
            assert!(
                view.seats[i].hole.is_none(),
                "seat {i}'s cards must never be serialized to seat 3"
            );
        }
        // The recipient's own hand is recoverable for the client's equity sim.
        assert_eq!(view.your_seat, PlayerId(3));
        assert_eq!(view.seats[3].hole, Some(engine.hole[3]));
    }

    #[test]
    fn every_recipient_gets_a_distinct_filtered_view() {
        let engine = fresh();
        for r in 0..6 {
            let view = PublicState::for_recipient(&engine, &names(), PlayerId(r));
            let revealed: Vec<usize> = (0..6).filter(|&i| view.seats[i].hole.is_some()).collect();
            assert_eq!(revealed, vec![r], "only seat {r} is revealed to seat {r}");
        }
    }

    #[test]
    fn showdown_reveals_non_folded_seats_to_everyone() {
        // Fold around to the big blind: hand completes with one survivor (seat 2).
        let mut engine = fresh();
        for _ in 0..5 {
            engine = apply(engine, Action::Fold).unwrap();
        }
        assert_eq!(engine.phase, Phase::Complete);
        // Seat 5 is folded; it should still see the survivor's cards but not the
        // other mucked hands.
        let view = PublicState::for_recipient(&engine, &names(), PlayerId(5));
        assert!(
            view.seats[2].hole.is_some(),
            "survivor revealed at showdown"
        );
        assert!(
            view.seats[5].hole.is_some(),
            "recipient always sees own cards"
        );
        assert!(
            view.seats[4].hole.is_none(),
            "a folded non-recipient seat stays mucked"
        );
    }

    #[test]
    fn pot_and_round_bets_zero_out_once_complete() {
        let mut engine = fresh();
        for _ in 0..5 {
            engine = apply(engine, Action::Fold).unwrap();
        }
        let view = PublicState::for_recipient(&engine, &names(), PlayerId(0));
        assert_eq!(view.pot, 0, "pot is paid out at completion");
        assert!(view.seats.iter().all(|s| s.round_bet == 0));
        assert_eq!(view.winners.len(), 1, "one winning group recorded");
    }

    #[test]
    fn last_action_reflects_the_log() {
        let mut engine = fresh();
        engine = apply(engine, Action::Call).unwrap(); // UTG (seat 3) calls
        let view = PublicState::for_recipient(&engine, &names(), PlayerId(0));
        assert_eq!(view.seats[3].last_action, Some(Action::Call));
        assert_eq!(view.seats[0].last_action, None, "BTN has not acted");
    }
}
