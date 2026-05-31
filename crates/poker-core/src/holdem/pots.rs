use super::types::PlayerId;
use super::state::HandState;
use crate::{evaluate, Card, HandStrength};

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

    // Chip-conservation invariant: every player's chips (folded or not) must be
    // enclosed by some level, or they vanish. This holds for all states the
    // betting engine can produce — a folded player can never end as the strict
    // top contributor (you cannot fold while all-in, and an uncalled bet leaves
    // a single player who is paid out without ever calling `build_pots`). Guard
    // it so a future change (refunds, straddles) that breaks the assumption trips
    // a deliberate review instead of silently leaking chips.
    debug_assert!(
        levels.is_empty()
            || levels.last().copied().unwrap() >= contributed.iter().copied().max().unwrap_or(0),
        "a contributor exceeds the top live level; chips would leak from the pots"
    );

    let mut pots: Vec<Pot> = Vec::new();
    let mut prev = 0u64;
    for &lvl in &levels {
        let amount: u64 = (0..n)
            .map(|i| contributed[i].min(lvl).saturating_sub(prev))
            .sum::<u64>();
        if amount == 0 {
            prev = lvl;
            continue;
        }
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

/// Resolve every pot at showdown: rank eligible players' 7-card hands, split
/// ties, and pay out. Mutates `state.stacks`, populates `state.pots` and
/// `state.winners`, logs each award, and sets the hand to `Phase::Complete`.
///
/// Returns, per pot, the `(Pot, winners, amount)` tuple — useful for callers
/// that want to present the result without re-deriving it.
pub fn settle(state: &mut HandState) -> Vec<(Pot, Vec<PlayerId>, u64)> {
    let pots = build_pots(&state.contributed, &state.folded);
    let mut results = Vec::with_capacity(pots.len());
    for (pot_idx, pot) in pots.iter().enumerate() {
        let winners = best_among(&pot.eligible, state);
        let share = pot.amount / winners.len() as u64;
        let mut leftover = pot.amount - share * winners.len() as u64;

        // Base even share.
        for &PlayerId(idx) in &winners {
            state.stacks[idx] += share;
            state.log.push(super::state::LogEntry {
                actor: Some(PlayerId(idx)),
                kind: super::state::LogKind::WinPot { pot_idx, amount: share },
            });
        }
        // Odd chips: distribute one at a time, starting left of the dealer.
        let n = state.config.num_players;
        let mut step = 1usize;
        while leftover > 0 {
            let cand = (state.config.dealer.0 + step) % n;
            if winners.contains(&PlayerId(cand)) {
                state.stacks[cand] += 1;
                leftover -= 1;
            }
            step += 1;
            if step > 2 * n {
                break; // safety: should never trigger when winners is non-empty
            }
        }
        // Every odd chip must land; a non-zero leftover would be a silent leak.
        debug_assert_eq!(leftover, 0, "odd chips left undistributed");
        results.push((pot.clone(), winners, pot.amount));
    }
    state.pots = pots;
    state.winners = results.iter().map(|(_, w, _)| w.clone()).collect();
    state.phase = super::types::Phase::Complete;
    state.to_act = None;
    results
}

/// The eligible player(s) holding the best 7-card hand. Returns all on a tie.
fn best_among(eligible: &[PlayerId], state: &HandState) -> Vec<PlayerId> {
    let mut best: Option<HandStrength> = None;
    let mut winners: Vec<PlayerId> = Vec::new();
    for &PlayerId(idx) in eligible {
        let mut seven: Vec<Card> = state.board.clone();
        seven.extend_from_slice(&state.hole[idx]);
        let strength = evaluate(&seven);
        match best {
            None => {
                best = Some(strength);
                winners.push(PlayerId(idx));
            }
            Some(curr) if strength > curr => {
                best = Some(strength);
                winners.clear();
                winners.push(PlayerId(idx));
            }
            Some(curr) if strength == curr => {
                winners.push(PlayerId(idx));
            }
            _ => {}
        }
    }
    winners
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

    /// The master invariant: pots must account for every chip contributed.
    /// Sweep a broad reachable input space and assert no chips appear or vanish.
    #[test]
    fn pots_conserve_every_chip() {
        let values = [0u64, 1, 2, 3, 5];
        for n in 1..=4usize {
            // Enumerate every (contribution, folded) combination for n players.
            let combos = values.len() * 2;
            for code in 0..combos.pow(n as u32) {
                let mut contributed = vec![0u64; n];
                let mut folded = vec![false; n];
                let mut rem = code;
                for p in 0..n {
                    let slot = rem % combos;
                    rem /= combos;
                    contributed[p] = values[slot / 2];
                    folded[p] = slot % 2 == 1;
                }
                // Skip the (engine-unreachable) state where a contributor exceeds
                // the top live level — build_pots intentionally drops those chips
                // and a debug_assert guards it.
                let top_level = (0..n)
                    .filter(|&i| !folded[i])
                    .map(|i| contributed[i])
                    .max()
                    .unwrap_or(0);
                if contributed.iter().copied().max().unwrap_or(0) > top_level {
                    continue;
                }
                let pots = build_pots(&contributed, &folded);
                let pot_total: u64 = pots.iter().map(|p| p.amount).sum();
                let contributed_total: u64 = contributed.iter().sum();
                assert_eq!(
                    pot_total, contributed_total,
                    "chip leak for contributed={contributed:?} folded={folded:?}"
                );
            }
        }
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

    use crate::holdem::state::HandState;
    use crate::holdem::transitions::apply;
    use crate::holdem::types::{Action, HandConfig};

    fn cfg_hu(seed: u64) -> HandConfig {
        HandConfig {
            num_players: 2,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed,
        }
    }

    fn cfg_4(seed: u64) -> HandConfig {
        HandConfig {
            num_players: 4,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed,
        }
    }

    #[test]
    fn heads_up_all_in_conserves_chips_and_picks_one_winner_set() {
        let mut s = HandState::new_hand(cfg_hu(42), vec![1_000, 1_000]);
        s = apply(s, Action::AllIn).unwrap(); // SB (dealer) shoves
        s = apply(s, Action::Call).unwrap(); // BB calls all-in
        // Round closure ran the board out and called settle via the Showdown branch.
        assert_eq!(s.phase, super::super::types::Phase::Complete);
        let total: u64 = s.stacks.iter().sum();
        assert_eq!(total, 2_000, "all chips conserved at showdown");
        // Exactly one pot, and winners recorded.
        assert_eq!(s.pots.len(), 1);
        assert_eq!(s.winners.len(), 1);
        assert!(!s.winners[0].is_empty(), "a winner was chosen");
    }

    #[test]
    fn settle_directly_splits_a_tied_main_pot_evenly() {
        // Construct a contrived state where two players tie on the board.
        // Board is a royal flush in spades → both players "play the board" and tie.
        let mut s = HandState::new_hand(cfg_hu(1), vec![1_000, 1_000]);
        // Overwrite board + hole so both players' best 5 is the board itself.
        s.board = vec![
            Card::parse("As"),
            Card::parse("Ks"),
            Card::parse("Qs"),
            Card::parse("Js"),
            Card::parse("Ts"),
        ];
        s.hole = vec![
            [Card::parse("2c"), Card::parse("3d")],
            [Card::parse("4c"), Card::parse("5d")],
        ];
        // Both contributed 500; neither folded.
        s.contributed = vec![500, 500];
        s.folded = vec![false, false];
        s.stacks = vec![0, 0];
        let results = super::settle(&mut s);
        assert_eq!(results.len(), 1);
        let (_, winners, amount) = &results[0];
        assert_eq!(*amount, 1_000);
        assert_eq!(winners.len(), 2, "both play the board → tie");
        // Even split, no odd chip.
        assert_eq!(s.stacks, vec![500, 500]);
    }

    #[test]
    fn settle_awards_odd_chip_to_first_eligible_left_of_dealer() {
        // A three-way tied pot whose amount is not divisible by 3 exercises the
        // odd-chip rule. Folded P3 contributes 1 extra chip into the main pot,
        // making it 301 with three eligible winners → share 100, leftover 1.
        // The leftover goes to the first winner clockwise from the dealer (0),
        // i.e. P1 (step=1 → cand=1, which is a winner).
        let mut s = HandState::new_hand(cfg_4(1), vec![1_000; 4]);
        // Spades royal flush board → every live player plays the board and ties.
        s.board = vec![
            Card::parse("As"),
            Card::parse("Ks"),
            Card::parse("Qs"),
            Card::parse("Js"),
            Card::parse("Ts"),
        ];
        s.hole = vec![
            [Card::parse("2c"), Card::parse("3d")],
            [Card::parse("4c"), Card::parse("5d")],
            [Card::parse("6c"), Card::parse("7d")],
            [Card::parse("8c"), Card::parse("9d")],
        ];
        // P0,P1,P2 each in for 100; P3 folded after putting in 1.
        s.contributed = vec![100, 100, 100, 1];
        s.folded = vec![false, false, false, true];
        s.stacks = vec![0, 0, 0, 0];
        let results = super::settle(&mut s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].2, 301); // pot amount: 100*3 + 1 folded chip
        assert_eq!(results[0].1.len(), 3); // three-way tie
        // share = 100 each, leftover 1 → first winner left of dealer 0 is idx 1.
        assert_eq!(s.stacks, vec![100, 101, 100, 0]);
        // Chip conservation: payouts equal the pot total.
        let total: u64 = s.stacks.iter().sum();
        assert_eq!(total, 301);
    }
}
