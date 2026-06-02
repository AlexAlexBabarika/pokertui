use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Hard ceiling on a single frame's payload (1 MiB). A peer that claims a larger
/// length is rejected before any buffer is allocated, so a malicious or corrupt
/// length prefix cannot drive an unbounded allocation.
pub const MAX_FRAME: u32 = 1 << 20;

/// Serialize `msg` with bincode and write it as a `u32` big-endian length prefix
/// followed by the payload. Flushes before returning.
pub async fn write_msg<W, T>(w: &mut W, msg: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if bytes.len() as u64 > MAX_FRAME as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "outgoing frame exceeds MAX_FRAME",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed frame and decode it as `T`. Rejects any prefix above
/// `MAX_FRAME` before allocating. Returns the underlying I/O error on EOF
/// (`UnexpectedEof`), which the caller treats as a disconnect.
pub async fn read_msg<R, T>(r: &mut R) -> std::io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "incoming frame exceeds MAX_FRAME",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::ClientMsg;
    use poker_core::holdem::Action;

    #[tokio::test]
    async fn write_then_read_round_trips_a_message() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let sent = ClientMsg::Action(Action::Bet { to: 500 });
        write_msg(&mut a, &sent).await.unwrap();
        let got: ClientMsg = read_msg(&mut b).await.unwrap();
        assert_eq!(got, sent);
    }

    #[tokio::test]
    async fn two_frames_are_decoded_in_order() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_msg(&mut a, &ClientMsg::Join { name: "you".into() })
            .await
            .unwrap();
        write_msg(&mut a, &ClientMsg::Chat("gl".into()))
            .await
            .unwrap();
        let first: ClientMsg = read_msg(&mut b).await.unwrap();
        let second: ClientMsg = read_msg(&mut b).await.unwrap();
        assert_eq!(first, ClientMsg::Join { name: "you".into() });
        assert_eq!(second, ClientMsg::Chat("gl".into()));
    }

    #[tokio::test]
    async fn an_oversized_length_prefix_is_rejected_without_allocating() {
        let (mut a, mut b) = tokio::io::duplex(64);
        // Hand-write a length prefix one byte over the ceiling.
        a.write_all(&(MAX_FRAME + 1).to_be_bytes()).await.unwrap();
        a.flush().await.unwrap();
        let err = read_msg::<_, ClientMsg>(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_closed_peer_reads_as_eof() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        let err = read_msg::<_, ClientMsg>(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
