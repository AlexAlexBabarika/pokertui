use poker_core::holdem::{HandState, Phase as EnginePhase, PlayerId};

use crate::state::{ChatLine, GameState, LogEntry, LogTone, Phase, Seat, SeatStatus};

pub struct NameRegistry {
    pub names: Vec<String>, // indexed by PlayerId
    pub hero: PlayerId,
}

impl NameRegistry {
    pub fn demo_six() -> Self {
        Self {
            names: vec![
                "nova".into(),
                "delta".into(),
                "gizmo".into(),
                "you".into(),
                "maple".into(),
                "rook".into(),
            ],
            hero: PlayerId(3),
        }
    }
}

/// Position labels for a standard 6-handed table, indexed by offset from the dealer.
fn position_label(offset: usize, n: usize) -> String {
    if n == 2 {
        return if offset == 0 {
            "BTN".into()
        } else {
            "BB".into()
        };
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
        let last_action = engine
            .log
            .iter()
            .rev()
            .find_map(|e| {
                if e.actor == Some(PlayerId(i)) {
                    Some(format_log_kind(&e.kind))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "—".into());
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
        // Once the hand is complete the pot has been paid into the winners'
        // stacks, so the live pot is empty. `contributed` is never reset, so
        // showing its sum at `Complete` would display the already-awarded pot.
        pot: if engine.phase == EnginePhase::Complete {
            0
        } else {
            engine.contributed.iter().sum()
        },
        to_call: to_call_for_hero(engine, names.hero),
        equity: 0,
        odds_pct: 0.0,
        rank: "—".into(),
        hint: "—".into(),
    };

    let log: Vec<LogEntry> = engine
        .log
        .iter()
        .map(|e| LogEntry {
            who: e
                .actor
                .map(|p| names.names[p.0].clone())
                .unwrap_or_else(|| "·".into()),
            what: format_log_kind(&e.kind),
            tone: tone_for(&e.kind),
        })
        .collect();

    GameState {
        blinds: format!(
            "{} / {}",
            engine.config.small_blind, engine.config.big_blind
        ),
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
    use poker_core::holdem::Action::*;
    use poker_core::holdem::LogKind::*;
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
    use poker_core::holdem::Action::*;
    use poker_core::holdem::LogKind::*;
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
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
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

    #[test]
    fn completed_hand_shows_empty_pot() {
        use poker_core::holdem::{Action, apply};
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        let mut engine = HandState::new_hand(cfg, vec![10_000; 6]);
        // Everyone folds to the BB → hand completes, pot is awarded.
        for _ in 0..5 {
            engine = apply(engine, Action::Fold).unwrap();
        }
        assert_eq!(engine.phase, EnginePhase::Complete);
        let view = to_presentation(&engine, &NameRegistry::demo_six());
        assert_eq!(view.phase.pot, 0, "pot is empty once it has been paid out");
    }
}
