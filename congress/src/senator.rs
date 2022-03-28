use std::{collections::HashMap, sync::Arc};
use tokio::{
    select,
    sync::{
        mpsc::{self, Sender, Receiver},
        Mutex, RwLock, RwLockWriteGuard,
    },
    task::JoinHandle,
    time::Instant,
};

use tokio::time::Duration;

use crate::{
    states::{candidate::Candidate, follower::Follower, leader::Leader},
    types::{Message, MessageType, Peer, Stream, UserMsg},
    Error, NodeID, Role, Senator, RPC, utils::get_random_timeout,
};

impl<Msg: UserMsg, R: RPC<Msg>> Senator<Msg, R> {
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let senator = self.clone();

        tokio::spawn(async move {
            loop {
                // each loop is a new term
                let senator = senator.clone();
                let role = { (*senator.role.lock().await).clone() };

                // notify all on_role listeners that the role has changed
                senator
                    .on_role
                    .read()
                    .await
                    .iter()
                    .for_each(|f| f(role.clone()));

                // run the state for this loop
                match role {
                    Role::Leader => Leader::new(senator.clone()).run().await,
                    Role::Follower => Follower::new(senator.clone()).run().await,
                    Role::Candidate => Candidate::new(senator.clone()).run().await,
                }

                // reset the next timeout in case we are a follower
                *senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
            }
        })
    }

    pub async fn on_role<F: Fn(Role) + Send + Sync + 'static>(self: &Arc<Self>, cb: F) {
        let mut on_role = self.on_role.write().await;
        on_role.push(Box::new(cb))
    }

    pub async fn on_message<F: Fn(Message<Msg>) + Send + Sync + 'static>(
        self: &Arc<Self>,
        cb: F,
    ) {
        let mut on_message = self.on_message.write().await;
        on_message.push(Box::new(cb))
    }

    pub async fn handle_user_message(self: &Arc<Self>, msg: Message<Msg>) {
        // lock the message handlers
        let mut on_message = self.on_message.write().await;

        on_message
            .iter_mut()
            .for_each(|f| f(msg.clone()));
    }

    pub async fn handle_vote_request(self: &Arc<Self>, to: NodeID, their_term: u64) {
        self.rpc
            .send_msg({
                // if the candidates term is less than ours, we will reject the vote
                let mut our_term = self.term.lock().await;
                if their_term < *our_term {
                    Message {
                        term: *our_term,
                        from: self.id,
                        to,
                        msg: MessageType::VoteResponse {
                            vote_granted: false,
                        },
                    }
                // if we observe a term greater than our own, we will become a follower
                } else {
                    if their_term > *our_term {
                        *our_term = their_term;
                        *self.role.lock().await = Role::Follower;
                    }

                    let mut voted_for = self.voted_for.lock().await;

                    match *voted_for {
                        // if we have already voted for someone else, we will reject
                        Some(_) => Message {
                            term: *our_term,
                            from: self.id,
                            to,
                            msg: MessageType::VoteResponse {
                                vote_granted: false,
                            }
                        },
                        // if we have not voted for anyone, we will accept
                        None => {
                            *voted_for = Some(to);
                            Message {
                                term: *our_term,
                                to,
                                from: self.id,
                                msg: MessageType::VoteResponse { vote_granted: true },
                            }
                        }
                    }
                }
            })
            .await;
    }

    pub async fn broadcast_message(self: &Arc<Self>, msg: MessageType<Msg>) {
        let term = self.term.lock().await.clone();
        let clients = self.rpc.members().await;

        for peer_id in clients.into_iter() {
            let msg = msg.clone();
            let senator = self.clone();
            tokio::spawn(async move {
                senator
                    .rpc
                    .send_msg(Message {
                        from: senator.id,
                        to: peer_id,
                        term,
                        msg
                    })
                    .await
            });
        }
    }

    pub fn new(minimum_delay: Duration, rpc: Arc<R>) -> Arc<Self> {
        Arc::new(Senator {
            id: rpc.our_id(),
            rpc,
            role: Mutex::new(Role::Follower),
            term: Mutex::new(0),
            voted_for: Mutex::new(None),
            next_timeout: Mutex::new(Instant::now() + minimum_delay + get_random_timeout()),
            current_leader: Mutex::new(None),
            on_message: RwLock::new(Vec::new()),
            on_role: RwLock::new(Vec::new()),
        })
    }
}

#[derive(Debug)]
pub struct Close;

pub struct RPCNetwork<Msg: UserMsg, S: Stream> {
    pub peers: RwLock<HashMap<NodeID, (Sender<Close>, Receiver<Close>, Arc<Peer<S>>)>>,
    pub phantom_req: std::marker::PhantomData<Msg>,
    pub msg_recv: Mutex<mpsc::UnboundedReceiver<Message<Msg>>>,
    pub msg_send: mpsc::UnboundedSender<Message<Msg>>,
    pub our_id: NodeID,
}

#[async_trait]
impl<Msg: UserMsg, S: Stream> RPC<Msg> for RPCNetwork<Msg, S> {
    async fn recv_msg(&self) -> Message<Msg> {
        match self.msg_recv.lock().await.recv().await {
            Some(msg) => msg,
            None => panic!("msg_send dropped"),
        }
    }

    async fn send_msg(&self, msg: Message<Msg>) -> () {
        let to = msg.to;
        match self.peers.read().await.get(&to) {
            Some((_, _, peer)) => match peer.send_msg::<Msg>(msg).await {
                Ok(()) => return,
                Err(err) => dbg!("Could not send message: Peer probably closed connection", err),
            },
            None => return eprintln!("Peer {} not found, maybe it failed or was removed", msg.to)
        };

        // This peer errored out, remove it from the list
        let _ = self.remove_peer(to).await;
    }

    async fn members(&self) -> Vec<NodeID> {
        self.peers.read().await.keys().map(|key| *key).collect()
    }

    fn our_id(&self) -> NodeID {
        self.our_id
    }
}

impl<Msg: UserMsg, S: Stream> RPCNetwork<Msg, S> {
    pub fn new(our_id: NodeID) -> Arc<Self> {
        let (msg_send, msg_recv) = mpsc::unbounded_channel();
        Arc::new(RPCNetwork {
            peers: RwLock::new(HashMap::new()),
            phantom_req: std::marker::PhantomData,
            msg_recv: Mutex::new(msg_recv),
            msg_send,
            our_id,
        })
    }

    pub async fn remove_peer(&self, id: NodeID) -> Result<(), Error> {
        self.remove_peer_with_guard(&mut self.peers.write().await, id).await
    }

    async fn remove_peer_with_guard(&self, guard: &mut RwLockWriteGuard<'_, HashMap<NodeID, (Sender<Close>, Receiver<Close>, Arc<Peer<S>>)>>, id: NodeID) -> Result<(), Error> {
        match guard.remove(&id) {
            Some((close, mut peer_closed, _)) => match close.send(Close).await {
                Ok(()) => {
                    peer_closed.recv().await;
                    Ok(())
                },
                Err(err) => Err(Error::Other(format!(
                    "Failed to close existing peer, which was established by a lower id: {}",
                    err
                )))?,
            },
            None => Err(Error::Other(format!(
                "Peer {} not found, maybe it crashed or was removed",
                id
            )))?,
        }
    }

    /// ## Adds a [Peer] to the network
    /// - Will spawn a task to handle the peer
    /// - This can error out depending on different cases described below:
    ///
    /// ### Arguments
    /// - client: The peer struct which includes:
    ///     - stream: the bi-directional async read/write stream
    ///     - peer_id: the peer we are connecting to
    ///     - established_by (u64): who established the connection (us or them)
    ///
    /// ### Algorithm (OUT OF DATE)
    /// - If the established_by does not match our_id or peer_id, we return an error
    /// - lock the peers map
    /// - if the peer_id is already in the map
    ///     - if the established_by is the same as the one in the map return an error
    ///     - else get the peer with the lower established_by call the close handler
    ///     - add the peer with the higher established_by to the map & spawn a task to handle it
    /// - else add the peer to the map & spawn a task to handle it
    ///
    /// Closing a peer, or a peer erroring out, will cause the peer to be removed from the hashmap
    pub async fn add_peer(
        self: &Arc<Self>,
        new_peer: Arc<Peer<S>>,
    ) -> Result<JoinHandle<Result<(), Error>>, Error> {
        if new_peer.established_by != self.our_id && new_peer.established_by != new_peer.peer_id {
            Err(Error::Other(format!(
                "Peer tried to connect to us with established_by: {} who is neither neither us: {} or them: {}",
                new_peer.established_by, self.our_id, new_peer.peer_id
            )))?
        }

        // hold this lock until the very end so we don't have conflicts
        // with adding/removing/closing peers
        let mut peers = self.peers.write().await;

        match peers.get_mut(&new_peer.peer_id) {
            // there is already a peer connected with this the same id
            // if it is the same established_by, then we have a duplicate connection
            // so we return an error
            Some((_, _, existing_peer)) => if new_peer.established_by == existing_peer.established_by {
                Err(Error::Other(format!(
                "Peer tried to connect to us with established_by({}) which is the same as the one in the map ({})",
                new_peer.established_by, existing_peer.established_by
            )))?
            } else if new_peer.established_by > existing_peer.established_by {
                self.remove_peer_with_guard(&mut peers, new_peer.peer_id).await?;
            } else {
                // the new peer is lower, so we will do nothing as there
                // is already a peer with the same id handling the connection
                // return a task which just returns Ok as if the peer was closed
                return Ok(tokio::task::spawn(async { Ok(()) }));
            }
            // there is no peer with this id
            None => {}
        };

        let (close, mut close_recv) = tokio::sync::mpsc::channel(1);
        let (peer_closed_send, peer_closed_recv) = tokio::sync::mpsc::channel(1);

        let rpc = self.clone();
        peers.insert(new_peer.peer_id, (close, peer_closed_recv, new_peer.clone()));

        Ok(tokio::task::spawn(async move {
            let res = loop {
                select! {
                    result = new_peer.read_msg() => match result {
                        Result::Ok(msg) => match rpc.msg_send.send(msg) {
                            Ok(..) => {},
                            Err(err) => break Err(Error::Other(format!("Deleting peer, failed to send message: {:?}", err)))
                        },
                        Result::Err(e) => break Err(Error::Other(format!("Deleting peer, couldn't read message from peer: {:?}", e))),
                    },
                    Some(_) = close_recv.recv() => break Ok(())
                }
            };

            // let the close handler know the peer is closed
            // this is needed because it may take some time before the task
            // receives the close message
            match peer_closed_send.send(Close).await {
                Ok(()) => res,
                Err(err) => Err(Error::Other(format!(
                    "Failed to send back peer closed message: {:?}, but had result: {:?}",
                    err, res
                )))
            }
        }))
    }

    /// Clears all peers from the network
    pub async fn clear_outgoing(&self) {
        self.peers.write().await.clear();
    }
}
