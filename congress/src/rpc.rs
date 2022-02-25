use std::{collections::HashMap, time::Duration};

use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tokio::{io::{AsyncRead, AsyncWrite, AsyncWriteExt, AsyncReadExt}, sync::RwLock, time::timeout};

use crate::{RPC, NodeID, Request, Response};


pub struct RPCNetwork<S> {
    pub listeners: RwLock<HashMap<NodeID, Peer<S>>>,
    pub listening: RwLock<HashMap<NodeID, Peer<S>>>
}

impl<S> RPCNetwork<S> {
    pub fn new() -> Self {
        Self {
            listeners: RwLock::new(HashMap::new()),
            listening: RwLock::new(HashMap::new())
        }
    }

    pub async fn add_listener(&self, id: NodeID, client: Peer<S>) {
        self.listeners.write().await.insert(id, client);
    }

    pub async fn add_listening(&self, id: NodeID, client: Peer<S>) {
        self.listening.write().await.insert(id, client);
    }
}

#[async_trait]
impl<S, State, StateRes> RPC<State, StateRes> for RPCNetwork<S> where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    State: 'static + Send + Serialize + Clone,
    StateRes: 'static + Send + DeserializeOwned,
{
    async fn broadcast_request(&self, req: Request<State>) -> Vec<Response<StateRes>> {
        let mut clients = self.listeners.write().await;

        let responses: Vec<_> = clients
            .values_mut()
            .map(|peer| peer.send_request::<State, StateRes>(req.clone()))
            .collect();

        futures::future::join_all(responses).await
            .into_iter()
            .filter_map(|res| res)
            .collect()
    }

    async fn handle_request(&self, req: Request<State>) -> Response<StateRes> {
        unimplemented!()
    }
}

/// Peer
pub struct Peer<S> {
    id: NodeID,
    stream: S,
}

#[derive(Debug, Serialize, Deserialize)]
struct Frame {
    length: u32,
    data: Vec<u8>,
}


impl<S> Peer<S> where S: AsyncRead + AsyncWrite + Send + Sync + Unpin {
    pub fn new(id: NodeID, stream: S) -> Self {
        Self {
            id,
            stream,
        }
    }

    /// Send a request to the peer
    /// The writes and reads can time out if the peer is not available,
    /// so in that case, we return None
    pub async fn send_request<Req: Serialize, Res: DeserializeOwned>(&mut self, req: Request<Req>) -> Option<Response<Res>> {
        let stream = &mut self.stream;

        // Write the request
        let req = bincode::serialize(&req).ok()?;
        let frame = bincode::serialize(&Frame { length: req.len() as u32, data: req }).ok()?;
        timeout(Duration::from_millis(40), stream.write_all(&frame))
            .await.ok()?
            .ok()?;

        // Read the response
        let mut buf = [0u8; 4];
        timeout(Duration::from_millis(40), stream.read_exact(&mut buf))
            .await.ok()?
            .ok()?;
        let length = u32::from_be_bytes(buf);
        let mut buf = vec![0u8; length as usize];
        timeout(Duration::from_millis(40), stream.read_exact(&mut buf))
            .await.ok()?
            .ok()?;

        let res: Response<Res> = bincode::deserialize(&buf).ok()?;

        Some(res)
    }
}