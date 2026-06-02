//! Async server task. Fully implemented in Task 5; this is a temporary stub so
//! the binary crate compiles while the pure `room` state machine is built and
//! unit-tested.

use crate::room::RoomConfig;

pub async fn serve(_bind: &str, _config: RoomConfig) -> std::io::Result<()> {
    unimplemented!("net::serve is implemented in Task 5")
}
