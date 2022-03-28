use std::sync::Arc;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

use crate::{
    types::{Message, Peer, Stream, UserMsg},
    Error, NodeID
};

/// format of message:
/// 4 bytes for length of message
impl<S: Stream> Peer<S> {
    pub fn new(established_by: NodeID, peer_id: NodeID, stream: S) -> Arc<Self> {
        let (read_half, write_half) = tokio::io::split(stream);
        Arc::new(Self {
            peer_id,
            established_by,
            read_half: Mutex::new(read_half),
            write_half: Mutex::new(write_half),
        })
    }

    pub async fn read_msg<Msg: UserMsg>(&self) -> Result<Message<Msg>, Error> {
        let mut read_half = self.read_half.lock().await;

        let mut buf = [0u8; 4];

        read_half.read_exact(&mut buf).await.map_err(|e| Error::IO(e))?;

        let msg_len = u32::from_be_bytes(buf);
        let mut buf = vec![0u8; msg_len as usize];

        read_half.read_exact(&mut buf).await.map_err(|e| Error::IO(e))?;
        let msg = bincode::deserialize::<Message<Msg>>(&buf)
            .map_err(|_| Error::CouldNotDeserialize)?;

        Ok(msg)
    }

    pub async fn send_msg<Msg: UserMsg>(
        &self,
        msg: Message<Msg>,
    ) -> Result<(), Error> {
        let mut write_half = self.write_half.lock().await;

        let buf = bincode::serialize(&msg).map_err(|_| Error::CouldNotSerialize)?;

        let len: [u8; 4] = (buf.len() as u32).to_be_bytes();

        write_half.write_all(&len).await.map_err(|e| Error::IO(e))?;
        write_half.write_all(&buf).await.map_err(|e| Error::IO(e))?;

        Ok(())
    }
}


impl<S: Stream> Drop for Peer<S> {
    fn drop(&mut self) {
        println!("Dropping peer with id {} established by {}", self.peer_id, self.established_by);
    }
}