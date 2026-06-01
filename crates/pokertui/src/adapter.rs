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
    // The cutoff is always the seat immediately to the right of the button,
    // i.e. the highest offset. Resolve it first so 5-handed (offset 4) labels
    // CO rather than falling through to the generic catch-all.
    if n >= 5 && offset == n - 1 {
        return "CO".into();
    }
    match offset {
        0 => "BTN".into(),
        1 => "SB".into(),
        2 => "BB".into(),
        3 => "UTG".into(),
        4 => "MP".into(), // only reached for n >= 6; n == 5 hits CO above
        // Catch-all for the middle seats of larger tables.
        _ => format!("P{offset}"),
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
        } else if engine.folded[i] && engine.stacks[i] == 0 {
            // A normally-folded player always keeps chips (you cannot fold while
            // all-in), so folded-and-broke uniquely marks a sat-out busted seat.
            SeatStatus::Busted
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
        // The hero always sees their own cards. At showdown the remaining
        // (non-folded) players reveal; folded players' cards stay hidden, as in
        // real poker — they mucked.
        let revealed_at_showdown = engine.phase == EnginePhase::Complete && !engine.folded[i];
        let hole_cards = if PlayerId(i) == names.hero || revealed_at_showdown {
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
            is_to_act: engine.to_act == Some(PlayerId(i)),
        });
    }

    // The hero's current made hand: their hole cards plus the visible board.
    let mut hero_cards = engine.hole[names.hero.0].to_vec();
    hero_cards.extend(engine.board.iter().copied());
    let rank = poker_core::combination_name(&hero_cards)
        .unwrap_or("—")
        .into();

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
        // Filled in by `App::view`, which owns the live raise selection.
        raise_to: None,
        equity: 0,
        odds_pct: 0.0,
        rank,
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
        // The App owns end-of-hand / game-over messaging; default to none here.
        notice: None,
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

    #[test]
    fn position_labels_cover_five_and_six_handed() {
        // 5-handed: cutoff is the last seat (offset 4), not a generic "P4".
        assert_eq!(position_label(0, 5), "BTN");
        assert_eq!(position_label(1, 5), "SB");
        assert_eq!(position_label(2, 5), "BB");
        assert_eq!(position_label(3, 5), "UTG");
        assert_eq!(position_label(4, 5), "CO");
        // 6-handed: offset 4 is MP, offset 5 (the last seat) is CO.
        assert_eq!(position_label(4, 6), "MP");
        assert_eq!(position_label(5, 6), "CO");
    }

    #[test]
    fn folded_players_keep_cards_hidden_at_showdown() {
        use poker_core::holdem::{Action, apply};
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        let mut engine = HandState::new_hand(cfg, vec![10_000; 6]);
        // Folds around to the BB(2): UTG(3), MP(4), CO(5), BTN(0), SB(1) fold.
        for _ in 0..5 {
            engine = apply(engine, Action::Fold).unwrap();
        }
        assert_eq!(engine.phase, EnginePhase::Complete);
        // Hero is PlayerId(3) per demo_six — folded, but still sees own cards.
        let view = to_presentation(&engine, &NameRegistry::demo_six());
        assert!(
            view.players[3].hole_cards.is_some(),
            "hero always sees own cards"
        );
        assert!(
            view.players[4].hole_cards.is_none(),
            "a folded non-hero player's cards stay hidden at showdown"
        );
        assert!(
            view.players[2].hole_cards.is_some(),
            "the surviving player (BB) reveals at showdown"
        );
    }
}
