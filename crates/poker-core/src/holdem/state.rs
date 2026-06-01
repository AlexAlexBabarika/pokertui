use super::pots::Pot;
use super::types::{HandConfig, Phase, PlayerId};
use crate::Card;
use crate::Deck;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub actor: Option<PlayerId>,
    pub kind: LogKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    PostBlind { amount: u64, is_big: bool },
    Action(super::types::Action),
    DealFlop([Card; 3]),
    DealTurn(Card),
    DealRiver(Card),
    Showdown,
    WinPot { pot_idx: usize, amount: u64 },
}

#[derive(Debug, Clone)]
pub struct HandState {
    pub config: HandConfig,
    pub phase: Phase,
    pub board: Vec<Card>,
    pub hole: Vec<[Card; 2]>,
    pub stacks: Vec<u64>,
    pub contributed: Vec<u64>,
    pub round_bet: Vec<u64>,
    pub folded: Vec<bool>,
    pub all_in: Vec<bool>,
    pub acted_this_round: Vec<bool>,
    pub to_act: Option<PlayerId>,
    pub current_bet: u64,
    pub min_raise: u64,
    pub last_aggressor: Option<PlayerId>,
    pub log: Vec<LogEntry>,
    pub pots: Vec<Pot>,
    pub winners: Vec<Vec<PlayerId>>,
    #[allow(dead_code)]
    pub(crate) deck: Vec<Card>,
}

impl HandState {
    /// Build a new hand: shuffle deck (deterministically), post blinds, deal
    /// two hole cards per player, set `to_act` to the player UTG.
    pub fn new_hand(config: HandConfig, stacks: Vec<u64>) -> Self {
        assert!(
            (2..=9).contains(&config.num_players),
            "num_players must be in 2..=9"
        );
        assert_eq!(
            stacks.len(),
            config.num_players,
            "stacks must match num_players"
        );
        assert!(config.big_blind > 0, "big blind must be positive");
        assert!(
            config.small_blind > 0 && config.small_blind < config.big_blind,
            "SB in (0, BB)"
        );
        assert!(
            config.dealer.0 < config.num_players,
            "dealer index in range"
        );

        let n = config.num_players;

        // Seats with no chips sit out: they start folded, post no blind, are
        // dealt no role, and never act. The hand's table size for blind/position
        // purposes is the number of *funded* seats, not the seat count.
        let funded: Vec<usize> = (0..n).filter(|&i| stacks[i] > 0).collect();
        assert!(funded.len() >= 2, "need at least 2 funded seats");
        assert!(stacks[config.dealer.0] > 0, "dealer must be a funded seat");
        let folded: Vec<bool> = (0..n).map(|i| stacks[i] == 0).collect();

        let mut deck = Deck::new();
        deck.shuffle_with_seed(config.seed);
        let mut deck: Vec<Card> = deck.cards().to_vec();

        // Deal 2 hole cards per player, alternating (real poker dealing order).
        // Sitting-out seats are folded, so their cards are mucked and never seen,
        // but every seat keeps a slot so per-seat vectors stay length `n`.
        let mut hole: Vec<[Card; 2]> = Vec::with_capacity(n);
        for _ in 0..n {
            hole.push([
                deck.remove(0),
                Card::new(crate::Rank::Two, crate::Suit::Clubs),
            ]);
        }
        for slot in hole.iter_mut() {
            slot[1] = deck.remove(0);
        }

        // Next funded seat clockwise from `from` (excluding `from`).
        let next_funded = |from: usize| -> usize {
            let mut c = (from + 1) % n;
            while stacks[c] == 0 {
                c = (c + 1) % n;
            }
            c
        };

        // Blind positions, walking only funded seats.
        // Heads-up (2 funded): dealer is SB, the other is BB, dealer acts first.
        // 3+ funded: SB = next funded after dealer, BB = next after SB, UTG next.
        let dealer = config.dealer.0;
        let (sb_idx, bb_idx, first_to_act) = if funded.len() == 2 {
            let other = next_funded(dealer);
            (dealer, other, dealer)
        } else {
            let sb = next_funded(dealer);
            let bb = next_funded(sb);
            let utg = next_funded(bb);
            (sb, bb, utg)
        };

        let mut stacks = stacks;
        let mut contributed = vec![0u64; n];
        let mut round_bet = vec![0u64; n];
        let mut all_in = vec![false; n];
        let mut log: Vec<LogEntry> = Vec::new();

        // Post small blind (capped at stack).
        let sb_amount = config.small_blind.min(stacks[sb_idx]);
        stacks[sb_idx] -= sb_amount;
        contributed[sb_idx] += sb_amount;
        round_bet[sb_idx] += sb_amount;
        if stacks[sb_idx] == 0 {
            all_in[sb_idx] = true;
        }
        log.push(LogEntry {
            actor: Some(PlayerId(sb_idx)),
            kind: LogKind::PostBlind {
                amount: sb_amount,
                is_big: false,
            },
        });

        // Post big blind (capped at stack).
        let bb_amount = config.big_blind.min(stacks[bb_idx]);
        stacks[bb_idx] -= bb_amount;
        contributed[bb_idx] += bb_amount;
        round_bet[bb_idx] += bb_amount;
        if stacks[bb_idx] == 0 {
            all_in[bb_idx] = true;
        }
        log.push(LogEntry {
            actor: Some(PlayerId(bb_idx)),
            kind: LogKind::PostBlind {
                amount: bb_amount,
                is_big: true,
            },
        });

        let acted_this_round = vec![false; n];

        HandState {
            config,
            phase: Phase::Preflop,
            board: Vec::new(),
            hole,
            stacks,
            contributed,
            round_bet,
            folded,
            all_in,
            acted_this_round,
            to_act: Some(PlayerId(first_to_act)),
            current_bet: bb_amount,
            min_raise: config.big_blind,
            last_aggressor: Some(PlayerId(bb_idx)), // BB is the "aggressor" until someone raises
            log,
            pots: Vec::new(),
            winners: Vec::new(),
            deck,
        }
    }

    pub(crate) fn all_have_acted_this_round(&self, actionable: &[usize]) -> bool {
        actionable.iter().all(|&i| self.acted_this_round[i])
    }

    /// Number of seats that still have chips. The game is over once this drops
    /// below 2 — only one player can fund a hand.
    pub fn funded_seats(&self) -> usize {
        self.stacks.iter().filter(|&&s| s > 0).count()
    }

    /// Deal the next hand: carry every seat's stack forward, move the dealer
    /// button to the next funded seat, and shuffle with `seed`. Seats that ran
    /// out of chips sit out (start folded). Requires at least 2 funded seats.
    pub fn next_hand(&self, seed: u64) -> Self {
        let n = self.config.num_players;
        // Advance the button to the next funded seat clockwise.
        let mut dealer = (self.config.dealer.0 + 1) % n;
        while self.stacks[dealer] == 0 {
            dealer = (dealer + 1) % n;
        }
        let config = HandConfig {
            dealer: PlayerId(dealer),
            seed,
            ..self.config
        };
        HandState::new_hand(config, self.stacks.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdem::types::{HandConfig, PlayerId};

    fn cfg(n: usize) -> HandConfig {
        HandConfig {
            num_players: n,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 0xC0FFEE,
        }
    }

    #[test]
    fn six_handed_new_hand_posts_blinds_and_sets_to_act() {
        let s = HandState::new_hand(cfg(6), vec![10_000; 6]);
        assert_eq!(s.phase, Phase::Preflop);
        assert_eq!(s.stacks[1], 9_950, "SB posts 50");
        assert_eq!(s.stacks[2], 9_900, "BB posts 100");
        assert_eq!(s.contributed[1], 50);
        assert_eq!(s.contributed[2], 100);
        assert_eq!(s.current_bet, 100);
        assert_eq!(s.min_raise, 100);
        assert_eq!(s.to_act, Some(PlayerId(3)), "UTG acts first preflop");
        assert_eq!(s.hole.len(), 6);
        assert_eq!(s.board.len(), 0);
    }

    #[test]
    fn heads_up_dealer_is_sb_and_acts_first() {
        let s = HandState::new_hand(cfg(2), vec![10_000; 2]);
        assert_eq!(s.stacks[0], 9_950, "dealer/SB posts 50 in HU");
        assert_eq!(s.stacks[1], 9_900, "BB posts 100");
        assert_eq!(
            s.to_act,
            Some(PlayerId(0)),
            "dealer/SB acts first preflop in HU"
        );
    }

    #[test]
    fn same_seed_deals_identical_hole_cards() {
        let a = HandState::new_hand(cfg(6), vec![10_000; 6]);
        let b = HandState::new_hand(cfg(6), vec![10_000; 6]);
        assert_eq!(a.hole, b.hole);
    }

    #[test]
    fn hole_cards_are_all_distinct() {
        let s = HandState::new_hand(cfg(9), vec![10_000; 9]);
        let seen: Vec<Card> = s.hole.iter().flat_map(|h| h.iter().copied()).collect();
        let unique = seen.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), seen.len(), "no duplicate hole cards");
    }

    #[test]
    fn short_stacked_blind_goes_all_in_on_post() {
        let mut stacks = vec![10_000u64; 6];
        stacks[1] = 30; // SB has only 30 chips
        let s = HandState::new_hand(cfg(6), stacks);
        assert_eq!(s.contributed[1], 30, "SB caps at stack");
        assert!(s.all_in[1]);
    }

    #[test]
    fn busted_seat_sits_out_folded_and_posts_nothing() {
        // Dealer 0; seat 1 (the usual SB) is busted. Blinds must skip it.
        let mut stacks = vec![10_000u64; 6];
        stacks[1] = 0;
        let s = HandState::new_hand(cfg(6), stacks);
        assert!(s.folded[1], "busted seat starts folded");
        assert!(!s.all_in[1], "busted seat is not all-in");
        assert_eq!(s.contributed[1], 0, "busted seat posts no blind");
        assert_eq!(s.stacks[1], 0);
        // SB rolls to the next funded seat (2), BB to (3), UTG to (4).
        assert_eq!(s.contributed[2], 50, "SB skips busted seat 1");
        assert_eq!(s.contributed[3], 100, "BB skips busted seat 1");
        assert_eq!(s.to_act, Some(PlayerId(4)), "UTG is first funded after BB");
        assert_ne!(s.to_act, Some(PlayerId(1)), "busted seat never acts");
    }

    #[test]
    fn two_funded_seats_play_heads_up_rules() {
        // Only seats 0 and 3 are funded → heads-up: dealer (0) is SB and acts
        // first preflop, the other funded seat (3) is BB.
        let stacks = vec![10_000, 0, 0, 10_000, 0, 0];
        let s = HandState::new_hand(cfg(6), stacks);
        assert_eq!(s.contributed[0], 50, "dealer is SB heads-up");
        assert_eq!(s.contributed[3], 100, "other funded seat is BB");
        assert_eq!(s.to_act, Some(PlayerId(0)), "dealer/SB acts first heads-up");
        for &busted in &[1usize, 2, 4, 5] {
            assert!(s.folded[busted], "seat {busted} busted and folded");
        }
    }

    #[test]
    #[should_panic(expected = "at least 2 funded")]
    fn fewer_than_two_funded_seats_panics() {
        let stacks = vec![0, 0, 10_000, 0, 0, 0];
        let _ = HandState::new_hand(cfg(6), stacks);
    }

    #[test]
    fn funded_seats_counts_nonzero_stacks() {
        let stacks = vec![10_000, 0, 500, 0, 0, 200];
        let s = HandState::new_hand(cfg(6), stacks);
        assert_eq!(s.funded_seats(), 3);
    }

    #[test]
    fn next_hand_carries_stacks_and_rotates_dealer() {
        let s = HandState::new_hand(cfg(6), vec![10_000; 6]);
        // Pretend the hand ended with uneven stacks.
        let mut ended = s;
        ended.stacks = vec![5_000, 12_000, 9_000, 11_000, 8_000, 15_000];
        let next = ended.next_hand(0xABCDEF);
        assert_eq!(next.config.dealer, PlayerId(1), "button moves one seat");
        // Stacks carry over, minus the blinds the new SB/BB just posted.
        // New dealer 1 → SB = 2 (9_000-50), BB = 3 (11_000-100).
        assert_eq!(next.stacks[0], 5_000);
        assert_eq!(next.stacks[2], 8_950, "new SB posted 50");
        assert_eq!(next.stacks[3], 10_900, "new BB posted 100");
        assert_eq!(next.config.seed, 0xABCDEF);
        assert_eq!(next.phase, Phase::Preflop);
    }

    #[test]
    fn next_hand_skips_busted_seats_for_the_button() {
        let s = HandState::new_hand(cfg(6), vec![10_000; 6]);
        let mut ended = s;
        // Seats 1 and 2 are busted; button is currently 0, so it must jump to 3.
        ended.stacks = vec![5_000, 0, 0, 11_000, 8_000, 15_000];
        let next = ended.next_hand(7);
        assert_eq!(next.config.dealer, PlayerId(3), "button skips busted seats");
        assert!(next.folded[1] && next.folded[2], "busted seats sit out");
    }
}
