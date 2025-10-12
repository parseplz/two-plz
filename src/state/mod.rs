use crate::{Connection, state::read::read_runner};
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod read;
mod write;

// Run the entire client/server connection loop
// E => Sent to user
// U => Received from user
async fn connection_runner<T, E, U>(mut conn: Connection<T, E, U>)
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    tokio::select! {
        frame = conn.stream.next() => {
            match frame {
                Some(Ok(frame)) => {
                    let state_result = read_runner(&mut conn, frame);
                    todo!()
                }
                Some(Err(_)) => todo!(),
                None => todo!(),
            }
        }
        msg = conn.handler.receiver.recv() => {
            todo!()
        }
    }
}
