//! Wire protocol for networked play: the client/server message enums, a
//! per-recipient filtered view of the table, and length-prefixed framing.
//!
//! No game rules live here — those stay in `poker-core`. This crate is the
//! shared vocabulary the server and the TUI client both depend on.

pub mod frame;
pub mod msg;
pub mod state;

pub use frame::{MAX_FRAME, read_msg, write_msg};
pub use msg::{ClientMsg, ServerMsg};
pub use state::{PublicSeat, PublicState};
