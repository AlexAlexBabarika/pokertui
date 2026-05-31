use crate::Card;
use crate::Deck;
use super::types::{HandConfig, Phase, PlayerId};
use super::pots::Pot;

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
        assert_eq!(stacks.len(), config.num_players, "stacks must match num_players");
        assert!(config.big_blind > 0, "big blind must be positive");
        assert!(config.small_blind > 0 && config.small_blind < config.big_blind, "SB in (0, BB)");
        assert!(config.dealer.0 < config.num_players, "dealer index in range");

        let n = config.num_players;
        let mut deck = Deck::new();
        deck.shuffle_with_seed(config.seed);
        let mut deck: Vec<Card> = deck.cards().to_vec();

        // Deal 2 hole cards per player, alternating (real poker dealing order).
        let mut hole: Vec<[Card; 2]> = Vec::with_capacity(n);
        for _ in 0..n {
            hole.push([deck.remove(0), Card::new(crate::Rank::Two, crate::Suit::Clubs)]);
        }
        for p in 0..n {
            hole[p][1] = deck.remove(0);
        }

        // Blind positions.
        // Heads-up: dealer is SB, the other is BB, dealer acts first preflop.
        // 3+: SB = dealer+1, BB = dealer+2, UTG = dealer+3.
        let (sb_idx, bb_idx, first_to_act) = if n == 2 {
            (config.dealer.0, (config.dealer.0 + 1) % n, config.dealer.0)
        } else {
            let sb = (config.dealer.0 + 1) % n;
            let bb = (config.dealer.0 + 2) % n;
            let utg = (config.dealer.0 + 3) % n;
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
        if stacks[sb_idx] == 0 { all_in[sb_idx] = true; }
        log.push(LogEntry {
            actor: Some(PlayerId(sb_idx)),
            kind: LogKind::PostBlind { amount: sb_amount, is_big: false },
        });

        // Post big blind (capped at stack).
        let bb_amount = config.big_blind.min(stacks[bb_idx]);
        stacks[bb_idx] -= bb_amount;
        contributed[bb_idx] += bb_amount;
        round_bet[bb_idx] += bb_amount;
        if stacks[bb_idx] == 0 { all_in[bb_idx] = true; }
        log.push(LogEntry {
            actor: Some(PlayerId(bb_idx)),
            kind: LogKind::PostBlind { amount: bb_amount, is_big: true },
        });

        HandState {
            config,
            phase: Phase::Preflop,
            board: Vec::new(),
            hole,
            stacks,
            contributed,
            round_bet,
            folded: vec![false; n],
            all_in,
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
        assert_eq!(s.to_act, Some(PlayerId(0)), "dealer/SB acts first preflop in HU");
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
}
