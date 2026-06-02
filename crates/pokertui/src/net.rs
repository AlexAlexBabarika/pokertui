use std::sync::mpsc::{Receiver, TryRecvError};

use crossterm::event::KeyCode;
use poker_core::Card;
use poker_core::equity::hero_equity;
use poker_core::holdem::{Action, Phase, PlayerId};
use poker_net::msg::{ClientMsg, ServerMsg};
use poker_net::state::PublicState;
use poker_net::{read_msg, write_msg};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;

use crate::net_adapter::{format_action, to_presentation_net};
use crate::state::{ChatLine, GameState, LogEntry, LogTone};
use crate::table::Table;

/// Monte Carlo trials per equity estimate (matches the local client).
const EQUITY_ITERS: u32 = 10_000;

pub struct NetClient {
    /// Frames arriving from the server, pushed by the background net thread.
    incoming: Receiver<ServerMsg>,
    /// Actions/chat headed to the server. `send` is non-blocking and usable from
    /// this synchronous context.
    to_net: UnboundedSender<ClientMsg>,

    your_seat: Option<PlayerId>,
    latest: Option<PublicState>,
    /// The legal actions for *our* current turn, or `None` when it is not our
    /// turn. Set by `YourTurn`, cleared when we act or the state moves on.
    legal: Option<Vec<Action>>,
    deadline_ms: u64,

    feed: Vec<LogEntry>,
    chat: Vec<ChatLine>,
    /// Last action seen per seat, to diff successive states into feed lines.
    last_seen: Vec<Option<Action>>,
    prev_board_len: usize,

    raise_to: Option<u64>,
    notice: Option<String>,

    /// Cached equity: recomputed only when (hole, board, opponents) changes.
    equity_cache: Option<(Vec<Card>, Vec<Card>, usize, u8)>,
}

impl NetClient {
    /// Connect to `addr`, send `Join`, and spawn the background net thread.
    pub fn connect(addr: &str, name: &str) -> Self {
        let (in_tx, in_rx) = std::sync::mpsc::channel::<ServerMsg>();
        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<ClientMsg>();
        let addr = addr.to_string();
        let name = name.to_string();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let stream = match TcpStream::connect(&addr).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = in_tx.send(ServerMsg::Error(format!("connect failed: {e}")));
                        return;
                    }
                };
                let (mut rd, mut wr) = stream.into_split();
                if write_msg(&mut wr, &ClientMsg::Join { name }).await.is_err() {
                    let _ = in_tx.send(ServerMsg::Error("failed to join".into()));
                    return;
                }
                // Writer: drain UI-originated messages to the socket.
                let writer = tokio::spawn(async move {
                    while let Some(m) = out_rx.recv().await {
                        if write_msg(&mut wr, &m).await.is_err() {
                            break;
                        }
                    }
                });
                // Reader: forward server frames to the UI until the link drops.
                loop {
                    match read_msg::<_, ServerMsg>(&mut rd).await {
                        Ok(m) => {
                            if in_tx.send(m).is_err() {
                                break; // UI gone
                            }
                        }
                        Err(_) => {
                            let _ = in_tx.send(ServerMsg::Error("disconnected".into()));
                            break;
                        }
                    }
                }
                writer.abort();
            });
        });

        NetClient {
            incoming: in_rx,
            to_net: out_tx,
            your_seat: None,
            latest: None,
            legal: None,
            deadline_ms: 0,
            feed: Vec::new(),
            chat: Vec::new(),
            last_seen: Vec::new(),
            prev_board_len: 0,
            raise_to: None,
            notice: Some("connecting…".into()),
            equity_cache: None,
        }
    }

    /// Fold one server message into client state. Pure with respect to the
    /// socket — the render loop calls this from `step`, tests call it directly.
    pub fn ingest(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome { your_seat } => {
                self.your_seat = Some(your_seat);
            }
            ServerMsg::StateUpdate(state) => {
                self.record_feed(&state);
                self.legal = None; // a fresh state supersedes any pending turn
                self.latest = Some(state);
                self.notice = self.derive_notice();
            }
            ServerMsg::YourTurn { legal, deadline_ms } => {
                self.legal = Some(legal);
                self.deadline_ms = deadline_ms;
                self.raise_to = None;
            }
            ServerMsg::Chat { who, msg } => {
                self.chat.push(ChatLine { who, msg });
            }
            ServerMsg::Error(e) => {
                self.notice = Some(e);
            }
        }
    }

    /// Append feed lines for any seat whose last action changed, and for new
    /// board cards, by diffing the incoming state against the previous one.
    fn record_feed(&mut self, state: &PublicState) {
        if self.last_seen.len() != state.num_players {
            self.last_seen = vec![None; state.num_players];
        }
        for i in 0..state.num_players {
            let act = state.seats[i].last_action;
            if let Some(a) = act
                && act != self.last_seen[i]
            {
                self.feed.push(LogEntry {
                    who: state.seats[i].name.clone(),
                    what: format_action(a),
                    tone: action_tone(a),
                });
                self.last_seen[i] = act;
            }
        }
        // New board cards → a street marker.
        if state.board.len() > self.prev_board_len {
            let label = match state.board.len() {
                3 => "flop",
                4 => "turn",
                5 => "river",
                _ => "board",
            };
            self.feed.push(LogEntry {
                who: "·".into(),
                what: label.into(),
                tone: LogTone::Dim,
            });
            // A new street also clears stale per-seat action markers.
            self.last_seen = vec![None; state.num_players];
        }
        self.prev_board_len = state.board.len();
    }

    /// The banner shown over the table, from the latest state.
    fn derive_notice(&self) -> Option<String> {
        let state = self.latest.as_ref()?;
        if state.phase == Phase::Complete {
            return Some("hand complete · next hand starting…".into());
        }
        None
    }

    /// Hero equity for the current state, cached on (hole, board, opponents).
    fn equity_pct(&mut self) -> u8 {
        let Some(state) = &self.latest else {
            return 0;
        };
        let hero = match self.your_seat {
            Some(h) => h,
            None => return 0,
        };
        let Some(hole) = state.seats[hero.0].hole else {
            return 0;
        };
        if state.phase == Phase::Complete || state.seats[hero.0].folded {
            return 0;
        }
        let opponents = (0..state.num_players)
            .filter(|&i| PlayerId(i) != hero && !state.seats[i].folded)
            .count();
        let board = state.board.clone();
        let hole_v = hole.to_vec();
        if let Some((ch, cb, co, pct)) = &self.equity_cache
            && *ch == hole_v
            && *cb == board
            && *co == opponents
        {
            return *pct;
        }
        // Deterministic seed from the situation, like the local client.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for c in hole_v.iter().chain(board.iter()) {
            seed = seed
                .wrapping_mul(31)
                .wrapping_add(c.rank() as u64 * 4 + c.suit() as u64 + 1);
        }
        let pct = hero_equity(hole, &board, opponents, EQUITY_ITERS, seed)
            .pct()
            .round() as u8;
        self.equity_cache = Some((hole_v, board, opponents, pct));
        pct
    }

    /// `(min, max)` raise/bet to-level for our seat in the current state, or
    /// `None` if no full raise/bet is possible.
    fn raise_bounds(&self) -> Option<(u64, u64)> {
        let state = self.latest.as_ref()?;
        let hero = self.your_seat?;
        let seat = &state.seats[hero.0];
        let min = if state.current_bet == 0 {
            state.big_blind
        } else {
            state.current_bet + state.min_raise
        };
        let max = seat.round_bet + seat.stack;
        (max >= min).then_some((min, max))
    }
}

impl Table for NetClient {
    fn view(&mut self) -> GameState {
        let equity = self.equity_pct();
        let raise_to = self
            .raise_bounds()
            .map(|(min, max)| self.raise_to.unwrap_or(min).clamp(min, max));
        let notice = self.notice.clone();

        match &self.latest {
            Some(state) => {
                let mut gs = to_presentation_net(state, &self.feed, &self.chat);
                gs.phase.equity = equity;
                gs.phase.raise_to = raise_to;
                gs.notice = notice;
                gs
            }
            None => GameState {
                blinds: "— / —".into(),
                players: Vec::new(),
                phase: crate::state::Phase {
                    label: "LOBBY".into(),
                    board: Vec::new(),
                    dealt: 0,
                    pot: 0,
                    to_call: 0,
                    raise_to: None,
                    equity: 0,
                    odds_pct: 0.0,
                    rank: "—".into(),
                },
                log: self.feed.clone(),
                chat: self.chat.clone(),
                notice: notice.or_else(|| Some("waiting for players…".into())),
                show_win_rate: true,
            },
        }
    }

    fn handle_key(&mut self, key: KeyCode) -> bool {
        // Only act when the server has told us it is our turn.
        let Some(legal) = self.legal.clone() else {
            return false;
        };

        // Bet-size selection (UI-only) while it is our turn.
        if matches!(key, KeyCode::Up | KeyCode::Down) {
            if let Some((min, max)) = self.raise_bounds() {
                let step = self.latest.as_ref().map(|s| s.small_blind).unwrap_or(1);
                let cur = self.raise_to.unwrap_or(min);
                self.raise_to = Some(if matches!(key, KeyCode::Up) {
                    cur.saturating_add(step).min(max)
                } else {
                    cur.saturating_sub(step).max(min)
                });
            }
            return false;
        }

        let action = match key {
            KeyCode::Char('f') | KeyCode::Char('F') => Some(Action::Fold),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if legal.contains(&Action::Check) {
                    Some(Action::Check)
                } else {
                    Some(Action::Call)
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => self.raise_bounds().map(|(min, max)| {
                let to = self.raise_to.unwrap_or(min).clamp(min, max);
                if self.latest.as_ref().map(|s| s.current_bet).unwrap_or(0) == 0 {
                    Action::Bet { to }
                } else {
                    Action::Raise { to }
                }
            }),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::AllIn),
            _ => None,
        };

        let Some(action) = action else {
            return false;
        };
        // Send to the server and optimistically clear our turn; the next
        // StateUpdate/YourTurn is authoritative.
        let _ = self.to_net.send(ClientMsg::Action(action));
        self.legal = None;
        self.raise_to = None;
        true
    }

    fn step(&mut self) {
        // Drain everything the net thread has delivered since the last tick.
        loop {
            match self.incoming.try_recv() {
                Ok(msg) => self.ingest(msg),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.notice = Some("connection closed".into());
                    break;
                }
            }
        }
    }
}

/// Feed tone for an action (mirrors the local adapter's `tone_for`).
fn action_tone(a: Action) -> LogTone {
    match a {
        Action::Bet { .. } | Action::Raise { .. } | Action::AllIn => LogTone::Amber,
        _ => LogTone::Fg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poker_core::holdem::{HandConfig, HandState, apply};

    fn names() -> Vec<String> {
        ["nova", "delta", "gizmo", "you", "maple", "rook"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// A NetClient with no live socket — `connect` spawns a thread that will fail
    /// to reach a dead address, which is fine: these tests drive `ingest`
    /// directly and never read `incoming`.
    fn detached() -> NetClient {
        NetClient::connect("127.0.0.1:1", "you")
    }

    fn state_for(seat: usize, build: impl Fn(HandState) -> HandState) -> PublicState {
        let cfg = HandConfig {
            num_players: 6,
            small_blind: 50,
            big_blind: 100,
            dealer: PlayerId(0),
            seed: 1,
        };
        let engine = build(HandState::new_hand(cfg, vec![10_000; 6]));
        PublicState::for_recipient(&engine, &names(), PlayerId(seat))
    }

    #[test]
    fn welcome_records_our_seat() {
        let mut c = detached();
        c.ingest(ServerMsg::Welcome {
            your_seat: PlayerId(3),
        });
        assert_eq!(c.your_seat, Some(PlayerId(3)));
    }

    #[test]
    fn a_state_update_becomes_the_rendered_view() {
        let mut c = detached();
        c.ingest(ServerMsg::Welcome {
            your_seat: PlayerId(3),
        });
        c.ingest(ServerMsg::StateUpdate(state_for(3, |e| e)));
        let gs = c.view();
        assert_eq!(gs.players.len(), 6);
        assert!(gs.players[3].hole_cards.is_some(), "hero cards present");
        assert!(gs.phase.equity <= 100, "equity is a percentage");
        assert!(gs.notice.is_none(), "no notice mid-hand");
    }

    #[test]
    fn your_turn_sets_legal_actions_and_a_call_is_sent() {
        let mut c = detached();
        c.ingest(ServerMsg::Welcome {
            your_seat: PlayerId(3),
        });
        c.ingest(ServerMsg::StateUpdate(state_for(3, |e| e)));
        c.ingest(ServerMsg::YourTurn {
            legal: vec![Action::Fold, Action::Call, Action::Raise { to: 200 }],
            deadline_ms: 30_000,
        });
        assert!(c.legal.is_some());
        // Pressing 'c' while facing a bet maps to Call and clears our turn.
        let consumed = c.handle_key(KeyCode::Char('c'));
        assert!(consumed, "a legal call is consumed");
        assert!(c.legal.is_none(), "turn cleared after acting");
    }

    #[test]
    fn keys_are_ignored_when_it_is_not_our_turn() {
        let mut c = detached();
        c.ingest(ServerMsg::Welcome {
            your_seat: PlayerId(3),
        });
        c.ingest(ServerMsg::StateUpdate(state_for(3, |e| e)));
        // No YourTurn yet.
        assert!(!c.handle_key(KeyCode::Char('f')), "ignored off-turn");
    }

    #[test]
    fn an_error_becomes_the_notice() {
        let mut c = detached();
        c.ingest(ServerMsg::Error("illegal action".into()));
        assert_eq!(c.view().notice.as_deref(), Some("illegal action"));
    }

    #[test]
    fn a_completed_state_shows_a_hand_over_notice() {
        let mut c = detached();
        c.ingest(ServerMsg::Welcome {
            your_seat: PlayerId(3),
        });
        let complete = state_for(3, |mut e| {
            for _ in 0..5 {
                e = apply(e, Action::Fold).unwrap();
            }
            e
        });
        c.ingest(ServerMsg::StateUpdate(complete));
        assert!(
            c.view().notice.unwrap().to_lowercase().contains("hand"),
            "completed hand surfaces a notice"
        );
    }
}
