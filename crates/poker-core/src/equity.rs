use crate::{Card, Rank, Suit, evaluate};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

/// Result of an equity simulation, as fractions of total trials.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equity {
    /// Fraction of trials where the hero held the single best hand.
    pub win: f64,
    /// Fraction of trials where the hero tied for best (a split pot).
    pub tie: f64,
    /// Fraction of trials where at least one opponent beat the hero.
    pub lose: f64,
}

impl Equity {
    /// Headline "win chance" for the UI
    pub fn pct(&self) -> f64 {
        (self.win + self.tie * 0.5) * 100.0
    }
}

/// Monte Carlo hero equity vs `num_opponents` opponents holding uniformly
/// random hands, simulated over the cards not already known (the hero's hole
/// cards plus the visible `board`). `board` must hold 0, 3, 4, or 5 cards.
/// Deterministic for a given `seed`.
pub fn hero_equity(
    hole: [Card; 2],
    board: &[Card],
    num_opponents: usize,
    iters: u32,
    seed: u64,
) -> Equity {
    // An uncontested pot needs no simulation — the hero always wins it.
    if num_opponents == 0 || iters == 0 {
        return Equity {
            win: 1.0,
            tie: 0.0,
            lose: 0.0,
        };
    }

    // The unknown cards are the deck minus everything we can already see.
    let mut known = vec![hole[0], hole[1]];
    known.extend_from_slice(board);
    let unknown: Vec<Card> = full_deck()
        .into_iter()
        .filter(|c| !known.contains(c))
        .collect();

    // Each trial draws 2 cards per opponent plus enough to fill the board to 5.
    let needed = num_opponents * 2 + (5 - board.len());
    let mut deck = unknown;
    let mut rng = StdRng::seed_from_u64(seed);

    let hero_known = [hole[0], hole[1]];
    let mut wins = 0u32;
    let mut ties = 0u32;

    let mut hero7: Vec<Card> = Vec::with_capacity(7);
    let mut opp7: Vec<Card> = Vec::with_capacity(7);

    for _ in 0..iters {
        // Only shuffle the prefix we need rather than the whole deck each time.
        deck.partial_shuffle(&mut rng, needed);

        // Complete the shared board from the tail of the dealt prefix.
        let runout = &deck[num_opponents * 2..needed];
        hero7.clear();
        hero7.extend_from_slice(&hero_known);
        hero7.extend_from_slice(board);
        hero7.extend_from_slice(runout);
        let hero_strength = evaluate(&hero7);

        // Compare against each opponent; the hero needs to beat them all.
        let mut hero_best = true; // strictly best so far
        let mut split = false; // tied with someone for best
        for i in 0..num_opponents {
            opp7.clear();
            opp7.extend_from_slice(&deck[i * 2..i * 2 + 2]);
            opp7.extend_from_slice(board);
            opp7.extend_from_slice(runout);
            let opp_strength = evaluate(&opp7);
            if opp_strength > hero_strength {
                hero_best = false;
                break;
            } else if opp_strength == hero_strength {
                split = true;
            }
        }

        if hero_best {
            if split {
                ties += 1;
            } else {
                wins += 1;
            }
        }
    }

    let total = iters as f64;
    let win = wins as f64 / total;
    let tie = ties as f64 / total;
    Equity {
        win,
        tie,
        lose: 1.0 - win - tie,
    }
}

/// A full 52-card deck, ordered. (Order is irrelevant — every trial shuffles.)
fn full_deck() -> Vec<Card> {
    let mut cards = Vec::with_capacity(52);
    for &suit in &Suit::ALL {
        for &rank in &Rank::ALL {
            cards.push(Card::new(rank, suit));
        }
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<Card> {
        s.split_whitespace().map(Card::parse).collect()
    }

    fn hole(s: &str) -> [Card; 2] {
        let v = h(s);
        [v[0], v[1]]
    }

    #[test]
    fn pocket_aces_beats_one_random_opponent_about_85_percent() {
        let eq = hero_equity(hole("As Ah"), &[], 1, 20_000, 1);
        assert!(
            (eq.pct() - 85.0).abs() < 1.5,
            "AA vs 1 random ≈ 85%, got {:.1}%",
            eq.pct()
        );
    }

    #[test]
    fn ak_suited_beats_one_random_opponent_about_67_percent() {
        let eq = hero_equity(hole("As Ks"), &[], 1, 20_000, 7);
        assert!(
            (eq.pct() - 67.0).abs() < 1.5,
            "AKs vs 1 random ≈ 67%, got {:.1}%",
            eq.pct()
        );
    }

    #[test]
    fn nuts_on_full_board_wins_outright() {
        // Hero holds the nut flush; a full board is already decided, so equity
        // is exact regardless of the opponent's random holding.
        let board = h("2h 7h 9h Kd 3c");
        let eq = hero_equity(hole("Ah Qh"), &board, 1, 5_000, 3);
        assert!(
            eq.pct() > 99.0,
            "nut flush on the river ≈ 100%, got {:.1}%",
            eq.pct()
        );
    }

    #[test]
    fn no_opponents_is_certain_win() {
        let eq = hero_equity(hole("2c 7d"), &[], 0, 5_000, 1);
        assert_eq!(eq.pct(), 100.0, "uncontested pot is a certain win");
    }

    #[test]
    fn same_seed_produces_identical_result() {
        let a = hero_equity(hole("Jc Jd"), &h("2s 8h Td"), 3, 5_000, 42);
        let b = hero_equity(hole("Jc Jd"), &h("2s 8h Td"), 3, 5_000, 42);
        assert_eq!(a, b, "equity is deterministic for a fixed seed");
    }
}
