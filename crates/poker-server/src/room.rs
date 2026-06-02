use std::time::Duration;

use poker_core::holdem::{Action, HandConfig, HandState, Phase, PlayerId, apply, legal_actions};
use poker_net::msg::ServerMsg;
use poker_net::state::PublicState;

/// Static room settings, fixed at startup from the CLI.
#[derive(Debug, Clone)]
pub struct RoomConfig {
    pub seats: usize,
    pub buy_in: u64,
    pub small_blind: u64,
    pub big_blind: u64,
    pub turn_timeout: Duration,
}

/// Who an `Outbound` is addressed to.
#[derive(Debug, Clone, PartialEq)]
pub enum Recipient {
    All,
    Seat(PlayerId),
}

/// One message the server task must deliver. The room produces these; the net
/// layer routes them to the right socket(s).
#[derive(Debug, Clone, PartialEq)]
pub struct Outbound {
    pub to: Recipient,
    pub msg: ServerMsg,
}

/// Everything that can change the room. The net layer manufactures these from
/// socket events and timers; the room is the sole place they are interpreted.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomEvent {
    /// A newly accepted connection claimed `seat` and announced `name`.
    Join {
        seat: PlayerId,
        name: String,
    },
    /// An action packet from `seat` (untrusted — may be illegal/out of turn).
    Action {
        seat: PlayerId,
        action: Action,
    },
    Chat {
        seat: PlayerId,
        text: String,
    },
    /// The turn timer for `seat` fired; `generation` guards against stale timers.
    Timeout {
        seat: PlayerId,
        generation: u64,
    },
    Disconnect {
        seat: PlayerId,
    },
    /// Sent by the net layer after the post-hand pause, to deal the next hand.
    Continue,
}

pub struct Room {
    config: RoomConfig,
    names: Vec<Option<String>>,
    connected: Vec<bool>,
    engine: Option<HandState>,
    /// Monotonic turn counter. Every state advance bumps it; a `Timeout` whose
    /// `gen` is stale is ignored, so a player who acts just before their timer
    /// fires is not double-processed.
    turn_gen: u64,
    /// Wall-clock seed source for deals; bumped per hand for variety.
    seed_ctr: u64,
}

impl Room {
    pub fn new(config: RoomConfig) -> Self {
        let n = config.seats;
        Room {
            names: vec![None; n],
            connected: vec![false; n],
            engine: None,
            turn_gen: 0,
            seed_ctr: 0,
            config,
        }
    }

    /// The current turn generation. The net layer tags each spawned timer with
    /// this so it can discard timers that a later action has invalidated.
    pub fn turn_gen(&self) -> u64 {
        self.turn_gen
    }

    pub fn turn_timeout(&self) -> Duration {
        self.config.turn_timeout
    }

    /// Number of seats in the room — used by the net layer to size its per-seat
    /// outbox table.
    pub fn seat_count(&self) -> usize {
        self.config.seats
    }

    /// True once every seat has a connected player and the game has not started.
    fn all_seated(&self) -> bool {
        self.engine.is_none() && self.connected.iter().all(|&c| c)
    }

    /// Interpret one event, mutating room state and returning every message the
    /// net layer must deliver. The engine is only ever advanced through
    /// `apply`, so no event can produce an illegal game state.
    pub fn apply_event(&mut self, event: RoomEvent) -> Vec<Outbound> {
        match event {
            RoomEvent::Join { seat, name } => self.on_join(seat, name),
            RoomEvent::Action { seat, action } => self.on_action(seat, action),
            RoomEvent::Chat { seat, text } => self.on_chat(seat, text),
            RoomEvent::Timeout { seat, generation } => self.on_timeout(seat, generation),
            RoomEvent::Disconnect { seat } => self.on_disconnect(seat),
            RoomEvent::Continue => self.on_continue(),
        }
    }

    fn on_join(&mut self, seat: PlayerId, name: String) -> Vec<Outbound> {
        self.names[seat.0] = Some(name);
        self.connected[seat.0] = true;
        let mut out = vec![Outbound {
            to: Recipient::Seat(seat),
            msg: ServerMsg::Welcome { your_seat: seat },
        }];
        if self.all_seated() {
            self.deal_first_hand();
            out.extend(self.broadcast_state());
            out.extend(self.announce_turn());
        } else {
            // Late joiners still get the lobby view (no hand yet → no YourTurn).
            out.extend(self.broadcast_state());
        }
        out
    }

    fn deal_first_hand(&mut self) {
        let cfg = HandConfig {
            num_players: self.config.seats,
            small_blind: self.config.small_blind,
            big_blind: self.config.big_blind,
            dealer: PlayerId(0),
            seed: self.next_seed(),
        };
        let stacks = vec![self.config.buy_in; self.config.seats];
        self.engine = Some(HandState::new_hand(cfg, stacks));
        self.turn_gen += 1;
    }

    fn next_seed(&mut self) -> u64 {
        self.seed_ctr = self.seed_ctr.wrapping_add(1);
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xC0FFEE);
        wall ^ self.seed_ctr.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// A filtered `StateUpdate` for every connected seat. Before the first deal
    /// there is no engine, so nothing is sent.
    fn broadcast_state(&self) -> Vec<Outbound> {
        let Some(engine) = &self.engine else {
            return Vec::new();
        };
        let names: Vec<String> = self
            .names
            .iter()
            .map(|n| n.clone().unwrap_or_default())
            .collect();
        (0..self.config.seats)
            .filter(|&i| self.connected[i])
            .map(|i| Outbound {
                to: Recipient::Seat(PlayerId(i)),
                msg: ServerMsg::StateUpdate(PublicState::for_recipient(
                    engine,
                    &names,
                    PlayerId(i),
                )),
            })
            .collect()
    }

    /// Tell the seat to act what it may do — unless that seat is disconnected, in
    /// which case auto-fold it immediately and recurse to the next actor.
    fn announce_turn(&mut self) -> Vec<Outbound> {
        let Some(engine) = &self.engine else {
            return Vec::new();
        };
        let Some(actor) = engine.to_act else {
            return Vec::new();
        };
        if !self.connected[actor.0] {
            return self.force_fold(actor);
        }
        vec![Outbound {
            to: Recipient::Seat(actor),
            msg: ServerMsg::YourTurn {
                legal: legal_actions(engine),
                deadline_ms: self.config.turn_timeout.as_millis() as u64,
            },
        }]
    }

    fn on_action(&mut self, seat: PlayerId, action: Action) -> Vec<Outbound> {
        let Some(engine) = &self.engine else {
            return vec![self.error(seat, "no hand in progress")];
        };
        // Never trust the client: it must be this seat's turn.
        if engine.to_act != Some(seat) {
            return vec![self.error(seat, "not your turn")];
        }
        // Move the engine out, attempt the action, move the result back.
        let taken = self.engine.take().expect("engine present");
        match apply(taken, action) {
            Ok(next) => {
                self.engine = Some(next);
                self.turn_gen += 1;
                let mut out = self.broadcast_state();
                out.extend(self.announce_turn());
                out
            }
            Err((restored, _)) => {
                // Illegal: restore untouched, tell the actor, re-arm their turn.
                self.engine = Some(restored);
                let mut out = vec![self.error(seat, "illegal action")];
                out.extend(self.announce_turn());
                out
            }
        }
    }

    /// Apply a forced Fold for `seat` (timeout or disconnect). If the fold is not
    /// currently legal for that seat the engine is left untouched.
    fn force_fold(&mut self, seat: PlayerId) -> Vec<Outbound> {
        let Some(engine) = &self.engine else {
            return Vec::new();
        };
        if engine.to_act != Some(seat) {
            return Vec::new();
        }
        let taken = self.engine.take().expect("engine present");
        match apply(taken, Action::Fold) {
            Ok(next) => {
                self.engine = Some(next);
                self.turn_gen += 1;
                let mut out = self.broadcast_state();
                out.extend(self.announce_turn());
                out
            }
            Err((restored, _)) => {
                self.engine = Some(restored);
                Vec::new()
            }
        }
    }

    fn on_chat(&mut self, seat: PlayerId, text: String) -> Vec<Outbound> {
        let who = self.names[seat.0].clone().unwrap_or_default();
        vec![Outbound {
            to: Recipient::All,
            msg: ServerMsg::Chat { who, msg: text },
        }]
    }

    fn on_timeout(&mut self, seat: PlayerId, generation: u64) -> Vec<Outbound> {
        // A timer is only valid for the generation it was armed in. Any action
        // (or another timeout) since then bumped `turn_gen`, making this stale.
        if generation != self.turn_gen {
            return Vec::new();
        }
        self.force_fold(seat)
    }

    fn on_disconnect(&mut self, seat: PlayerId) -> Vec<Outbound> {
        self.connected[seat.0] = false;
        // If it was their turn, fold now; otherwise `announce_turn` will fold
        // them the moment the action reaches their seat.
        let mut out = Vec::new();
        if let Some(engine) = &self.engine
            && engine.to_act == Some(seat)
        {
            out.extend(self.force_fold(seat));
        }
        out
    }

    fn on_continue(&mut self) -> Vec<Outbound> {
        match &self.engine {
            Some(engine) if engine.phase == Phase::Complete && engine.funded_seats() >= 2 => {}
            // Game over, no hand yet, or called spuriously — nothing to deal.
            _ => return Vec::new(),
        }
        let seed = self.next_seed();
        let next = self
            .engine
            .as_ref()
            .expect("engine present")
            .next_hand(seed);
        self.engine = Some(next);
        self.turn_gen += 1;
        let mut out = self.broadcast_state();
        out.extend(self.announce_turn());
        out
    }

    fn error(&self, seat: PlayerId, msg: &str) -> Outbound {
        Outbound {
            to: Recipient::Seat(seat),
            msg: ServerMsg::Error(msg.to_string()),
        }
    }

    #[cfg(test)]
    fn engine(&self) -> &HandState {
        self.engine.as_ref().expect("hand in progress")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(seats: usize) -> RoomConfig {
        RoomConfig {
            seats,
            buy_in: 10_000,
            small_blind: 50,
            big_blind: 100,
            turn_timeout: Duration::from_secs(30),
        }
    }

    fn join_all(room: &mut Room, n: usize) -> Vec<Outbound> {
        let mut out = Vec::new();
        for i in 0..n {
            out.extend(room.apply_event(RoomEvent::Join {
                seat: PlayerId(i),
                name: format!("p{i}"),
            }));
        }
        out
    }

    #[test]
    fn a_join_is_welcomed_with_its_seat() {
        let mut room = Room::new(config(2));
        let out = room.apply_event(RoomEvent::Join {
            seat: PlayerId(0),
            name: "you".into(),
        });
        assert!(out.contains(&Outbound {
            to: Recipient::Seat(PlayerId(0)),
            msg: ServerMsg::Welcome {
                your_seat: PlayerId(0)
            },
        }));
    }

    #[test]
    fn the_game_starts_once_every_seat_is_filled() {
        let mut room = Room::new(config(2));
        let out = join_all(&mut room, 2);
        // The last join deals the hand: someone is now to act, and that seat got
        // a YourTurn, and everyone got a StateUpdate.
        assert_eq!(room.engine().phase, Phase::Preflop);
        let your_turns = out
            .iter()
            .filter(|o| matches!(o.msg, ServerMsg::YourTurn { .. }))
            .count();
        assert_eq!(your_turns, 1, "exactly the seat to act gets YourTurn");
        let updates = out
            .iter()
            .filter(|o| matches!(o.msg, ServerMsg::StateUpdate(_)))
            .count();
        assert_eq!(updates, 2, "both seats get a filtered StateUpdate");
    }

    /// Drive a 2-seat room to the start of a hand and return the seat to act.
    fn started() -> (Room, PlayerId) {
        let mut room = Room::new(config(2));
        join_all(&mut room, 2);
        let actor = room.engine().to_act.unwrap();
        (room, actor)
    }

    #[test]
    fn an_out_of_turn_action_is_rejected_and_state_is_untouched() {
        let (mut room, actor) = started();
        let other = PlayerId((actor.0 + 1) % 2);
        let log_before = room.engine().log.len();
        let out = room.apply_event(RoomEvent::Action {
            seat: other,
            action: Action::Fold,
        });
        assert!(
            out.iter()
                .any(|o| o.to == Recipient::Seat(other) && matches!(o.msg, ServerMsg::Error(_))),
            "the off-turn seat gets an Error"
        );
        assert_eq!(
            room.engine().log.len(),
            log_before,
            "an off-turn packet must not advance the engine"
        );
    }

    #[test]
    fn an_illegal_action_is_rejected_and_the_turn_is_re_announced() {
        let (mut room, actor) = started();
        // Preflop facing the BB, Check is illegal for the seat to act.
        let out = room.apply_event(RoomEvent::Action {
            seat: actor,
            action: Action::Check,
        });
        assert!(
            out.iter().any(|o| matches!(o.msg, ServerMsg::Error(_))),
            "illegal action yields an Error"
        );
        assert!(
            out.iter()
                .any(|o| o.to == Recipient::Seat(actor)
                    && matches!(o.msg, ServerMsg::YourTurn { .. })),
            "the actor is reminded it is still their turn"
        );
        assert_eq!(room.engine().to_act, Some(actor), "still their turn");
    }

    #[test]
    fn a_legal_action_advances_the_engine_and_broadcasts() {
        let (mut room, actor) = started();
        let out = room.apply_event(RoomEvent::Action {
            seat: actor,
            action: Action::Call,
        });
        assert_ne!(room.engine().to_act, Some(actor), "turn advanced");
        assert_eq!(
            out.iter()
                .filter(|o| matches!(o.msg, ServerMsg::StateUpdate(_)))
                .count(),
            2,
            "both seats get a fresh filtered state"
        );
        assert_eq!(
            out.iter()
                .filter(|o| matches!(o.msg, ServerMsg::YourTurn { .. }))
                .count(),
            1,
            "the new actor gets YourTurn"
        );
    }

    #[test]
    fn a_completed_hand_broadcasts_but_sends_no_your_turn() {
        // Heads-up: dealer/SB (seat 0) is to act first preflop. SB folds → BB
        // wins immediately, hand is Complete.
        let (mut room, actor) = started();
        let out = room.apply_event(RoomEvent::Action {
            seat: actor,
            action: Action::Fold,
        });
        assert_eq!(room.engine().phase, Phase::Complete);
        assert!(
            !out.iter()
                .any(|o| matches!(o.msg, ServerMsg::YourTurn { .. })),
            "no one acts on a completed hand"
        );
        assert!(
            out.iter()
                .any(|o| matches!(o.msg, ServerMsg::StateUpdate(_))),
            "the completed (showdown) state is still broadcast"
        );
    }
}
