use std::{fmt::Debug, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::{Mutex, RwLock}, time::Instant, io::{WriteHalf, ReadHalf, AsyncRead, AsyncWrite}};

pub type NodeID = u64;

pub trait UserMsg: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
impl<T> UserMsg for T where T: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}

pub trait Stream: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}
impl<T> Stream for T where T: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static {}

/// Remote procedure call api trait
#[async_trait]
pub trait RPC<Msg: UserMsg>: Send + Sync + 'static {
    async fn members(&self) -> Vec<NodeID>;
    async fn recv_msg(&self) -> Message<Msg>;
    async fn send_msg(&self, msg: Message<Msg>);
    /// This ID should never change
    fn our_id(&self) -> NodeID;
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Copy)]
pub enum Role {
    Leader,
    Follower,
    Candidate,
}
#[derive(Derivative)]
#[derivative(Debug)]
pub struct Senator<Msg: UserMsg, R: RPC<Msg>> {
    pub id: NodeID,
    pub rpc: Arc<R>,
    pub role: RwLock<Role>,
    pub term: RwLock<u64>,
    pub voted_for: Mutex<Option<NodeID>>,
    pub next_timeout: Mutex<Instant>,
    pub current_leader: Mutex<Option<NodeID>>,
    #[derivative(Debug="ignore")]
    pub on_role: RwLock<Vec<Box<dyn Fn(Role) + Send + Sync + 'static>>>,
    #[derivative(Debug="ignore")]
    pub on_message: RwLock<Vec<Box<dyn Fn(Message<Msg>) + Send + Sync + 'static>>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Message<UserMsg> {
    pub from: NodeID,
    pub from_role: Role,
    pub to : NodeID,
    pub term: u64,
    pub msg: MessageType<UserMsg>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum MessageType<UserMsg> {
    LeaderHeartbeat,
    VoteRequest,
    VoteGranted,
    Custom(UserMsg)
}
#[derive(Debug)]
pub struct Peer<S: Stream> {
    pub established_by: NodeID,
    pub peer_id: NodeID,
    pub write_half: Mutex<WriteHalf<S>>,
    pub read_half: Mutex<ReadHalf<S>>,
}