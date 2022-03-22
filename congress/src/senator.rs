use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
    time::{Instant},
};

use crate::{
    types::{Peer, Stream, UserReq, UserRes, Message, MessageType, UserMessage},
    utils::get_random_timeout,
    states::{candidate::Candidate, follower::Follower, leader::Leader},
    NodeID, Response, Role, Senator, RPC, Request,
};

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Senator<Req, Res, R> {
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let senator = self.clone();

        tokio::spawn(async move {
            loop {
                // each loop is a new term
                let senator = senator.clone();
                let role = (*senator.role.lock().await).clone();

                senator.on_role.read().await.iter().for_each(|f| f(role.clone()));

                match role {
                    Role::Leader => Leader::new(senator).run().await,
                    Role::Follower => Follower::new(senator).run().await,
                    Role::Candidate => Candidate::new(senator).run().await,
                }
            }
        })
    }

    pub async fn on_role<F: Fn(Role) + Send + Sync + 'static>(self: &Arc<Self>, cb: F) {
        let mut on_role = self.on_role.write().await;
        on_role.push(Box::new(cb))
    }

    pub async fn on_message<F: Fn(UserMessage<Req, Res>) + Send + Sync + 'static>(self: &Arc<Self>, cb: F) {
        let mut on_message = self.on_message.write().await;
        on_message.push(Box::new(cb))
    }

    pub async fn handle_user_request(self: &Arc<Self>, req: Req) {
        // lock the message handlers
        let mut on_message = self.on_message.write().await;

        on_message.iter_mut().for_each(|f| f(UserMessage::Request(req.clone())));
    }

    pub async fn handle_vote_request(self: &Arc<Self>, to: NodeID, their_term: u64) {
        self.rpc.send_msg({
            // if the candidates term is less than ours, we will reject the vote
            let mut our_term = self.term.lock().await;
            if their_term < *our_term {
                Message {
                    term: *our_term,
                    from: self.id,
                    to,
                    msg: MessageType::Response(Response::Vote {
                        vote_granted: false,
                    }),
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
                        msg: MessageType::Response(Response::Vote {
                            vote_granted: false,
                        }),
                    },
                    // if we have not voted for anyone, we will accept
                    None => {
                        *voted_for = Some(to);
                        Message {
                            term: *our_term,
                            to,
                            from: self.id,
                            msg: MessageType::Response(Response::Vote {
                                vote_granted: true,
                            }),
                        }
                    }
                }
            }
        }).await;
    }

    pub async fn broadcast_request(
        self: &Arc<Self>,
        req: Request<Req>,
    ) {
        let term = self.term.lock().await.clone();
        let clients = self.rpc.members().await;

        for peer_id in clients.into_iter() {
            let req = req.clone();
            let senator = self.clone();
            tokio::spawn(async move {
                senator.rpc.send_msg(Message {
                    from: senator.id,
                    to: peer_id,
                    term,
                    msg: MessageType::Request(req.clone()),
                }).await
            });
        }
    }

    pub fn new(id: NodeID, rpc: Arc<R>) -> Arc<Self> {
        Arc::new(Senator {
            id,
            rpc,
            role: Mutex::new(Role::Follower),
            term: Mutex::new(0),
            last_state: Mutex::new(None),
            voted_for: Mutex::new(None),
            next_timeout: Mutex::new(Instant::now() + get_random_timeout()),
            current_leader: Mutex::new(None),
            phantom_res: std::marker::PhantomData,
            on_message: RwLock::new(Vec::new()),
            on_role: RwLock::new(Vec::new()),
        })
    }
}

pub struct RPCNetwork<Req: UserReq, Res: UserRes, S: Stream> {
    pub peers: RwLock<HashMap<u64, Arc<Peer<S>>>>,
    pub phantom_req: std::marker::PhantomData<Req>,
    pub msg_recv: Mutex<mpsc::UnboundedReceiver<Message<Req, Res>>>,
    pub msg_send: mpsc::UnboundedSender<Message<Req, Res>>,
}

#[async_trait]
impl<Req: UserReq, Res: UserRes, S: Stream> RPC<Req, Res> for RPCNetwork<Req, Res, S> {
    async fn recv_msg(&self) -> Message<Req, Res> {
        // this will never fail if msg_send is not dropped
        self.msg_recv.lock().await.recv().await.unwrap()
    }

    async fn send_msg(&self, msg: Message<Req, Res>){
        let peer = self
            .peers
            .read()
            .await
            .get(&msg.to)
            .cloned();

        match peer {
            Some(peer) => match peer.send_msg::<Req, Res>(msg).await {
                Ok(()) => {},
                Err(err) => eprintln!("Failed to send message to peer: {:?}", err)
            },
            None => eprintln!("Peer {} not found", msg.to),
        };
    }

    async fn members(&self) -> Vec<NodeID> {
        self.peers.read().await.keys().map(|key| *key).collect()
    }
}

impl<Req: UserReq, Res: UserRes, S: Stream> RPCNetwork<Req, Res, S> {
    pub fn new() -> Arc<Self> {
        let (msg_send, msg_recv) = mpsc::unbounded_channel();
        Arc::new(RPCNetwork {
            peers: RwLock::new(HashMap::new()),
            phantom_req: std::marker::PhantomData,
            msg_recv: Mutex::new(msg_recv),
            msg_send,
        })
    }

    pub async fn add_peer(self: &Arc<Self>, client: Arc<Peer<S>>) -> JoinHandle<()> {
        self.peers.write().await.insert(client.id, client.clone());

        let client = client.clone();
        let rpc = self.clone();

        tokio::task::spawn(async move {
            loop {
                match client.read_msg().await {
                    Result::Ok(msg) => match rpc.msg_send.send(msg) {
                        Ok(..) => {}
                        Err(err) => {
                            eprintln!("Deleting peer, failed to send message: {:?}", err);
                            break
                        }
                    },
                    Result::Err(e) => {
                        eprintln!("Deleting peer, couldn't read message from peer: {:?}", e);
                        break
                    },
                }
            }

            rpc.peers.write().await.remove(&client.id);
        })
    }

    pub async fn clear_outgoing(&self) {
        self.peers.write().await.clear();
    }
}
