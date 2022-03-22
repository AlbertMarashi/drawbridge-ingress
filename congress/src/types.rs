use std::{fmt::Debug, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::{Mutex, RwLock}, time::Instant, io::{WriteHalf, ReadHalf, AsyncRead, AsyncWrite}};

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
    async fn recv_msg(&self) -> Message<Req, Res>;
    async fn send_msg(&self, msg: Message<Req, Res>);
}

#[derive(Debug, Clone)]
pub enum Role {
    Leader,
    Follower,
    Candidate,
}

#[derive(Debug)]
pub enum UserMessage<Req, Res> {
    Request(Req),
    Response(Res),
}

#[derive(Derivative)]
#[derivative(Debug)]
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
    #[derivative(Debug="ignore")]
    pub on_role: RwLock<Vec<Box<dyn Fn(Role) + Send + Sync + 'static>>>,
    #[derivative(Debug="ignore")]
    pub on_message: RwLock<Vec<Box<dyn Fn(UserMessage<Req, Res>) + Send + Sync + 'static>>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Message<Req, Res> {
    pub from: NodeID,
    pub to : NodeID,
    pub term: u64,
    pub msg: MessageType<Req, Res>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum MessageType<Req, Res> {
    Request(Request<Req>),
    Response(Response<Res>),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Response<Res> {
    Heartbeat,
    Vote { vote_granted: bool },
    Custom(Res),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Request<Req> {
    Heartbeat,
    VoteRequest,
    Custom(Req),
}
#[derive(Debug)]
pub struct Peer<S: Stream> {
    pub id: NodeID,
    pub write_half: Mutex<WriteHalf<S>>,
    pub read_half: Mutex<ReadHalf<S>>,
}