//! Pure poker rules kernel — no UI, no async, no I/O.

use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Card {
    rank: Rank,
    suit: Suit,
}

impl Card {
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }

    pub const fn rank(self) -> Rank {
        self.rank
    }

    pub const fn suit(self) -> Suit {
        self.suit
    }

    /// Parse a two-character card notation like "As", "Td", "2c".
    /// Panics on invalid input
    pub fn parse(s: &str) -> Card {
        let mut chars = s.chars();
        let rank_c = chars.next().expect("rank char");
        let suit_c = chars.next().expect("suit char");
        let rank = match rank_c {
            '2' => Rank::Two,
            '3' => Rank::Three,
            '4' => Rank::Four,
            '5' => Rank::Five,
            '6' => Rank::Six,
            '7' => Rank::Seven,
            '8' => Rank::Eight,
            '9' => Rank::Nine,
            'T' => Rank::Ten,
            'J' => Rank::Jack,
            'Q' => Rank::Queen,
            'K' => Rank::King,
            'A' => Rank::Ace,
            other => panic!("invalid rank char {other:?}"),
        };
        let suit = match suit_c {
            'c' => Suit::Clubs,
            'd' => Suit::Diamonds,
            'h' => Suit::Hearts,
            's' => Suit::Spades,
            other => panic!("invalid suit char {other:?}"),
        };
        Card::new(rank, suit)
    }
}

#[derive(Debug, Clone)]
pub struct Deck {
    cards: Vec<Card>,
}

impl Deck {
    pub fn new() -> Self {
        let mut cards = Vec::with_capacity(52);
        for &suit in &Suit::ALL {
            for &rank in &Rank::ALL {
                cards.push(Card::new(rank, suit));
            }
        }
        Self { cards }
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn remaining(&self) -> usize {
        self.cards.len()
    }

    pub fn shuffle_with_seed(&mut self, seed: u64) {
        let mut rng = StdRng::seed_from_u64(seed);
        self.cards.shuffle(&mut rng);
    }

    pub fn deal(&mut self) -> Option<Card> {
        if self.cards.is_empty() {
            None
        } else {
            Some(self.cards.remove(0))
        }
    }
}

impl Default for Deck {
    fn default() -> Self {
        Self::new()
    }
}

/// Strength of a poker hand. Compare with `<`, `>`, `==`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandStrength(rs_poker::core::Rank);

/// Evaluate the best 5-card hand strength from 5–7 cards.
pub fn evaluate(cards: &[Card]) -> HandStrength {
    use rs_poker::core::Rankable;
    let converted: Vec<rs_poker::core::Card> = cards.iter().copied().map(to_rs_card).collect();
    HandStrength(converted.rank())
}

fn to_rs_card(c: Card) -> rs_poker::core::Card {
    let value = match c.rank() {
        Rank::Two => rs_poker::core::Value::Two,
        Rank::Three => rs_poker::core::Value::Three,
        Rank::Four => rs_poker::core::Value::Four,
        Rank::Five => rs_poker::core::Value::Five,
        Rank::Six => rs_poker::core::Value::Six,
        Rank::Seven => rs_poker::core::Value::Seven,
        Rank::Eight => rs_poker::core::Value::Eight,
        Rank::Nine => rs_poker::core::Value::Nine,
        Rank::Ten => rs_poker::core::Value::Ten,
        Rank::Jack => rs_poker::core::Value::Jack,
        Rank::Queen => rs_poker::core::Value::Queen,
        Rank::King => rs_poker::core::Value::King,
        Rank::Ace => rs_poker::core::Value::Ace,
    };
    let suit = match c.suit() {
        Suit::Clubs => rs_poker::core::Suit::Club,
        Suit::Diamonds => rs_poker::core::Suit::Diamond,
        Suit::Hearts => rs_poker::core::Suit::Heart,
        Suit::Spades => rs_poker::core::Suit::Spade,
    };
    rs_poker::core::Card { value, suit }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_ace_is_highest_two_is_lowest() {
        assert!(Rank::Ace > Rank::King);
        assert!(Rank::Two < Rank::Three);
        assert_eq!(Rank::ALL.len(), 13);
        assert_eq!(Rank::ALL.first().copied(), Some(Rank::Two));
        assert_eq!(Rank::ALL.last().copied(), Some(Rank::Ace));
    }

    #[test]
    fn there_are_four_suits() {
        assert_eq!(Suit::ALL.len(), 4);
        let mut set: Vec<Suit> = Suit::ALL.to_vec();
        set.sort();
        set.dedup();
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn card_exposes_rank_and_suit() {
        let c = Card::new(Rank::Ace, Suit::Spades);
        assert_eq!(c.rank(), Rank::Ace);
        assert_eq!(c.suit(), Suit::Spades);
    }

    #[test]
    fn fresh_deck_has_52_unique_cards() {
        let deck = Deck::new();
        assert_eq!(deck.remaining(), 52);
        let mut cards: Vec<Card> = deck.cards().to_vec();
        cards.sort_by_key(|c| (c.suit() as u8, c.rank() as u8));
        cards.dedup();
        assert_eq!(cards.len(), 52);
    }

    #[test]
    fn same_seed_produces_same_shuffle() {
        let mut a = Deck::new();
        let mut b = Deck::new();
        a.shuffle_with_seed(0xDEADBEEF);
        b.shuffle_with_seed(0xDEADBEEF);
        assert_eq!(a.cards(), b.cards());
    }

    #[test]
    fn different_seeds_produce_different_shuffles() {
        let mut a = Deck::new();
        let mut b = Deck::new();
        a.shuffle_with_seed(1);
        b.shuffle_with_seed(2);
        assert_ne!(a.cards(), b.cards());
    }

    #[test]
    fn deal_removes_card_from_top() {
        let mut deck = Deck::new();
        let top = deck.cards()[0];
        let drawn = deck.deal().expect("non-empty deck");
        assert_eq!(drawn, top);
        assert_eq!(deck.remaining(), 51);
    }

    fn h(s: &str) -> Vec<Card> {
        s.split_whitespace().map(Card::parse).collect()
    }

    #[test]
    fn parse_round_trips_card_notation() {
        assert_eq!(Card::parse("As"), Card::new(Rank::Ace, Suit::Spades));
        assert_eq!(Card::parse("Td"), Card::new(Rank::Ten, Suit::Diamonds));
        assert_eq!(Card::parse("2c"), Card::new(Rank::Two, Suit::Clubs));
    }

    #[test]
    fn royal_flush_beats_straight_flush() {
        let royal = evaluate(&h("As Ks Qs Js Ts 2c 3d"));
        let straight_flush = evaluate(&h("9h 8h 7h 6h 5h 2c 3d"));
        assert!(royal > straight_flush);
    }

    #[test]
    fn straight_flush_beats_quads() {
        let sf = evaluate(&h("9h 8h 7h 6h 5h 2c 3d"));
        let quads = evaluate(&h("Ah Ad Ac As Kd 2c 3d"));
        assert!(sf > quads);
    }

    #[test]
    fn quads_beats_full_house() {
        let quads = evaluate(&h("2h 2d 2c 2s Kd 3c 4d"));
        let boat = evaluate(&h("Ah Ad Ac Ks Kd 3c 4d"));
        assert!(quads > boat);
    }

    #[test]
    fn full_house_beats_flush() {
        let boat = evaluate(&h("2h 2d 2c Ks Kd 3c 4d"));
        let flush = evaluate(&h("Ah Kh 9h 7h 5h 2c 3d"));
        assert!(boat > flush);
    }

    #[test]
    fn flush_beats_straight() {
        let flush = evaluate(&h("Ah Kh 9h 7h 5h 2c 3d"));
        let straight = evaluate(&h("9h 8d 7c 6s 5h 2c 3d"));
        assert!(flush > straight);
    }

    #[test]
    fn straight_beats_trips() {
        let straight = evaluate(&h("9h 8d 7c 6s 5h 2c 3d"));
        let trips = evaluate(&h("Ah Ad Ac Ks Qd 3c 4d"));
        assert!(straight > trips);
    }

    #[test]
    fn trips_beats_two_pair() {
        let trips = evaluate(&h("2h 2d 2c Ks Qd 3c 4d"));
        let two_pair = evaluate(&h("Ah Ad Ks Kd Qc 3c 4d"));
        assert!(trips > two_pair);
    }

    #[test]
    fn two_pair_beats_one_pair() {
        let two_pair = evaluate(&h("2h 2d 3s 3d Qc 4d 5c"));
        let one_pair = evaluate(&h("Ah Ad Ks Qd Jc 4d 5c"));
        assert!(two_pair > one_pair);
    }

    #[test]
    fn pair_beats_high_card() {
        let pair = evaluate(&h("2h 2d 5s 7d 9c Jd Qs"));
        let high = evaluate(&h("Ah Kd Qs Jd 9c 7d 5s"));
        assert!(pair > high);
    }

    #[test]
    fn higher_straight_beats_lower_straight() {
        let big = evaluate(&h("Ah Kd Qs Jc Th 2c 3d"));
        let small = evaluate(&h("6h 5d 4s 3c 2h 8c 9d"));
        assert!(big > small);
    }

    #[test]
    fn wheel_is_lowest_straight() {
        let wheel = evaluate(&h("Ah 2d 3c 4s 5h 8c 9d"));
        let six_high = evaluate(&h("6h 5d 4c 3s 2h 9c Qd"));
        assert!(six_high > wheel);
    }
}
