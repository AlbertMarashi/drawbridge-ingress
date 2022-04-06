use std::{collections::HashMap, sync::Arc};
use tokio::{
    select,
    sync::{mpsc, oneshot, Mutex, RwLock},
    task::JoinHandle,
    time::Instant,
};

use tokio::time::Duration;

use crate::{
    states::{candidate::Candidate, follower::Follower, leader::Leader},
    types::{Message, MessageType, Peer, Stream, UserMsg},
    utils::get_random_timeout,
    Error, NodeID, Role, Senator, RPC,
};

impl<Msg: UserMsg, R: RPC<Msg>> Senator<Msg, R> {
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let senator = self.clone();

        tokio::spawn(async move {
            let mut last_role = { (*senator.role.read().await).clone() };
            loop {
                // each loop is a new term
                let new_role = { (*senator.role.read().await).clone() };

                // notify all on_role listeners that the role has changed
                if last_role != new_role {
                    for listener in senator.on_role.read().await.iter() {
                        listener(new_role.clone());
                    }
                }

                last_role = new_role;

                // run the state for this loop
                match new_role {
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

    pub async fn on_message<F: Fn(Message<Msg>) + Send + Sync + 'static>(self: &Arc<Self>, cb: F) {
        let mut on_message = self.on_message.write().await;
        on_message.push(Box::new(cb))
    }

    pub async fn handle_user_message(self: &Arc<Self>, msg: Message<Msg>) {
        // lock the message handlers
        let mut on_message = self.on_message.write().await;

        on_message.iter_mut().for_each(|f| f(msg.clone()));
    }

    pub async fn handle_vote_request(self: &Arc<Self>, to: NodeID, their_term: u64) {
        // if the candidates term is less than or equal to ours,
        // then we will not vote for them
        let mut our_term = self.term.write().await;
        if their_term <= *our_term {
            return;
        }
        // else we will become a follower, and vote for them
        let mut voted_for = self.voted_for.lock().await;

        *our_term = their_term;
        *self.role.write().await = Role::Follower;
        *voted_for = Some(to);

        self.rpc
            .send_msg(Message {
                from_role: *self.role.read().await,
                term: *our_term,
                to,
                from: self.id,
                msg: MessageType::VoteGranted
            })
            .await;
    }

    pub async fn broadcast_message(self: &Arc<Self>, msg: MessageType<Msg>) {
        let term = self.term.read().await.clone();
        let clients = self.rpc.members().await;

        for peer_id in clients.into_iter() {
            let msg = msg.clone();
            let senator = self.clone();
            tokio::spawn(async move {
                senator
                    .rpc
                    .send_msg(Message {
                        from_role: *senator.role.read().await,
                        from: senator.id,
                        to: peer_id,
                        term,
                        msg,
                    })
                    .await
            });
        }
    }

    pub fn new(minimum_delay: Duration, rpc: Arc<R>) -> Arc<Self> {
        Arc::new(Senator {
            id: rpc.our_id(),
            rpc,
            role: RwLock::new(Role::Follower),
            term: RwLock::new(0),
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
    pub peers: RwLock<HashMap<NodeID, (oneshot::Sender<oneshot::Sender<()>>, Arc<Peer<S>>)>>,
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
            Some((.., peer)) => match peer.send_msg::<Msg>(msg).await {
                Ok(()) => return,
                Err(Error::IO(e)) => println!("Could not send message: Peer probably closed connection {:?}", e),
                Err(err) => println!("Unexpected error {:?}", err)
            },
            None => println!("Peer {} not found, maybe it failed or was removed", msg.to)
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
        self.remove_peer_with_peers(&mut *self.peers.write().await, id)
            .await
    }

    async fn remove_peer_with_peers(
        &self,
        peers: &mut HashMap<NodeID, (oneshot::Sender<oneshot::Sender<()>>, Arc<Peer<S>>)>,
        id: NodeID,
    ) -> Result<(), Error> {
        let (closed_tx, closed_rx) = oneshot::channel();
        match peers.remove(&id) {
            Some((close, ..)) => {
                if let Err(_) = close.send(closed_tx) {
                    println!("Could not send close signal for peer {}", id);
                }
                if let Err(_) = closed_rx.await {
                    println!("Could not receive close signal for peer {}", id);
                }
            }
            None => println!("Peer {} not found, maybe it failed or was removed", id)
        }
        Ok(())
    }

    /// ## Adds a [Peer] to the network
    /// - Will spawn a task to handle the peer
    pub async fn add_peer(self: &Arc<Self>, new_peer: Arc<Peer<S>>) -> Result<(), Error> {
        // if this peer was established by neither us or them, then it is invalid
        if new_peer.established_by != self.our_id && new_peer.established_by != new_peer.peer_id {
            Err(Error::Other(format!(
                "Peer connection {} was neither established by us or them",
                new_peer.peer_id
            )))?
        }

        // hold this lock until the very end so we don't have conflicts
        // with adding or removing peers
        let mut peers = self.peers.write().await;

        if let Some((.., existing_peer)) = peers.get_mut(&new_peer.peer_id) {
            if new_peer.established_by == existing_peer.established_by {
                // if there is a duplicate existing peer with the same established_by
                // then we return an error, because this is a duplicate connection
                Err(Error::Other(format!(
                    "Peer connection {} was already established by us",
                    new_peer.peer_id
                )))?
            } else if new_peer.established_by > existing_peer.established_by {
                // if the new peer has a higher established by, we close the existing peer
                self.remove_peer_with_peers(&mut peers, new_peer.peer_id)
                    .await?
            } else {
                // the new peer is lower, so we will not add as there
                // is already a peer with the same id handling the connection
                Ok(())?
            }
        };

        let (close_sender, close_receiver) = tokio::sync::oneshot::channel();

        peers.insert(new_peer.peer_id, (close_sender, new_peer.clone()));

        let rpc = self.clone();

        tokio::task::spawn(async move {
            let res = select! {
                res = async {
                    loop {
                        match new_peer.read_msg().await {
                            Result::Ok(msg) => match rpc.msg_send.send(msg) {
                                Ok(..) => {},
                                Err(err) => break Err(Error::Other(format!("Deleting peer, failed to send message: {:?}", err)))
                            },
                            Result::Err(e) => break Err(Error::Other(format!("Deleting peer, couldn't read message from peer: {:?}", e))),
                        }
                    }
                } => res,
                recv = close_receiver => match recv {
                    Ok(closed_sender) => Ok(closed_sender),
                    Err(err) => Err(Error::Other(format!("Deleting peer, failed to receive close signal: {:?}", err)))
                }
            };

            match res {
                // we were closed
                Ok(closed_sender) => match closed_sender.send(()) {
                    Ok(..) => {},
                    Err(err) => eprintln!("Failed to send close signal to peer: {:?}", err),
                },
                // an error occured, remove the peer
                Err(err) => eprintln!("Error in peer task: {:?}", err)
            }
        });

        Ok(())
    }
}

// if a duplicate peer is added, use the one with the higher established_by
// we do this by keeping a close handle for each peer
// when we want to switch to a higher established_by peer, we close the lower one
// we want do do all of this while holding the write lock on the peers map
//
// we don't return join handles, as they get messy.
// we can return errors on add_peer, but there should be no errors
// beyond that point
//
// if we want to close a peer, then the peer should close it's stream
