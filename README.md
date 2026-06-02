# pokertui

A terminal Texas Hold'em game written in Rust. Play heads-up or full-ring
against bots in a single binary, or connect to a headless server and play a
table over the network.

The interface is a TUI built with [ratatui](https://ratatui.rs), complete with
rendered cards, an action bar, and an optional live equity / pot-odds rail.

## Workspace layout

The project is a Cargo workspace of four crates:

- `poker-core` — the pure rules kernel: hand evaluation, Hold'em state machine,
  side pots, equity estimation, and bot logic. No UI, no async, no I/O.
- `poker-net` — the wire protocol: client/server messages, a per-recipient
  filtered view of the table, and length-prefixed framing.
- `poker-server` — a headless server that hosts one room, deals once every seat
  is filled, and runs hands with turn timers and disconnect auto-fold.
- `pokertui` — the terminal client. Plays locally against bots, or joins a
  server with `--join`.

## Requirements

- A recent Rust toolchain (edition 2024). Install via [rustup](https://rustup.rs).
- A terminal that supports an alternate screen and Unicode.

## Build

```sh
cargo build --release
```

## Play locally against bots

With no arguments the client deals a local table and fills the empty seats with
bots:

```sh
cargo run --release -p pokertui
```

Open the in-game settings menu (`S`) to adjust the table without leaving play.

## Networked play

Start a server:

```sh
cargo run --release -p poker-server -- --bind 0.0.0.0:4000 --seats 2
```

Then connect one client per seat:

```sh
cargo run --release -p pokertui -- --join 127.0.0.1:4000 --name alice
```

The first hand is dealt automatically once every seat is filled.

### Server options

| Flag | Default | Description |
| --- | --- | --- |
| `--bind` | `0.0.0.0:4000` | Address to bind. |
| `--seats` | `2` | Number of seats; the first hand deals once this many players join. |
| `--buy-in` | `10000` | Starting stack per seat. |
| `--small-blind` | `50` | Small blind. |
| `--big-blind` | `100` | Big blind. |
| `--turn-secs` | `30` | Seconds a player has to act before being auto-folded. |

### Client options

| Flag | Default | Description |
| --- | --- | --- |
| `--join` | _(none)_ | Server address to connect to. Omit for local play vs bots. |
| `--name` | `you` | Display name to join with. |

## Controls

| Key | Action |
| --- | --- |
| `F` | Fold |
| `C` | Check / call |
| `R` | Raise (uses the selected bet size) |
| `A` | All-in |
| `↑` / `↓` | Adjust bet size |
| `⏎` | Confirm |
| `S` | Settings (local play only) |
| `Q` | Quit |

When a hand finishes, any key deals the next one. The game ends once only one
player still has chips.

## Settings (local play)

The `S` menu lets you tune the local game; arrow keys move between rows and
cycle each value, `Esc` closes it:

- **Bot delay** — how long bots wait before acting (applies live).
- **Equity / pot-odds rail** — show or hide the win-rate panel (applies live).
- **Seats** — table size (applies on the next game).
- **Blinds** — small/big blind level (applies on the next game).

Settings persist to `$XDG_CONFIG_HOME/pokertui/config.toml`, falling back to
`$HOME/.config/pokertui/config.toml`. The file is plain TOML and safe to edit
by hand.

## Tests

```sh
cargo test
```

## License

Licensed under either of MIT or Apache-2.0, at your option.
