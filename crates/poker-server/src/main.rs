mod net;
mod room;

use std::time::Duration;

use clap::Parser;

use crate::room::RoomConfig;

/// Headless Texas Hold'em server: hosts one room, deals once every seat is
/// filled, and plays hands until only one funded player remains.
#[derive(Parser, Debug)]
#[command(name = "pokertui-server")]
struct Args {
    /// Address to bind, e.g. 0.0.0.0:4000
    #[arg(long, default_value = "0.0.0.0:4000")]
    bind: String,
    /// Number of seats; the first hand deals once this many players join.
    #[arg(long, default_value_t = 2)]
    seats: usize,
    /// Starting stack per seat.
    #[arg(long, default_value_t = 10_000)]
    buy_in: u64,
    #[arg(long, default_value_t = 50)]
    small_blind: u64,
    #[arg(long, default_value_t = 100)]
    big_blind: u64,
    /// Seconds a player has to act before being auto-folded.
    #[arg(long, default_value_t = 30)]
    turn_secs: u64,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let config = RoomConfig {
        seats: args.seats,
        buy_in: args.buy_in,
        small_blind: args.small_blind,
        big_blind: args.big_blind,
        turn_timeout: Duration::from_secs(args.turn_secs),
    };
    net::serve(&args.bind, config).await
}
