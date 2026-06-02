use poker_core::holdem::{Action, PlayerId};
use serde::{Deserialize, Serialize};

use crate::state::PublicState;

/// A message from a client to the server. The server treats every variant as
/// untrusted input: a `Join` may name an already-taken seat, an `Action` may be
/// illegal or arrive out of turn. Validation happens server-side in plan 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Join { name: String },
    Action(Action),
    Chat(String),
}

/// A message from the server to one client. `StateUpdate` carries that client's
/// own filtered `PublicState`; `YourTurn` is only ever sent to the seat that is
/// actually to act, with the exact set of legal actions the server will accept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    /// Sent once when the client is seated, telling it which seat it owns.
    Welcome { your_seat: PlayerId },
    /// The filtered table state, rebuilt for this recipient after every change.
    StateUpdate(PublicState),
    /// Only the seat to act receives this. `legal` is authoritative; the client
    /// must not offer anything outside it. `deadline_ms` is how long the client
    /// has before the server auto-folds.
    YourTurn {
        legal: Vec<Action>,
        deadline_ms: u64,
    },
    /// A chat line relayed from another player.
    Chat { who: String, msg: String },
    /// A rejected action or protocol problem, surfaced to the user.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_action_round_trips_through_bincode() {
        let msg = ClientMsg::Action(Action::Raise { to: 300 });
        let bytes = bincode::serialize(&msg).expect("serialize");
        let back: ClientMsg = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back, msg);
    }

    #[test]
    fn server_your_turn_round_trips_through_bincode() {
        let msg = ServerMsg::YourTurn {
            legal: vec![Action::Fold, Action::Call, Action::Raise { to: 200 }],
            deadline_ms: 30_000,
        };
        let bytes = bincode::serialize(&msg).expect("serialize");
        let back: ServerMsg = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back, msg);
    }
}
