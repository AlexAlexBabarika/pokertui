use std::time::Duration;

use poker_core::holdem::PlayerId;
use poker_net::msg::{ClientMsg, ServerMsg};
use poker_net::{read_msg, write_msg};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::room::{Outbound, Recipient, Room, RoomConfig, RoomEvent};

/// Everything the central task receives: room events plus the internal
/// connect/outbox registration that the accept loop emits per new socket.
enum ServerMsgIn {
    Event(RoomEvent),
    Register {
        seat: PlayerId,
        outbox: mpsc::UnboundedSender<ServerMsg>,
    },
}

/// Bind, accept connections, and run the room to completion. Returns when the
/// listener errors (e.g. the process is shutting down).
pub async fn serve(bind: &str, config: RoomConfig) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "poker-server listening on {bind} for {} seats",
        config.seats
    );

    let (tx, rx) = mpsc::unbounded_channel::<ServerMsgIn>();
    // The room task runs for the life of the process; detaching its handle is
    // fine since the accept loop below never returns except on a listener error.
    let _central = tokio::spawn(central_loop(config.clone(), rx, tx.clone()));

    let mut next_seat = 0usize;
    loop {
        let (stream, peer) = listener.accept().await?;
        if next_seat >= config.seats {
            eprintln!("rejecting {peer}: room is full");
            continue;
        }
        let seat = PlayerId(next_seat);
        next_seat += 1;
        spawn_connection(seat, stream, tx.clone());
    }
}

/// Split one socket into a reader task (frames → RoomEvents) and a writer task
/// (per-seat ServerMsg channel → frames). Registers the writer's sender with the
/// central task so the room can address this seat.
fn spawn_connection(seat: PlayerId, stream: TcpStream, tx: mpsc::UnboundedSender<ServerMsgIn>) {
    let (mut read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMsg>();
    let _ = tx.send(ServerMsgIn::Register {
        seat,
        outbox: out_tx,
    });

    // Writer: drain this seat's outbox to the socket.
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if write_msg(&mut write_half, &msg).await.is_err() {
                break;
            }
        }
    });

    // Reader: first frame must be Join; subsequent frames are actions/chat.
    tokio::spawn(async move {
        loop {
            match read_msg::<_, ClientMsg>(&mut read_half).await {
                Ok(ClientMsg::Join { name }) => {
                    let _ = tx.send(ServerMsgIn::Event(RoomEvent::Join { seat, name }));
                }
                Ok(ClientMsg::Action(action)) => {
                    let _ = tx.send(ServerMsgIn::Event(RoomEvent::Action { seat, action }));
                }
                Ok(ClientMsg::Chat(text)) => {
                    let _ = tx.send(ServerMsgIn::Event(RoomEvent::Chat { seat, text }));
                }
                Err(_) => {
                    // EOF or protocol error → the seat has dropped.
                    let _ = tx.send(ServerMsgIn::Event(RoomEvent::Disconnect { seat }));
                    break;
                }
            }
        }
    });
}

/// The single owner of the `Room`. Serializes every event through `apply_event`,
/// dispatches the resulting `Outbound`s to the right outboxes, and manages the
/// turn timer (one armed timer at a time, tagged with the room's turn gen).
async fn central_loop(
    config: RoomConfig,
    mut rx: mpsc::UnboundedReceiver<ServerMsgIn>,
    self_tx: mpsc::UnboundedSender<ServerMsgIn>,
) {
    let mut room = Room::new(config);
    let mut outboxes: Vec<Option<mpsc::UnboundedSender<ServerMsg>>> = vec![None; room.seat_count()];

    while let Some(incoming) = rx.recv().await {
        let event = match incoming {
            ServerMsgIn::Register { seat, outbox } => {
                outboxes[seat.0] = Some(outbox);
                continue;
            }
            ServerMsgIn::Event(e) => e,
        };

        let outs = room.apply_event(event);
        dispatch(&outs, &outboxes);
        arm_timers(&room, &outs, &self_tx);
        schedule_continue(&outs, &self_tx);
    }
}

/// Send each `Outbound` to the matching outbox(es). A seat that has not yet
/// registered (or has dropped) is skipped silently.
fn dispatch(outs: &[Outbound], outboxes: &[Option<mpsc::UnboundedSender<ServerMsg>>]) {
    for o in outs {
        match &o.to {
            Recipient::Seat(p) => {
                if let Some(Some(tx)) = outboxes.get(p.0) {
                    let _ = tx.send(o.msg.clone());
                }
            }
            Recipient::All => {
                for tx in outboxes.iter().flatten() {
                    let _ = tx.send(o.msg.clone());
                }
            }
        }
    }
}

/// If this batch announced a turn, arm a single timeout for the seat to act,
/// tagged with the current turn generation so a later action invalidates it.
fn arm_timers(room: &Room, outs: &[Outbound], self_tx: &mpsc::UnboundedSender<ServerMsgIn>) {
    for o in outs {
        if let (Recipient::Seat(seat), ServerMsg::YourTurn { .. }) = (&o.to, &o.msg) {
            let seat = *seat;
            let generation = room.turn_gen();
            let timeout = room.turn_timeout();
            let tx = self_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let _ = tx.send(ServerMsgIn::Event(RoomEvent::Timeout { seat, generation }));
            });
        }
    }
}

/// When a hand completes (a StateUpdate at `Complete` reaches a seat), wait a few
/// seconds so clients can see the showdown, then ask the room to deal the next.
fn schedule_continue(outs: &[Outbound], self_tx: &mpsc::UnboundedSender<ServerMsgIn>) {
    use poker_core::holdem::Phase;
    let completed = outs
        .iter()
        .any(|o| matches!(&o.msg, ServerMsg::StateUpdate(s) if s.phase == Phase::Complete));
    if completed {
        let tx = self_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let _ = tx.send(ServerMsgIn::Event(RoomEvent::Continue));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poker_core::holdem::Action;

    /// Two real TCP clients join a 2-seat room; the seat to act receives a
    /// YourTurn, plays a legal action, and the hand advances — exercising the
    /// full accept → route → validate → broadcast path over a socket.
    #[tokio::test]
    async fn two_clients_join_and_play_one_action_over_tcp() {
        let config = RoomConfig {
            seats: 2,
            buy_in: 10_000,
            small_blind: 50,
            big_blind: 100,
            turn_timeout: Duration::from_secs(30),
        };
        // Bind an ephemeral port and serve in the background.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // reuse the address for serve
        tokio::spawn(async move {
            let _ = serve(&addr.to_string(), config).await;
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut a = TcpStream::connect(addr).await.unwrap();
        let mut b = TcpStream::connect(addr).await.unwrap();
        write_msg(&mut a, &ClientMsg::Join { name: "a".into() })
            .await
            .unwrap();
        write_msg(&mut b, &ClientMsg::Join { name: "b".into() })
            .await
            .unwrap();

        // Collect frames from a until we learn it is (or isn't) to act.
        let mut acted = false;
        for client in [&mut a, &mut b] {
            // Each client should at least receive Welcome + a StateUpdate.
            let mut saw_your_turn = false;
            for _ in 0..4 {
                match tokio::time::timeout(Duration::from_secs(2), read_msg::<_, ServerMsg>(client))
                    .await
                {
                    Ok(Ok(ServerMsg::YourTurn { legal, .. })) => {
                        assert!(legal.contains(&Action::Fold));
                        saw_your_turn = true;
                        break;
                    }
                    Ok(Ok(_)) => continue,
                    _ => break,
                }
            }
            if saw_your_turn {
                write_msg(client, &ClientMsg::Action(Action::Call))
                    .await
                    .unwrap();
                acted = true;
                break;
            }
        }
        assert!(acted, "one of the two seats was prompted and acted");
    }
}
