use poker_core::holdem::{Action, HandConfig, HandState, Phase, PlayerId, apply};

fn cfg(n: usize, seed: u64) -> HandConfig {
    HandConfig {
        num_players: n,
        small_blind: 50,
        big_blind: 100,
        dealer: PlayerId(0),
        seed,
    }
}

#[test]
fn three_way_all_in_three_pots() {
    // P0 short (200), P1 mid (500), P2 big (10_000)
    let mut s = HandState::new_hand(cfg(3, 99), vec![200, 500, 10_000]);
    // In 3-handed: dealer=0 (BTN), 1=SB, 2=BB, action starts on BTN (idx 0).
    s = apply(s, Action::AllIn).unwrap(); // BTN shoves 200
    s = apply(s, Action::AllIn).unwrap(); // SB shoves 500
    s = apply(s, Action::AllIn).unwrap(); // BB shoves 10_000
    assert_eq!(s.phase, Phase::Complete);

    let total: u64 = s.stacks.iter().sum();
    assert_eq!(
        total,
        200 + 500 + 10_000,
        "chips conserved across multi-way all-in"
    );

    // Three pots, smallest first.
    assert_eq!(s.pots.len(), 3);
    assert_eq!(s.pots[0].amount, 600, "main pot = 200 * 3");
    assert_eq!(s.pots[1].amount, 600, "side pot 1 = (500-200)*2");
    assert_eq!(s.pots[2].amount, 9_500, "side pot 2 = (10_000-500)*1");
}

#[test]
fn folded_player_chips_count_toward_main_pot() {
    // 4-handed: BTN(0)=200 (short), SB(1)=400 (mid, will fold), BB(2)=10_000, UTG(3)=10_000
    let mut s = HandState::new_hand(cfg(4, 17), vec![200, 400, 10_000, 10_000]);
    // Preflop action: UTG raises to 300, BTN shoves 200 (under-raise),
    // SB folds (loses 50 SB), BB calls 300.
    s = apply(s, Action::Raise { to: 300 }).unwrap(); // UTG raises to 300
    s = apply(s, Action::AllIn).unwrap(); // BTN shoves 200
    s = apply(s, Action::Fold).unwrap(); // SB folds, loses 50
    s = apply(s, Action::Call).unwrap(); // BB calls 300

    // UTG already put 300; current_bet = 300; UTG is last_aggressor.
    // Action is back to UTG, who already matched → round closes.
    // Run out the board (only BB + UTG can act postflop; BTN is all-in).
    // Postflop is checked down because they have no incentive (no model: we just check).
    loop {
        if matches!(s.phase, Phase::Complete) {
            break;
        }
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
