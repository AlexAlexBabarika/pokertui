use poker_core::{Card, Rank, Suit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatStatus {
    Hero,
    Active,
    Bet,
    Folded,
}

#[derive(Debug, Clone)]
pub struct Seat {
    pub name: &'static str,
    pub pos: &'static str,
    pub stack: u64,
    pub status: SeatStatus,
    pub last_action: &'static str,
    pub hole_cards: Option<[Card; 2]>,
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
    pub who: &'static str,
    pub what: &'static str,
    pub tone: LogTone,
}

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub who: &'static str,
    pub msg: &'static str,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub label: &'static str,
    pub board: Vec<Card>,
    pub dealt: usize,
    pub pot: u64,
    pub to_call: u64,
    pub equity: u8,    // 0..=100
    pub odds_pct: f32, // e.g. 24.5
    pub rank: &'static str,
    pub hint: &'static str,
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub blinds: &'static str,
    pub players: Vec<Seat>,
    pub phase: Phase,
    pub log: Vec<LogEntry>,
    pub chat: Vec<ChatLine>,
}

impl GameState {
    pub fn hero(&self) -> Option<&Seat> {
        self.players.iter().find(|p| p.is_hero())
    }

    pub fn by_name(&self, name: &str) -> Option<&Seat> {
        self.players.iter().find(|p| p.name == name)
    }

    pub fn demo() -> Self {
        use Rank::*;
        use Suit::*;

        let card = Card::new;

        Self {
            blinds: "50 / 100",
            players: vec![
                Seat {
                    name: "nova",
                    pos: "UTG",
                    stack: 5_300,
                    status: SeatStatus::Folded,
                    last_action: "fold",
                    hole_cards: None,
                },
                Seat {
                    name: "delta",
                    pos: "MP",
                    stack: 14_700,
                    status: SeatStatus::Folded,
                    last_action: "fold",
                    hole_cards: None,
                },
                Seat {
                    name: "gizmo",
                    pos: "CO",
                    stack: 9_050,
                    status: SeatStatus::Active,
                    last_action: "call 600",
                    hole_cards: None,
                },
                Seat {
                    name: "you",
                    pos: "BTN",
                    stack: 12_450,
                    status: SeatStatus::Hero,
                    last_action: "—",
                    hole_cards: Some([card(Ace, Spades), card(King, Hearts)]),
                },
                Seat {
                    name: "maple",
                    pos: "SB",
                    stack: 8_900,
                    status: SeatStatus::Folded,
                    last_action: "fold",
                    hole_cards: None,
                },
                Seat {
                    name: "rook",
                    pos: "BB",
                    stack: 22_100,
                    status: SeatStatus::Bet,
                    last_action: "bet 600",
                    hole_cards: Some([card(King, Clubs), card(Queen, Diamonds)]),
                },
            ],
            phase: Phase {
                label: "TURN",
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
                rank: "A-high · open-ender (needs T)",
                hint: "odds 24.5% < eq 38% — call is +EV",
            },
            log: vec![
                LogEntry {
                    who: "rook",
                    what: "posts BB 100",
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "you",
                    what: "raise 300",
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo",
                    what: "call 300",
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "rook",
                    what: "call 300",
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "·",
                    what: "flop  Q♣ J♦ 4♥",
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "rook",
                    what: "check",
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "you",
                    what: "bet 300",
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo",
                    what: "call 300",
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "rook",
                    what: "call 300",
                    tone: LogTone::Fg,
                },
                LogEntry {
                    who: "·",
                    what: "turn  9♠",
                    tone: LogTone::Dim,
                },
                LogEntry {
                    who: "rook",
                    what: "bet 600",
                    tone: LogTone::Amber,
                },
                LogEntry {
                    who: "gizmo",
                    what: "call 600",
                    tone: LogTone::Fg,
                },
            ],
            chat: vec![
                ChatLine {
                    who: "rook",
                    msg: "nice spot",
                },
                ChatLine {
                    who: "gizmo",
                    msg: "i hate this turn",
                },
                ChatLine {
                    who: "you",
                    msg: "tank-calling…",
                },
            ],
        }
    }
}
