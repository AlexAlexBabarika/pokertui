use poker_core::Card;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatStatus {
    Hero,
    Active,
    Bet,
    Folded,
}

#[derive(Debug, Clone)]
pub struct Seat {
    pub name: String,
    pub pos: String,
    pub stack: u64,
    pub status: SeatStatus,
    pub last_action: String,
    pub hole_cards: Option<[Card; 2]>,
    /// True when it is this seat's turn to act.
    pub is_to_act: bool,
}

impl Seat {
    pub fn is_hero(&self) -> bool {
        self.status == SeatStatus::Hero
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum LogTone {
    Dim,
    Muted,
    Fg,
    Amber,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub who: String,
    pub what: String,
    pub tone: LogTone,
}

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub who: String,
    pub msg: String,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub label: String,
    pub board: Vec<Card>,
    pub dealt: usize,
    pub pot: u64,
    pub to_call: u64,
    pub equity: u8,    // 0..=100
    pub odds_pct: f32, // e.g. 24.5
    pub rank: String,
    pub hint: String,
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub blinds: String,
    pub players: Vec<Seat>,
    pub phase: Phase,
    pub log: Vec<LogEntry>,
    pub chat: Vec<ChatLine>,
}

impl GameState {
    pub fn hero(&self) -> Option<&Seat> {
        self.players.iter().find(|p| p.is_hero())
    }

    #[cfg(test)]
    pub fn demo() -> Self {
        use poker_core::{Rank::*, Suit::*};

        let card = Card::new;

        Self {
            blinds: "50 / 100".into(),
            players: vec![
                Seat {
                    name: "nova".into(),
                    pos: "UTG".into(),
                    stack: 5_300,
                    status: SeatStatus::Folded,
                    last_action: "fold".into(),
                    hole_cards: None,
                    is_to_act: false,
                },
                Seat {
                    name: "delta".into(),
                    pos: "MP".into(),
                    stack: 14_700,
                    status: SeatStatus::Folded,
                    last_action: "fold".into(),
                    hole_cards: None,
                    is_to_act: false,
                },
                Seat {
                    name: "gizmo".into(),
                    pos: "CO".into(),
                    stack: 9_050,
                    status: SeatStatus::Active,
                    last_action: "call 600".into(),
                    hole_cards: None,
                    is_to_act: false,
                },
                Seat {
                    name: "you".into(),
                    pos: "BTN".into(),
                    stack: 12_450,
                    status: SeatStatus::Hero,
                    last_action: "—".into(),
                    hole_cards: Some([card(Ace, Spades), card(King, Hearts)]),
                    is_to_act: true,
                },
                Seat {
                    name: "maple".into(),
                    pos: "SB".into(),
                    stack: 8_900,
                    status: SeatStatus::Folded,
                    last_action: "fold".into(),
                    hole_cards: None,
                    is_to_act: false,
                },
                Seat {
                    name: "rook".into(),
                    pos: "BB".into(),
                    stack: 22_100,
                    status: SeatStatus::Bet,
                    last_action: "bet 600".into(),
                    hole_cards: Some([card(King, Clubs), card(Queen, Diamonds)]),
                    is_to_act: false,
                },
            ],
            phase: Phase {
                label: "TURN".into(),
                board: vec![
                    card(Queen, Clubs),
                    card(Jack, Diamonds),
                    card(Four, Hearts),
                    card(Nine, Spades),
                ],
                dealt: 4,
                pot: 1_850,
                to_call: 600,
                equity: 38,
                odds_pct: 24.5,
                rank: "A-high · open-ender (needs T)".into(),
                hint: "odds 24.5% < eq 38% — call is +EV".into(),
            },
            log: vec![
                LogEntry {
                    who: "rook".into(),
                    what: "posts BB 100".into(),
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "you".into(),
                    what: "raise 300".into(),
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo".into(),
                    what: "call 300".into(),
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "rook".into(),
                    what: "call 300".into(),
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "·".into(),
                    what: "flop  Q♣ J♦ 4♥".into(),
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "rook".into(),
                    what: "check".into(),
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "you".into(),
                    what: "bet 300".into(),
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo".into(),
                    what: "call 300".into(),
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "rook".into(),
                    what: "call 300".into(),
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "·".into(),
                    what: "turn  9♠".into(),
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "rook".into(),
                    what: "bet 600".into(),
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo".into(),
                    what: "call 600".into(),
                    tone: LogTone::Fg,
                },
            ],
            chat: vec![
                ChatLine {
                    who: "rook".into(),
                    msg: "nice spot".into(),
                },
                ChatLine {
                    who: "gizmo".into(),
                    msg: "i hate this turn".into(),
                },
                ChatLine {
                    who: "you".into(),
                    msg: "tank-calling…".into(),
                },
            ],
        }
    }
}
