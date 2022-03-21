use std::sync::Arc;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

use crate::{
    types::{Message, Peer, Stream},
    Error, NodeID, Request, Response, UserReq, UserRes,
};

/// format of message:
/// 4 bytes for length of message
impl<S: Stream> Peer<S> {
    pub fn new(id: NodeID, stream: S) -> Arc<Self> {
        let (read_half, write_half) = tokio::io::split(stream);
        Arc::new(Self {
            id,
            read_half: Mutex::new(read_half),
            write_half: Mutex::new(write_half),
        })
    }

    pub async fn read_msg<Req: UserReq, Res: UserRes>(&self) -> Result<Message<Req, Res>, Error> {
        let mut read_half = self.read_half.lock().await;

        let mut buf = [0u8; 4];

        read_half.read_exact(&mut buf).await.map_err(|e| Error::IO(e))?;

        let msg_len = u32::from_be_bytes(buf);
        let mut buf = vec![0u8; msg_len as usize];

        read_half.read_exact(&mut buf).await.map_err(|e| Error::IO(e))?;
        let mut msg = bincode::deserialize::<Message<Req, Res>>(&buf)
            .map_err(|_| Error::CouldNotDeserialize)?;

        match &mut msg {
            Message::Request(req) => { req.peer_id = self.id },
            Message::Response(res) => { res.peer_id = self.id },
        }

        Ok(msg)
    }

    pub async fn send_msg<Req: UserReq, Res: UserRes>(
        &self,
        msg: Message<Req, Res>,
    ) -> Result<(), Error> {
        let mut write_half = self.write_half.lock().await;

        let buf = bincode::serialize(&msg).map_err(|_| Error::CouldNotSerialize)?;

        let len: [u8; 4] = (buf.len() as u32).to_be_bytes();

        write_half.write_all(&len).await.map_err(|e| Error::IO(e))?;
        write_half.write_all(&buf).await.map_err(|e| Error::IO(e))?;

        Ok(())
    }

    /// Send a request to the peer
    /// the request should probably be wrapped in a timeout to ensure that we eventually get a response
    pub async fn send_request<Req: UserReq, Res: UserRes>(
        &self,
        req: Request<Req>,
    ) -> Result<(), Error> {
        self.send_msg::<Req, Res>(Message::Request(req)).await?;

        Ok(())
    }

    pub async fn send_response<Req: UserReq, Res: UserRes>(
        &self,
        res: Response<Res>,
    ) -> Result<(), Error> {
        self.send_msg::<Req, Res>(Message::Response(res)).await?;

        Ok(())
    }
}
