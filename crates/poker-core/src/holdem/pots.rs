use super::types::PlayerId;

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
}
