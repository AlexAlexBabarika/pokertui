use crossterm::event::KeyCode;

use crate::state::GameState;

/// The single seam the render loop talks to. `App` (local play vs bots) and
/// `NetClient` (networked play) both implement it, so `main` can drive either
/// without knowing which it holds.
pub trait Table {
    /// The current presentation snapshot to render this frame.
    fn view(&mut self) -> GameState;
    /// Handle one key press. Returns true if it changed anything meaningful.
    fn handle_key(&mut self, key: KeyCode) -> bool;
    /// Advance time-based work once per tick (bot pacing locally; draining
    /// inbound network messages remotely).
    fn step(&mut self);
}
