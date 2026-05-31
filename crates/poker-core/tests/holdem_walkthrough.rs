use poker_core::holdem::{Action, HandConfig, HandState, Phase, PlayerId, apply};

fn cfg6(seed: u64) -> HandConfig {
    HandConfig {
        num_players: 6,
        small_blind: 50,
        big_blind: 100,
        dealer: PlayerId(0),
        seed,
    }
}

#[test]
fn full_hand_six_handed_limp_check_down() {
    let mut s = HandState::new_hand(cfg6(1), vec![10_000; 6]);
    // Preflop: 3, 4, 5, 0, 1 all call; 2 checks.
    for _ in 0..5 {
        s = apply(s, Action::Call).unwrap();
    }
    s = apply(s, Action::Check).unwrap();
    assert_eq!(s.phase, Phase::Flop);

    // Flop: everyone checks.
    for _ in 0..6 {
        s = apply(s, Action::Check).unwrap();
    }
    assert_eq!(s.phase, Phase::Turn);
    // Turn: everyone checks.
    for _ in 0..6 {
        s = apply(s, Action::Check).unwrap();
    }
    assert_eq!(s.phase, Phase::River);
    // River: everyone checks.
    for _ in 0..6 {
        s = apply(s, Action::Check).unwrap();
    }
    assert_eq!(s.phase, Phase::Complete);

    let total: u64 = s.stacks.iter().sum();
    assert_eq!(total, 60_000, "chips conserved over the whole hand");
}

#[test]
fn raise_and_fold_around() {
    let mut s = HandState::new_hand(cfg6(2), vec![10_000; 6]);
    // UTG raises to 300, others fold.
    s = apply(s, Action::Raise { to: 300 }).unwrap();
    for _ in 0..5 {
        s = apply(s, Action::Fold).unwrap();
    }
    assert_eq!(s.phase, Phase::Complete);
    // UTG wins pot of SB(50)+BB(100) = 150; UTG put in 300 then got it all back.
    assert_eq!(s.stacks[3], 10_000 + 150);
}
