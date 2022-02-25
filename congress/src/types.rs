use std::{time::{Instant, Duration}, sync::Arc};

use tokio::sync::Mutex;

pub type NodeID = u64;


/// Remote procedure call api trait
#[async_trait]
pub trait RPC<State, StateRes> {
    async fn broadcast_request(&self, req: Request<State>) -> Vec<Response<StateRes>>;
    async fn handle_request(&self, req: Request<State>) -> Response<StateRes>;
}

pub enum Role {
    Leader,
    Follower,
    Candidate,
}

pub struct Senator<R: RPC<State, StateRes>, State, StateRes> {
    pub id: NodeID,
    pub role: Mutex<Role>,
    pub rpc: R,
    pub term: Mutex<u64>,
    pub last_state: Mutex<State>,
    pub voted_for: Mutex<Option<NodeID>>,
    pub next_timeout: Mutex<Instant>,
    pub current_leader: Mutex<Option<NodeID>>,
    pub phantom_res: std::marker::PhantomData<StateRes>,
}

impl<R, State, StateRes> Senator<R, State, StateRes>
where
    R: RPC<State, StateRes>,
    State: Send,
    StateRes: Send,
{
    pub fn new(id: NodeID, rpc: R, initial_state: State) -> Arc<Self> {
        Arc::new(Senator {
            id,
            role: Mutex::new(Role::Follower),
            rpc,
            term: Mutex::new(0),
            last_state: Mutex::new(initial_state),
            voted_for: Mutex::new(None),
            next_timeout: Mutex::new(Instant::now() + Duration::from_millis(250)),
            current_leader: Mutex::new(None),
            phantom_res: std::marker::PhantomData,
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Request<State> {
    Heartbeat { term: u64, peer_id: NodeID },
    RequestVote { term: u64, candidate_id: NodeID },
    Custom(State),
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Response<StateRes> {
    Heartbeat { term: u64, success: bool },
    RequestVote { term: u64, vote_granted: bool },
    Custom(StateRes),
}