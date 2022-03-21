use std::{fmt::Debug, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::{Mutex}, time::Instant, io::{WriteHalf, ReadHalf, AsyncRead, AsyncWrite}};
use uuid::Uuid;

use crate::Error;

pub type NodeID = u64;

pub trait Stream: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}
pub trait UserReq: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
pub trait UserRes: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}

impl<T> UserReq for T where T: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
impl<T> UserRes for T where T: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
impl<T> Stream for T where T: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}

/// Remote procedure call api trait
#[async_trait]
pub trait RPC<Req: UserReq, Res: UserRes>: Send + Sync + 'static {
    async fn members(&self) -> Vec<NodeID>;
    async fn recv(&self) -> Option<Request<Req>>;
    async fn send_request(&self, req: Request<Req>) -> Result<Response<Res>, Error>;
    async fn send_response(&self, res: Response<Res>) -> Result<(), Error>;
}

#[derive(Debug, Clone)]
pub enum Role {
    Leader,
    Follower,
    Candidate,
}

#[derive(Debug)]
pub struct Senator<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    pub id: NodeID,
    pub rpc: Arc<R>,
    pub role: Mutex<Role>,
    pub term: Mutex<u64>,
    pub last_state: Mutex<Option<Req>>,
    pub voted_for: Mutex<Option<NodeID>>,
    pub next_timeout: Mutex<Instant>,
    pub current_leader: Mutex<Option<NodeID>>,
    pub phantom_res: std::marker::PhantomData<Res>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Message<Req, Res> {
    Request(Request<Req>),
    Response(Response<Res>),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum ResponseType<Res> {
    Heartbeat,
    RequestVote { vote_granted: bool },
    Custom(Res),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Response<Res> {
    pub peer_id: NodeID,
    pub req_id: Uuid,
    pub term: u64,
    pub msg: ResponseType<Res>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Request<Req> {
    pub peer_id: NodeID,
    pub req_id: Uuid,
    pub term: u64,
    pub msg: RequestType<Req>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum RequestType<Req> {
    Heartbeat,
    RequestVote,
    Custom(Req),
}

pub struct VoteRequest {
    pub term: u64,
    pub req_id: Uuid,
    pub peer_id: NodeID,
}

#[derive(Debug)]
pub struct Peer<S: Stream> {
    pub id: NodeID,
    pub write_half: Mutex<WriteHalf<S>>,
    pub read_half: Mutex<ReadHalf<S>>,
}